use core::{pin::Pin, ptr};

use rustix::{
    fd::{AsRawFd, RawFd},
    io::Errno,
};

use crate::{
    managed::{
        SelectBuf, SelectGroup, UringProxy,
        buffer::{InFlight, IoBuf, IoBufMut},
        cancellation::Cancellation,
        completion::CompletionResult,
        submission::{Op, SingleOp},
    },
    sys::Sqe,
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

pub struct Read<B: IoBufMut, O: Offset = Current> {
    fd: RawFd,
    buf: InFlight<B>,
    offset: O,
}

impl<B: IoBufMut> Read<B> {
    pub fn new(fd: &impl AsRawFd, buf: B) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<B: IoBufMut> Read<B, At> {
    pub fn new_at(fd: &impl AsRawFd, buf: B, offset: usize) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

impl<B: IoBufMut, O: Offset> Op for Read<B, O> {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        // SAFETY: nothing is moved out of the pinned op
        let this = unsafe { self.get_unchecked_mut() };
        let (ptr, len) = this.buf.raw_buf_mut();
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        sqe.prep_read(this.fd, ptr, len, this.offset.position());
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.buf.into(), true)
    }
}

impl<B: IoBufMut, O: Offset> SingleOp for Read<B, O> {
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

pub struct Write<B: IoBuf, O: Offset = Current> {
    fd: RawFd,
    buf: InFlight<B>,
    offset: O,
}

impl<B: IoBuf> Write<B> {
    pub fn new(fd: &impl AsRawFd, buf: B) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            buf: InFlight::new(buf),
            offset: Current,
        }
    }
}

impl<B: IoBuf> Write<B, At> {
    pub fn new_at(fd: &impl AsRawFd, buf: B, offset: usize) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            buf: InFlight::new(buf),
            offset: At(offset),
        }
    }
}

impl<B: IoBuf, O: Offset> Op for Write<B, O> {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        // SAFETY: nothing is moved out of the pinned op
        let this = unsafe { self.get_unchecked_mut() };
        let (ptr, len) = this.buf.raw_buf();
        let len = u32::try_from(len).unwrap_or(u32::MAX);
        sqe.prep_write(this.fd, ptr, len, this.offset.position());
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.buf.into(), true)
    }
}

impl<B: IoBuf, O: Offset> SingleOp for Write<B, O> {
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
pub struct ReadSelect<U: UringProxy, O: Offset = Current> {
    fd: RawFd,
    group: SelectGroup<U>,
    offset: O,
}

impl<U: UringProxy> ReadSelect<U> {
    pub fn new(fd: &impl AsRawFd, group: &SelectGroup<U>) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            group: group.clone(),
            offset: Current,
        }
    }
}

impl<U: UringProxy> ReadSelect<U, At> {
    pub fn new_at(fd: &impl AsRawFd, group: &SelectGroup<U>, offset: usize) -> Self {
        Self {
            fd: fd.as_raw_fd(),
            group: group.clone(),
            offset: At(offset),
        }
    }
}

impl<U: UringProxy, O: Offset> Op for ReadSelect<U, O> {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        let size = self.group.buffer_size();
        let offset = self.offset.position();
        sqe.prep_read(self.fd, ptr::null_mut(), size, offset);
        sqe.buffer_select(self.group.id().as_raw());
    }
}

impl<U: UringProxy, O: Offset> SingleOp for ReadSelect<U, O> {
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
