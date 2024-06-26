use std::{
    io::{Error, Result},
    os::fd::RawFd,
    task::{Context, Poll},
};

use io_uring::{
    opcode,
    squeue::Entry,
    types::{self, Timespec},
};
use oneshot::Receiver;
use tracing::warn;

use crate::reactor::{self, CancelHandle};

#[derive(Default)]
pub struct RingHandle {
    inner: Option<(Receiver<i32>, CancelHandle)>,
}

fn from_ret<T>(ret: i32) -> Result<T>
where
    T: TryFrom<i32>,
{
    if ret >= 0 {
        let Ok(t) = ret.try_into() else {
            panic!(
                "Could not perform conversion from {} to {}",
                ret,
                std::any::type_name::<T>()
            );
        };

        Ok(t)
    } else {
        Err(Error::from_raw_os_error(ret))
    }
}

impl RingHandle {
    pub fn submit<E>(&mut self, cx: &mut Context, entry: E) -> Poll<i32>
    where
        E: FnOnce() -> Entry,
    {
        if self.inner.is_none() {
            let (sender, receiver) = oneshot::channel();
            let handle = reactor::submit(entry(), sender, cx);
            self.inner.replace((receiver, handle));

            return Poll::Pending;
        }

        match self.inner.as_ref().unwrap().0.try_recv() {
            Ok(ret) => {
                self.inner.take();
                Poll::Ready(ret)
            }
            Err(_) => Poll::Pending,
        }
    }

    pub fn cancel(&mut self) {
        if let Some(inner) = self.inner.take() {
            if inner.0.try_recv().is_ok() {
                warn!("Tried to cancel completed event");
            } else {
                inner.1.cancel();
            }
        }
    }

    // buf must live until the RingHandle is dropped
    pub unsafe fn read(
        &mut self,
        cx: &mut Context,
        fd: RawFd,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let read = || opcode::Read::new(types::Fd(fd), buf.as_mut_ptr(), buf.len() as _).build();

        self.submit(cx, read).map(from_ret)
    }

    // buf must live until the RingHandle is dropped
    pub unsafe fn write(&mut self, cx: &mut Context, fd: RawFd, buf: &[u8]) -> Poll<Result<usize>> {
        let read = || opcode::Write::new(types::Fd(fd), buf.as_ptr(), buf.len() as _).build();

        self.submit(cx, read).map(from_ret)
    }

    // time must live until the RingHandle is dropped
    pub unsafe fn timeout(&mut self, cx: &mut Context, time: *const Timespec) -> Poll<()> {
        let timeout = || opcode::Timeout::new(time).build();

        self.submit(cx, timeout).map(|ret| {
            assert_eq!(ret, -libc::ETIME);
        })
    }

    pub fn socket(&mut self, cx: &mut Context, domain: i32, stype: i32) -> Poll<Result<RawFd>> {
        let socket = || opcode::Socket::new(domain, stype, 0).build();

        self.submit(cx, socket).map(from_ret)
    }

    // sockaddr and socklen must live until the RingHandle is dropped
    pub unsafe fn accept(
        &mut self,
        cx: &mut Context,
        fd: i32,
        sockaddr: *mut libc::sockaddr,
        socklen: *mut libc::socklen_t,
    ) -> Poll<Result<i32>> {
        let socket = || opcode::Accept::new(types::Fd(fd), sockaddr, socklen).build();

        self.submit(cx, socket).map(from_ret)
    }

    // sockaddr and socklen must live until the RingHandle is dropped
    pub unsafe fn connect(
        &mut self,
        cx: &mut Context,
        fd: i32,
        sockaddr: *const libc::sockaddr,
        socklen: libc::socklen_t,
    ) -> Poll<Result<i32>> {
        let socket = || opcode::Connect::new(types::Fd(fd), sockaddr, socklen).build();

        self.submit(cx, socket).map(from_ret)
    }
}

impl Drop for RingHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}
