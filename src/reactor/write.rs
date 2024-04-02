use std::{
    io::Result,
    os::fd::{AsRawFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::Future;

use crate::reactor::handle::RingHandle;

pub trait AsyncRingWrite {
    fn ring_write(&mut self, buf: Vec<u8>) -> RingWrite;
}

impl<T> AsyncRingWrite for T
where
    T: AsRawFd,
{
    fn ring_write(&mut self, buf: Vec<u8>) -> RingWrite {
        RingWrite {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            ring: RingHandle::default(),
        }
    }
}

pub struct RingWrite {
    fd: RawFd,
    buf: Option<Vec<u8>>,
    ring: RingHandle,
}

impl Future for RingWrite {
    type Output = Result<(Vec<u8>, usize)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let len = ready!(this.ring.write(cx, this.fd, this.buf.as_mut().unwrap()))?;
        Poll::Ready(Ok((this.buf.take().unwrap(), len)))
    }
}
