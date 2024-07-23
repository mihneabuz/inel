use std::{
    io::Result,
    os::fd::{AsRawFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncRead, Future};
use pin_project_lite::pin_project;

use crate::reactor::handle::RingHandle;

pub trait AsyncRingRead {
    fn ring_read<Buf>(&mut self, buf: Buf) -> RingRead<Buf>
    where
        Buf: AsMut<[u8]>;

    fn ring_read_at<Buf>(&mut self, buf: Buf, start: usize) -> RingRead<Buf>
    where
        Buf: AsMut<[u8]>;
}

impl<T> AsyncRingRead for T
where
    T: AsRawFd,
{
    fn ring_read<Buf>(&mut self, buf: Buf) -> RingRead<Buf> {
        RingRead {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            start: 0,
            ring: RingHandle::default(),
        }
    }

    fn ring_read_at<Buf>(&mut self, buf: Buf, start: usize) -> RingRead<Buf> {
        RingRead {
            fd: self.as_raw_fd(),
            buf: Some(buf),
            start,
            ring: RingHandle::default(),
        }
    }
}

pub struct RingRead<Buf> {
    fd: RawFd,
    buf: Option<Buf>,
    start: usize,
    ring: RingHandle,
}

impl<Buf> Future for RingRead<Buf>
where
    Buf: AsMut<[u8]> + Unpin,
{
    type Output = (Buf, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let buf = &mut this.buf.as_mut().unwrap().as_mut()[this.start..];
        let len = ready!(unsafe { this.ring.read(cx, this.fd, buf) });
        Poll::Ready((this.buf.take().unwrap(), len))
    }
}

pin_project! {
    pub struct RingBufReader<T> {
        #[pin]
        inner: T,
        ring: RingHandle,
        buf: Vec<u8>,
        pos: usize,
        cap: usize,
    }
}

impl<T> RingBufReader<T>
where
    T: AsRawFd,
{
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            ring: RingHandle::default(),
            buf: vec![0u8; 4096],
            pos: 0,
            cap: 0,
        }
    }

    pub fn with_buf_size(buf_size: usize, inner: T) -> Self {
        Self {
            inner,
            ring: RingHandle::default(),
            buf: vec![0u8; buf_size],
            pos: 0,
            cap: 0,
        }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> AsyncRead for RingBufReader<T>
where
    T: AsRawFd,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let this = self.project();
        let fd = this.inner.as_raw_fd();

        if *this.pos >= *this.cap {
            assert_eq!(*this.pos, *this.cap);
            *this.cap = ready!(unsafe { this.ring.read(cx, fd, this.buf) })?;
            *this.pos = 0;
        }

        let len = buf.len().min(*this.cap - *this.pos);
        let range = *this.pos..*this.pos + len;
        buf.copy_from_slice(&this.buf[range]);

        *this.pos += len;

        Poll::Ready(Ok(len))
    }
}

impl<T> AsyncBufRead for RingBufReader<T>
where
    T: AsRawFd,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        let this = self.project();
        let fd = this.inner.as_raw_fd();

        if *this.pos >= *this.cap {
            assert_eq!(*this.pos, *this.cap);
            *this.cap = ready!(unsafe { this.ring.read(cx, fd, this.buf) })?;
            *this.pos = 0;
        }

        Poll::Ready(Ok(&this.buf[*this.pos..*this.cap]))
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        *self.project().pos += amt;
    }
}
