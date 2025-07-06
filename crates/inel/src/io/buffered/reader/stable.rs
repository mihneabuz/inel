use std::{
    io::{Error, Result},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncBufRead, AsyncBufReadExt, AsyncRead};

use super::{fixed::FixedBufReader, generic::*};
use crate::io::{owned::ReadOwned, AsyncReadOwned, ReadSource};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type BoxBuf = Box<[u8]>;

struct OwnedAdapter;
impl<S: ReadSource> BufReaderAdapter<S, BoxBuf, ReadOwned<BoxBuf>> for OwnedAdapter {
    fn create_future(&self, source: &mut S, buffer: BoxBuf) -> ReadOwned<BoxBuf> {
        source.read_owned(buffer)
    }
}

pub struct BufReader<S>(BufReaderGeneric<S, BoxBuf, ReadOwned<BoxBuf>, OwnedAdapter>);

impl<S> BufReader<S> {
    pub fn new(source: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, source)
    }

    pub fn with_capacity(capacity: usize, source: S) -> Self {
        Self(BufReaderGeneric::empty(
            vec![0; capacity].into_boxed_slice(),
            source,
            OwnedAdapter,
        ))
    }

    pub fn fix(self) -> Result<FixedBufReader<S>> {
        let (source, buf) = self.0.into_raw_parts();
        let (buf, pos, filled) = buf.ok_or(Error::other("BufReader was in use"))?;
        FixedBufReader::from_raw(source, buf, pos, filled)
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

impl<S> AsyncRead for BufReader<S>
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

impl<S> AsyncBufRead for BufReader<S>
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
