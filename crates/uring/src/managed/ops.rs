use core::ptr;
use std::ffi::{CStr, CString};

use rustix::{
    fd::AsRawFd,
    fs::{CWD, Mode, OFlags},
    io::Errno,
};

use crate::{
    managed::{
        DirectFd, IoSource, SelectBuf, SelectGroup, UringProxy,
        buffer::{InFlight, IoBuf, IoBufMut},
        cancellation::Cancellation,
        completion::CompletionResult,
        submission::{Op, SingleOp},
    },
    sys::{FILE_INDEX_ALLOC, Sqe},
};

mod private {
    pub trait Sealed {}
}

/// Where a read or write happens in the file.
pub trait Offset: private::Sealed {
    fn position(&self) -> u64;
}

/// At the file's current position, advancing it (offset `-1`). This is also
/// what non-seekable fds (pipes, sockets) use.
pub struct Current;

impl private::Sealed for Current {}

impl Offset for Current {
    fn position(&self) -> u64 {
        u64::MAX
    }
}

/// At a fixed position, leaving the file's current position untouched.
pub struct At(pub usize);

impl private::Sealed for At {}

impl Offset for At {
    fn position(&self) -> u64 {
        self.0 as u64
    }
}

const fn expect_positive(res: &CompletionResult) -> Result<usize, Errno> {
    let ret = res.result();
    if ret < 0 {
        Err(Errno::from_raw_os_error(-ret))
    } else {
        Ok(ret as usize)
    }
}

pub struct Read<'a, S: IoSource, B: IoBufMut, O: Offset = Current> {
    src: &'a S,
    buf: InFlight<B>,
    offset: O,
}

impl<'a, S: IoSource, B: IoBufMut> Read<'a, S, B> {
    pub fn new(src: &'a S, buf: B) -> Self {
        Self {
            src,
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<'a, S: IoSource, B: IoBufMut> Read<'a, S, B, At> {
    pub fn new_at(src: &'a S, buf: B, offset: usize) -> Self {
        Self {
            src,
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE only references the buffer, which lives behind `InFlight`
// and is handed over on cancel
unsafe impl<S: IoSource, B: IoBufMut, O: Offset> Op for Read<'_, S, B, O> {
    fn prep(&mut self, sqe: &mut Sqe) {
        let (ptr, len) = self.buf.raw_buf_mut();
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        sqe.prep_read(self.src.raw_fd(), ptr, len, self.offset.position());
        sqe.add_flag(self.src.flags());
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.buf.into_cancel(), true)
    }
}

// SAFETY: a plain read, one CQE
unsafe impl<S: IoSource, B: IoBufMut, O: Offset> SingleOp for Read<'_, S, B, O> {
    type Output = (B, Result<usize, Errno>);

    fn result(self, cqe: CompletionResult) -> Self::Output {
        let res = expect_positive(&cqe);
        // SAFETY: we got the CQE, so the kernel is done with the buffer
        let buf = match res {
            Ok(n) => unsafe { self.buf.into_inner_init(n) },
            Err(_) => unsafe { self.buf.into_inner() },
        };
        (buf, res)
    }
}

pub struct Write<'a, S: IoSource, B: IoBuf, O: Offset = Current> {
    src: &'a S,
    buf: InFlight<B>,
    offset: O,
}

impl<'a, S: IoSource, B: IoBuf> Write<'a, S, B> {
    pub fn new(src: &'a S, buf: B) -> Self {
        Self {
            src,
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<'a, S: IoSource, B: IoBuf> Write<'a, S, B, At> {
    pub fn new_at(src: &'a S, buf: B, offset: usize) -> Self {
        Self {
            src,
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE only references the buffer, which lives behind `InFlight`
// and is handed over on cancel
unsafe impl<S: IoSource, B: IoBuf, O: Offset> Op for Write<'_, S, B, O> {
    fn prep(&mut self, sqe: &mut Sqe) {
        let (ptr, len) = self.buf.raw_buf();
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        sqe.prep_write(self.src.raw_fd(), ptr, len, self.offset.position());
        sqe.add_flag(self.src.flags());
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.buf.into_cancel(), true)
    }
}

// SAFETY: a plain write, one CQE
unsafe impl<S: IoSource, B: IoBuf, O: Offset> SingleOp for Write<'_, S, B, O> {
    type Output = (B, Result<usize, Errno>);

    fn result(self, cqe: CompletionResult) -> Self::Output {
        // SAFETY: we got the CQE, so the kernel is done with the buffer
        let buf = unsafe { self.buf.into_inner() };
        (buf, expect_positive(&cqe))
    }
}

/// A read into a buffer the kernel picks from `group` once data is available,
/// so idle reads don't tie up memory.
///
/// Holds its own handle to the group, so it has no lifetime and the group
/// stays alive until the op is done.
///
/// Fails with `ENOBUFS` if the group has no free buffer at that point; on
/// pollable fds nothing was read then, so the read can simply be retried.
pub struct ReadSelect<'a, U: UringProxy, S: IoSource, O: Offset = Current> {
    src: &'a S,
    group: SelectGroup<U>,
    offset: O,
}

impl<'a, U: UringProxy, S: IoSource> ReadSelect<'a, U, S> {
    pub fn new(src: &'a S, group: &SelectGroup<U>) -> Self {
        Self {
            src,
            group: group.clone(),
            offset: Current,
        }
    }
}

impl<'a, U: UringProxy, S: IoSource> ReadSelect<'a, U, S, At> {
    pub fn new_at(src: &'a S, group: &SelectGroup<U>, offset: usize) -> Self {
        Self {
            src,
            group: group.clone(),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE references no memory, the kernel picks a buffer from the
// group, which the ring keeps alive while the op is in flight
unsafe impl<U: UringProxy, S: IoSource, O: Offset> Op for ReadSelect<'_, U, S, O> {
    fn prep(&mut self, sqe: &mut Sqe) {
        let size = self.group.buffer_size();
        let offset = self.offset.position();
        sqe.prep_read(self.src.raw_fd(), ptr::null_mut(), size, offset);
        sqe.add_flag(self.src.flags());
        sqe.buffer_select(self.group.id().as_raw());
    }
}

// SAFETY: a plain (not multishot) read, one CQE
unsafe impl<U: UringProxy, S: IoSource, O: Offset> SingleOp for ReadSelect<'_, U, S, O> {
    type Output = Result<SelectBuf<U>, Errno>;

    fn result(self, cqe: CompletionResult) -> Self::Output {
        match (expect_positive(&cqe), cqe.buffer_id()) {
            // SAFETY: we got the CQE, so the kernel is done with the buffer and
            // wrote exactly the number of bytes it returned
            (Ok(n), Some(bid)) => Ok(unsafe { self.group.take(bid, n) }),
            (Err(err), None) => Err(err),
            (Err(err), Some(bid)) => {
                // SAFETY: the bid comes from this CQE, and 0 bytes are always
                // initialized
                drop(unsafe { self.group.take(bid, 0) });
                Err(err)
            }
            (Ok(_), None) => unreachable!("buffer select read completed without a buffer"),
        }
    }
}

/// Opens a file (relative to the current directory) as a [`DirectFd`] in a
/// slot the kernel picks.
///
/// `O_CLOEXEC` is ignored: direct descriptors aren't inherited anyway. Fails
/// with `ENFILE` when the table is full.
///
/// Dropping the submission after the kernel opened the file leaks the slot
/// (until the ring is dropped): the result is discarded unread.
pub struct OpenDirect<U: UringProxy> {
    path: CString,
    flags: OFlags,
    mode: Mode,
    ring: U,
}

impl<U: UringProxy> OpenDirect<U> {
    /// `ring` must be the ring the op runs on: it owns the descriptor.
    pub fn new(path: &CStr, flags: OFlags, mode: Mode, ring: U) -> Self {
        Self {
            path: path.to_owned(),
            flags: flags.difference(OFlags::CLOEXEC),
            mode,
            ring,
        }
    }
}

// SAFETY: the SQE only references the path, which lives on the heap and is
// handed over on cancel
unsafe impl<U: UringProxy> Op for OpenDirect<U> {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_openat(
            CWD.as_raw_fd(),
            self.path.as_ptr(),
            self.flags,
            self.mode.bits(),
        );
        sqe.set_file_index(FILE_INDEX_ALLOC);
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.path.into(), true)
    }
}

// SAFETY: a plain open, one CQE
unsafe impl<U: UringProxy> SingleOp for OpenDirect<U> {
    type Output = Result<DirectFd<U>, Errno>;

    fn result(self, cqe: CompletionResult) -> Self::Output {
        let slot = expect_positive(&cqe)?;
        // SAFETY: the kernel allocated the slot for this op, and the ring
        // started tracking it when handing out the result
        Ok(unsafe { DirectFd::from_raw(slot as u32, self.ring) })
    }
}
