use std::{
    io::{self, Result},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncRead, FutureExt};

use super::generic::*;
use crate::{
    buffer::Fixed,
    io::{owned::ReadFixed, AsyncReadFixed},
};

type FixedBoxBuf = Fixed<Box<[u8]>>;
type FixedReadFut = ReadFixed<FixedBoxBuf>;

pub struct FixedBufReader<S>(BufReaderGeneric<S, FixedBoxBuf, FixedReadFut>);

impl<S> FixedBufReader<S> {
    pub(crate) fn from_raw(source: S, buffer: FixedBoxBuf) -> Self {
        Self(BufReaderGeneric {
            state: BufReaderState::Ready(Buffer::new(buffer, 0)),
            source,
        })
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
    S: AsyncReadFixed + Unpin,
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

impl<S> AsyncBufRead for FixedBufReader<S>
where
    S: AsyncReadFixed + Unpin,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufReaderState::Empty => unreachable!(),

            BufReaderState::Pending(fut) => {
                let (buf, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(filled) => {
                        this.0.state = BufReaderState::Ready(Buffer::new(buf, filled));
                    }

                    Err(err) => {
                        this.0.state = BufReaderState::Ready(Buffer::new(buf, 0));

                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufReaderState::Ready(buf) => {
                if buf.is_empty() {
                    let BufReaderState::Ready(buf) = std::mem::take(&mut this.0.state) else {
                        unreachable!();
                    };

                    let fut = this.0.source.read_fixed(buf.into_inner());
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
