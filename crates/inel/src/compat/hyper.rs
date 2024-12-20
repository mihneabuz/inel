use std::{
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncWrite};

use crate::{compat::stream::BufTcpStream, net::TcpStream};

pub struct HyperStream {
    inner: BufTcpStream,
}

impl HyperStream {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            inner: BufTcpStream::new(stream),
        }
    }

    fn project(self: Pin<&mut Self>) -> Pin<&mut BufTcpStream> {
        Pin::new(&mut Pin::into_inner(self).inner)
    }
}

impl hyper::rt::Read for HyperStream {
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

impl hyper::rt::Write for HyperStream {
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
