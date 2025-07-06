use std::{
    io::Result,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncBufRead, AsyncBufReadExt, AsyncRead};

use super::generic::*;
use crate::{
    buffer::Fixed,
    io::{owned::ReadFixed, AsyncReadOwned, ReadSource},
};

type FixedRead = ReadFixed<Fixed>;

struct FixedAdapter;
impl<S: ReadSource> BufReaderAdapter<S, Fixed, FixedRead> for FixedAdapter {
    fn create_future(&self, source: &mut S, buffer: Fixed) -> FixedRead {
        source.read_fixed(buffer)
    }
}

pub struct FixedBufReader<S>(BufReaderGeneric<S, Fixed, FixedRead, FixedAdapter>);

impl<S> FixedBufReader<S> {
    pub(crate) fn from_raw(
        source: S,
        buffer: Box<[u8]>,
        pos: usize,
        filled: usize,
    ) -> Result<Self> {
        Ok(Self(BufReaderGeneric::new(
            Fixed::register(buffer)?,
            source,
            pos,
            filled,
            FixedAdapter,
        )))
    }

    pub fn capacity(&self) -> Option<usize> {
        self.0.capacity()
    }

    pub fn buffer(&self) -> Option<&[u8]> {
        self.0.buffer()
    }

    pub fn inner(&self) -> &S {
        self.0.inner()
    }

    pub fn inner_mut(&mut self) -> &mut S {
        self.0.inner_mut()
    }

    pub fn into_inner(self) -> S {
        self.0.into_inner()
    }
}

impl<S> AsyncRead for FixedBufReader<S>
where
    S: ReadSource + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut Pin::into_inner(self).0).poll_read(cx, buf)
    }
}

impl<S> AsyncBufRead for FixedBufReader<S>
where
    S: ReadSource + Unpin,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        Pin::new(&mut Pin::into_inner(self).0).poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        self.0.consume_unpin(amt);
    }
}
