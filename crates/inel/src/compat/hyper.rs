use std::{
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncRead, AsyncWrite};

use crate::{
    compat::stream::{BufStream, FixedBufStream, ShareBufStream},
    group::BufferShareGroup,
};

pub struct HyperStream<Stream>(Stream);

impl<S> HyperStream<S> {
    pub fn new(stream: S) -> Self {
        Self(stream)
    }
}

impl<S> HyperStream<BufStream<S>> {
    pub fn new_buffered(stream: S) -> Self {
        Self(BufStream::new(stream))
    }
}

impl<S> HyperStream<FixedBufStream<S>> {
    pub fn new_buffered_fixed(stream: S) -> Result<Self> {
        FixedBufStream::new(stream).map(Self)
    }
}

impl<S> HyperStream<ShareBufStream<S>> {
    pub fn with_shared_buffers(stream: S, group: &BufferShareGroup) -> Self {
        Self(ShareBufStream::new(stream, group))
    }
}

impl<S: Unpin> HyperStream<S> {
    fn project(self: Pin<&mut Self>) -> Pin<&mut S> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl<S> hyper::rt::Read for HyperStream<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut cursor: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<()>> {
        let buf: &mut [u8] = unsafe { std::mem::transmute(cursor.as_mut()) };
        let read = ready!(self.project().poll_read(cx, buf))?;
        unsafe {
            cursor.advance(read);
        }
        Poll::Ready(Ok(()))
    }
}

impl<S> hyper::rt::Write for HyperStream<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.project().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().poll_close(cx)
    }
}
