use std::{
    io::Result,
    ops::RangeTo,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};
use inel_reactor::buffer::View;

use crate::{
    buffer::Fixed,
    io::{owned::WriteFixed, AsyncWriteOwned},
};

use super::generic::{BufWriterGeneric, BufWriterState, WBuffer};

type FixedWriteFut = WriteFixed<View<Fixed, RangeTo<usize>>>;

pub struct FixedBufWriter<S>(BufWriterGeneric<S, Fixed, FixedWriteFut>);

impl<S> FixedBufWriter<S> {
    pub(crate) fn from_raw(sink: S, buffer: WBuffer<Fixed>) -> Self {
        let state = BufWriterState::Ready(buffer);
        Self(BufWriterGeneric { state, sink })
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

impl<S> AsyncWrite for FixedBufWriter<S>
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

                    let fut = this.0.sink.write_fixed(internal.into_view());
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
