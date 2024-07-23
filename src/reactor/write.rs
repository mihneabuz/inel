use std::{
    io::Result,
    os::fd::{AsRawFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::Future;

use crate::reactor::handle::RingHandle;

pub trait AsyncRingWrite {
    fn ring_write<Buf>(&mut self, buf: Buf) -> RingWrite<Buf>
    where
        Buf: AsRef<[u8]>;

    fn ring_write_at<Buf>(&mut self, buf: Buf, start: usize) -> RingWrite<Buf>
    where
        Buf: AsRef<[u8]>;
}

impl<T> AsyncRingWrite for T
where
    T: AsRawFd,
{
    fn ring_write<Buf>(&mut self, buf: Buf) -> RingWrite<Buf> {
        RingWrite {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            start: 0,
            ring: RingHandle::default(),
        }
    }

    fn ring_write_at<Buf>(&mut self, buf: Buf, start: usize) -> RingWrite<Buf> {
        RingWrite {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            start,
            ring: RingHandle::default(),
        }
    }
}

pub struct RingWrite<Buf> {
    fd: RawFd,
    buf: Option<Buf>,
    start: usize,
    ring: RingHandle,
}

impl<Buf> Future for RingWrite<Buf>
where
    Buf: AsRef<[u8]> + Unpin,
{
    type Output = (Buf, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let buf = this.buf.as_mut().unwrap().as_ref()[this.start..];
        let len = ready!(unsafe { this.ring.write(cx, this.fd, buf) });
        Poll::Ready((this.buf.take().unwrap(), len))
    }
}

pub trait AsyncRingWriteExt: AsyncRingWrite {
    fn ring_write_all(&mut self, buf: Vec<u8>) -> impl Future<Output = (Vec<u8>, Result<()>)>;
}

impl<T> AsyncRingWriteExt for T
where
    T: AsyncRingWrite,
{
    async fn ring_write_all(&mut self, mut buf: Vec<u8>) -> (Vec<u8>, Result<()>) {
        let mut pos = 0;
        while pos < buf.len() {
            let (ret_buf, res) = self.ring_write_at(buf, pos).await;
            buf = ret_buf;

            match res {
                Ok(wrote) => pos += wrote,
                Err(err) => return (buf, Err(err)),
            }
        }

        (buf, Ok(()))
    }
}
