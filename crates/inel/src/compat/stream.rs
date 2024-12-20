use std::{
    io::Result,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    io::{BufReader, BufWriter, FixedBufReader, FixedBufWriter, ReadHandle, WriteHandle},
    net::TcpStream,
};
use futures::{AsyncBufRead, AsyncRead, AsyncWrite};

pub struct BufTcpStream {
    reader: BufReader<ReadHandle<TcpStream>>,
    writer: BufWriter<WriteHandle<TcpStream>>,
}

impl BufTcpStream {
    pub fn new(stream: TcpStream) -> Self {
        let (reader, writer) = stream.split_buffered();
        Self { reader, writer }
    }

    pub fn fix(self) -> Result<FixedBufTcpStream> {
        let BufTcpStream { reader, writer } = self;
        let (reader, writer) = (reader.fix()?, writer.fix()?);
        Ok(FixedBufTcpStream { reader, writer })
    }

    fn pinned_reader(self: Pin<&mut Self>) -> Pin<&mut BufReader<ReadHandle<TcpStream>>> {
        Pin::new(&mut Pin::into_inner(self).reader)
    }

    fn pinned_writer(self: Pin<&mut Self>) -> Pin<&mut BufWriter<WriteHandle<TcpStream>>> {
        Pin::new(&mut Pin::into_inner(self).writer)
    }
}

impl Deref for BufTcpStream {
    type Target = TcpStream;

    fn deref(&self) -> &Self::Target {
        self.reader.inner()
    }
}

impl AsyncRead for BufTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        self.pinned_reader().poll_read(cx, buf)
    }
}

impl AsyncBufRead for BufTcpStream {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        self.pinned_reader().poll_fill_buf(cx)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        self.pinned_reader().consume(amt)
    }
}

impl AsyncWrite for BufTcpStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.pinned_writer().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_writer().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_writer().poll_close(cx)
    }
}

pub struct FixedBufTcpStream {
    reader: FixedBufReader<ReadHandle<TcpStream>>,
    writer: FixedBufWriter<WriteHandle<TcpStream>>,
}

impl FixedBufTcpStream {
    fn pinned_reader(self: Pin<&mut Self>) -> Pin<&mut FixedBufReader<ReadHandle<TcpStream>>> {
        Pin::new(&mut Pin::into_inner(self).reader)
    }

    fn pinned_writer(self: Pin<&mut Self>) -> Pin<&mut FixedBufWriter<WriteHandle<TcpStream>>> {
        Pin::new(&mut Pin::into_inner(self).writer)
    }
}

impl Deref for FixedBufTcpStream {
    type Target = TcpStream;

    fn deref(&self) -> &Self::Target {
        self.reader.inner()
    }
}

impl AsyncRead for FixedBufTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        self.pinned_reader().poll_read(cx, buf)
    }
}

impl AsyncBufRead for FixedBufTcpStream {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        self.pinned_reader().poll_fill_buf(cx)
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        self.pinned_reader().consume(amt)
    }
}

impl AsyncWrite for FixedBufTcpStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.pinned_writer().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_writer().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_writer().poll_close(cx)
    }
}
