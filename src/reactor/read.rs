use std::{
    io::Result,
    os::fd::{AsRawFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::Future;

use crate::reactor::handle::RingHandle;

pub trait AsyncRingRead {
    fn ring_read(&mut self, buf: Vec<u8>) -> RingRead;
}

impl<T> AsyncRingRead for T
where
    T: AsRawFd,
{
    fn ring_read(&mut self, buf: Vec<u8>) -> RingRead {
        RingRead {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            ring: RingHandle::default(),
        }
    }
}

pub struct RingRead {
    fd: RawFd,
    buf: Option<Vec<u8>>,
    ring: RingHandle,
}

impl Future for RingRead {
    type Output = Result<(Vec<u8>, usize)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let len = ready!(this.ring.read(cx, this.fd, this.buf.as_mut().unwrap()))?;
        Poll::Ready(Ok((this.buf.take().unwrap(), len)))
    }
}
