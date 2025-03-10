use std::{
    io::{Error, Result},
    ops::RangeTo,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};
use inel_reactor::buffer::View;

use super::{
    generic::{BufWriterGeneric, BufWriterState, WBuffer},
    FixedBufWriter,
};
use crate::{
    buffer::Fixed,
    io::{owned::WriteOwned, AsyncWriteOwned},
};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type BoxBuf = Box<[u8]>;
type WriteFut = WriteOwned<View<BoxBuf, RangeTo<usize>>>;

pub struct BufWriter<S>(BufWriterGeneric<S, BoxBuf, WriteFut>);

impl<S> BufWriter<S>
where
    S: AsyncWriteOwned,
{
    pub fn new(sink: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, sink)
    }

    pub fn with_capacity(capacity: usize, sink: S) -> Self {
        let state = BufWriterState::Ready(WBuffer::empty(capacity));
        Self(BufWriterGeneric { state, sink })
    }

    pub fn fix(self) -> Result<FixedBufWriter<S>> {
        let (source, buf) = self.0.into_raw_parts();

        let (buf, len) = buf.ok_or(Error::other("BufWriter was in use"))?;
        let fixed = Fixed::register(buf)?;

        Ok(FixedBufWriter::from_raw(source, WBuffer::new(fixed, len)))
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

impl<S> AsyncWrite for BufWriter<S>
where
    S: AsyncWriteOwned + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(_) => {
                let mut pinned = Pin::new(this);
                ready!(pinned.as_mut().poll_flush(cx))?;
                pinned.as_mut().poll_write(cx, buf)
            }

            BufWriterState::Ready(internal) => {
                let spare_capacity = internal.spare_capacity();
                if spare_capacity == 0 {
                    let mut pinned = Pin::new(this);
                    ready!(pinned.as_mut().poll_flush(cx))?;
                    pinned.as_mut().poll_write(cx, buf)
                } else {
                    let len = spare_capacity.min(buf.len());
                    let res = internal.write(&buf[..len]);
                    Poll::Ready(Ok(res))
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(fut) => {
                let (view, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(wrote) => {
                        assert_eq!(wrote, view.range().end);
                        this.0.state = BufWriterState::Ready(WBuffer::new(view.unview(), 0));
                    }

                    Err(err) => {
                        let pos = view.range().end;
                        this.0.state = BufWriterState::Ready(WBuffer::new(view.unview(), pos));
                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufWriterState::Ready(internal) => {
                if !internal.is_empty() {
                    let BufWriterState::Ready(internal) = std::mem::take(&mut this.0.state) else {
                        unreachable!();
                    };

                    let fut = this.0.sink.write_owned(internal.into_view());
                    this.0.state = BufWriterState::Pending(fut);

                    return Pin::new(this).poll_flush(cx);
                }
            }
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }
}
