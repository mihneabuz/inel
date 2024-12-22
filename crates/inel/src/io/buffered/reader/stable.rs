use std::{
    io::{self, Error, Result},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncRead, FutureExt};

use super::{fixed::FixedBufReader, generic::*};
use crate::{
    buffer::Fixed,
    io::{owned::ReadOwned, AsyncReadOwned},
};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type BoxBuf = Box<[u8]>;
type ReadFut = ReadOwned<BoxBuf>;

pub struct BufReader<S>(BufReaderGeneric<S, BoxBuf, ReadFut>);

impl<S> BufReader<S> {
    pub fn new(source: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, source)
    }

    pub fn with_capacity(capacity: usize, source: S) -> Self {
        Self(BufReaderGeneric {
            state: BufReaderState::Ready(RBuffer::empty(capacity)),
            source,
        })
    }

    pub fn fix(self) -> Result<FixedBufReader<S>> {
        let (source, buf) = self.0.into_raw_parts();
        let (buf, pos, filled) = buf.ok_or(Error::other("BufReader was in use"))?;

        let fixed = Fixed::register(buf)?;
        let buffer = RBuffer::new(fixed, pos, filled);

        Ok(FixedBufReader::from_raw(source, buffer))
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
    S: AsyncReadOwned + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let mut inner = ready!(self.as_mut().poll_fill_buf(cx))?;
        let len = io::Read::read(&mut inner, buf)?;
        self.consume(len);
        Poll::Ready(Ok(len))
    }
}

impl<S> AsyncBufRead for BufReader<S>
where
    S: AsyncReadOwned + Unpin,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufReaderState::Empty => unreachable!(),

            BufReaderState::Pending(fut) => {
                let (buf, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(filled) => {
                        this.0.state = BufReaderState::Ready(RBuffer::new(buf, 0, filled));
                    }

                    Err(err) => {
                        this.0.state = BufReaderState::Ready(RBuffer::new(buf, 0, 0));

                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufReaderState::Ready(buf) => {
                if buf.is_empty() {
                    let BufReaderState::Ready(buf) = std::mem::take(&mut this.0.state) else {
                        unreachable!();
                    };

                    let fut = this.0.source.read_owned(buf.into_raw_parts().0);

                    this.0.state = BufReaderState::Pending(fut);

                    return Pin::new(this).poll_fill_buf(cx);
                }
            }
        };

        Poll::Ready(Ok(this.0.buffer().unwrap()))
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        if let BufReaderState::Ready(buf) = &mut Pin::into_inner(self).0.state {
            buf.consume(amt);
        }
    }
}
