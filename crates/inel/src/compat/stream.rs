use std::{
    io::Result,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    group::{BufferShareGroup, ShareBuffered},
    io::{BufReader, BufWriter, FixedBufReader, FixedBufWriter, ReadSource, WriteSource},
};
use futures::{AsyncBufRead, AsyncRead, AsyncWrite};

macro_rules! impl_stream {
    ($stream:ident) => {
        impl<S> AsyncRead for $stream<S>
        where
            S: Unpin + ReadSource + WriteSource,
        {
            fn poll_read(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &mut [u8],
            ) -> Poll<Result<usize>> {
                self.pinned_inner().poll_read(cx, buf)
            }
        }

        impl<S> AsyncBufRead for $stream<S>
        where
            S: Unpin + ReadSource + WriteSource,
        {
            fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
                self.pinned_inner().poll_fill_buf(cx)
            }

            fn consume(self: Pin<&mut Self>, amt: usize) {
                self.pinned_inner().consume(amt)
            }
        }

        impl<S> AsyncWrite for $stream<S>
        where
            S: Unpin + ReadSource + WriteSource,
        {
            fn poll_write(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<Result<usize>> {
                self.pinned_inner().poll_write(cx, buf)
            }

            fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
                self.pinned_inner().poll_flush(cx)
            }

            fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
                self.pinned_inner().poll_close(cx)
            }
        }
    };
}

pub struct BufStream<S>(BufWriter<BufReader<S>>);

impl<S> BufStream<S> {
    pub fn new(stream: S) -> Self {
        Self(BufWriter::new(BufReader::new(stream)))
    }
}

impl<S> BufStream<S>
where
    S: Unpin + ReadSource + WriteSource,
{
    fn pinned_inner(self: Pin<&mut Self>) -> Pin<&mut BufWriter<BufReader<S>>> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl_stream!(BufStream);

pub struct FixedBufStream<S>(FixedBufWriter<FixedBufReader<S>>);

impl<S> FixedBufStream<S> {
    pub fn new(stream: S) -> Result<Self> {
        Ok(Self(BufWriter::new(BufReader::new(stream).fix()?).fix()?))
    }
}

impl<S> FixedBufStream<S>
where
    S: Unpin + ReadSource + WriteSource,
{
    fn pinned_inner(self: Pin<&mut Self>) -> Pin<&mut FixedBufWriter<FixedBufReader<S>>> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl_stream!(FixedBufStream);

pub struct ShareBufStream<S>(ShareBuffered<S>);

impl<S> ShareBufStream<S> {
    pub fn new(stream: S, group: &BufferShareGroup) -> Self {
        Self(group.supply_to(stream))
    }
}

impl<S> ShareBufStream<S>
where
    S: Unpin + ReadSource + WriteSource,
{
    fn pinned_inner(self: Pin<&mut Self>) -> Pin<&mut ShareBuffered<S>> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl_stream!(ShareBufStream);
