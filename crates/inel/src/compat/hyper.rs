use std::{
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncWrite};

use crate::{
    compat::stream::{BufTcpStream, FixedBufTcpStream, ShareTcpStream},
    group::BufferShareGroup,
    net::TcpStream,
};

pub struct HyperStream<S>(S);

impl HyperStream<BufTcpStream> {
    pub fn new(stream: TcpStream) -> Self {
        Self(BufTcpStream::new(stream))
    }
}

impl HyperStream<FixedBufTcpStream> {
    pub fn new_fixed(stream: TcpStream) -> Result<Self> {
        FixedBufTcpStream::new(stream).map(Self)
    }
}

impl HyperStream<ShareTcpStream> {
    pub fn with_shared_buffers(stream: TcpStream, group: &BufferShareGroup) -> Self {
        Self(ShareTcpStream::new(stream, group))
    }
}

impl<S: Unpin> HyperStream<S> {
    fn project(self: Pin<&mut Self>) -> Pin<&mut S> {
        Pin::new(&mut Pin::into_inner(self).0)
    }
}

impl<S> hyper::rt::Read for HyperStream<S>
where
    S: AsyncBufRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut cursor: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<()>> {
        let mut inner = self.project();
        let buf = ready!(inner.as_mut().poll_fill_buf(cx))?;
        let size = cursor.remaining().min(buf.len());
        cursor.put_slice(&buf[..size]);
        inner.consume(size);
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
