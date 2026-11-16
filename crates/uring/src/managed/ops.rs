use core::ptr;
use std::ffi::CStr;

use rustix::{
    fd::{AsRawFd, RawFd},
    fs::{CWD, Mode, OFlags},
    io::Errno,
};

use crate::{
    managed::{
        DirectFd, IoSrc, RawSrc, SelectBuf, SelectGroup, UringFd, UringProxy,
        buffer::{InFlight, IoBuf, IoBufMut},
        cancellation::Cancellation,
        completion::CompletionResult,
        submission::{Op, Scratch, SingleOp},
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

pub struct Read<'a, B: IoBufMut, O: Offset = Current> {
    src: RawSrc<'a>,
    buf: InFlight<B>,
    offset: O,
}

impl<'a, B: IoBufMut> Read<'a, B> {
    pub fn new(src: &'a impl IoSrc, buf: B) -> Self {
        Self {
            src: src.raw_src(),
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<'a, B: IoBufMut> Read<'a, B, At> {
    pub fn new_at(src: &'a impl IoSrc, buf: B, offset: usize) -> Self {
        Self {
            src: src.raw_src(),
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE only references the buffer, which lives behind `InFlight`
// and is handed over on cancel
unsafe impl<B: IoBufMut, O: Offset> Op for Read<'_, B, O> {
    fn prep(&mut self, sqe: &mut Sqe, _scratch: &Scratch) {
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
unsafe impl<U: UringProxy, B: IoBufMut, O: Offset> SingleOp<U> for Read<'_, B, O> {
    type Output = (B, Result<usize, Errno>);

    fn result(self, cqe: CompletionResult, _: &U) -> Self::Output {
        let res = expect_positive(&cqe);
        // SAFETY: we got the CQE, so the kernel is done with the buffer
        let buf = match res {
            Ok(n) => unsafe { self.buf.into_inner_init(n) },
            Err(_) => unsafe { self.buf.into_inner() },
        };
        (buf, res)
    }
}

pub struct Write<'a, B: IoBuf, O: Offset = Current> {
    src: RawSrc<'a>,
    buf: InFlight<B>,
    offset: O,
}

impl<'a, B: IoBuf> Write<'a, B> {
    pub fn new(src: &'a impl IoSrc, buf: B) -> Self {
        Self {
            src: src.raw_src(),
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<'a, B: IoBuf> Write<'a, B, At> {
    pub fn new_at(src: &'a impl IoSrc, buf: B, offset: usize) -> Self {
        Self {
            src: src.raw_src(),
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE only references the buffer, which lives behind `InFlight`
// and is handed over on cancel
unsafe impl<B: IoBuf, O: Offset> Op for Write<'_, B, O> {
    fn prep(&mut self, sqe: &mut Sqe, _scratch: &Scratch) {
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
unsafe impl<U: UringProxy, B: IoBuf, O: Offset> SingleOp<U> for Write<'_, B, O> {
    type Output = (B, Result<usize, Errno>);

    fn result(self, cqe: CompletionResult, _: &U) -> Self::Output {
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
pub struct ReadSelect<'a, U: UringProxy, O: Offset = Current> {
    src: RawSrc<'a>,
    group: SelectGroup<U>,
    offset: O,
}

impl<'a, U: UringProxy> ReadSelect<'a, U> {
    pub fn new(src: &'a impl IoSrc, group: &SelectGroup<U>) -> Self {
        Self {
            src: src.raw_src(),
            group: group.clone(),
            offset: Current,
        }
    }
}

impl<'a, U: UringProxy> ReadSelect<'a, U, At> {
    pub fn new_at(src: &'a impl IoSrc, group: &SelectGroup<U>, offset: usize) -> Self {
        Self {
            src: src.raw_src(),
            group: group.clone(),
            offset: At(offset),
        }
    }
}

// SAFETY: the SQE references no memory, the kernel picks a buffer from the
// group, which the ring keeps alive while the op is in flight
unsafe impl<U: UringProxy, O: Offset> Op for ReadSelect<'_, U, O> {
    fn prep(&mut self, sqe: &mut Sqe, _scratch: &Scratch) {
        let size = self.group.buffer_size();
        let offset = self.offset.position();
        sqe.prep_read(self.src.raw_fd(), ptr::null_mut(), size, offset);
        sqe.add_flag(self.src.flags());
        sqe.buffer_select(self.group.id().as_raw());
    }
}

// SAFETY: a plain (not multishot) read, one CQE
unsafe impl<U: UringProxy, O: Offset> SingleOp<U> for ReadSelect<'_, U, O> {
    type Output = Result<SelectBuf<U>, Errno>;

    fn result(self, cqe: CompletionResult, _: &U) -> Self::Output {
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

/// Opens a file (relative to the current directory) as a [`UringFd`].
///
/// `O_CLOEXEC` is always added, like `std` does.
///
/// Dropping the submission after the kernel opened the file leaks the fd: the
/// result is discarded unread.
pub struct Open<'a> {
    path: &'a CStr,
    flags: OFlags,
    mode: Mode,
}

impl<'a> Open<'a> {
    pub const fn new(path: &'a CStr, flags: OFlags, mode: Mode) -> Self {
        Self {
            path,
            flags: flags.union(OFlags::CLOEXEC),
            mode,
        }
    }
}

// SAFETY: the SQE only references the path's copy in scratch, which the
// kernel reads during submit, so the borrowed path only has to live for prep
unsafe impl Op for Open<'_> {
    fn prep(&mut self, sqe: &mut Sqe, scratch: &Scratch) {
        sqe.prep_openat(
            CWD.as_raw_fd(),
            scratch.alloc_cstr(self.path),
            self.flags,
            self.mode.bits(),
        );
    }
}

// SAFETY: a plain open, one CQE
unsafe impl<U: UringProxy> SingleOp<U> for Open<'_> {
    type Output = Result<UringFd<U>, Errno>;

    fn result(self, cqe: CompletionResult, ring: &U) -> Self::Output {
        let fd = expect_positive(&cqe)?;
        // SAFETY: the kernel just opened the fd for this op, nothing else owns it
        Ok(unsafe { UringFd::from_raw(fd as RawFd, ring.clone()) })
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
pub struct OpenDirect<'a> {
    path: &'a CStr,
    flags: OFlags,
    mode: Mode,
}

impl<'a> OpenDirect<'a> {
    pub const fn new(path: &'a CStr, flags: OFlags, mode: Mode) -> Self {
        Self {
            path,
            flags: flags.difference(OFlags::CLOEXEC),
            mode,
        }
    }
}

// SAFETY: the SQE only references the path's copy in scratch, which the
// kernel reads during submit, so the borrowed path only has to live for prep
unsafe impl Op for OpenDirect<'_> {
    fn prep(&mut self, sqe: &mut Sqe, scratch: &Scratch) {
        sqe.prep_openat(
            CWD.as_raw_fd(),
            scratch.alloc_cstr(self.path),
            self.flags,
            self.mode.bits(),
        );
        sqe.set_file_index(FILE_INDEX_ALLOC);
    }
}

// SAFETY: a plain open, one CQE
unsafe impl<U: UringProxy> SingleOp<U> for OpenDirect<'_> {
    type Output = Result<DirectFd<U>, Errno>;

    fn result(self, cqe: CompletionResult, ring: &U) -> Self::Output {
        let slot = expect_positive(&cqe)?;
        // SAFETY: the kernel allocated the slot for this op in `ring`'s table
        // (the one it ran on), and nothing else owns it
        Ok(unsafe { DirectFd::from_raw(slot as u32, ring.clone()) })
    }
}
