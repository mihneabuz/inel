use std::{
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};

use crate::{
    buffer::Fixed,
    io::{owned::WriteFixed, AsyncWriteFixed},
};

use super::generic::{BufWriterGeneric, BufWriterState};

type FixedVecBuf = Fixed<Vec<u8>>;
type FixedWriteFut = WriteFixed<FixedVecBuf>;

pub struct FixedBufWriter<S>(BufWriterGeneric<S, FixedVecBuf, FixedWriteFut>);

impl<S> FixedBufWriter<S> {
    pub(crate) fn from_raw(sink: S, buffer: FixedVecBuf) -> Self {
        Self(BufWriterGeneric {
            state: BufWriterState::Ready(buffer),
            sink,
        })
    }

    pub fn capacity(&self) -> Option<usize> {
        self.0.ready().map(|fixed| fixed.inner().capacity())
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
    S: AsyncWriteFixed + Unpin,
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

            BufWriterState::Ready(fixed) => {
                if fixed.inner_mut().spare_capacity_mut().len() >= buf.len() {
                    let res = std::io::Write::write(fixed.inner_mut(), buf);

                    Poll::Ready(res)
                } else {
                    let mut pinned = Pin::new(this);
                    ready!(pinned.as_mut().poll_flush(cx))?;
                    pinned.as_mut().poll_write(cx, buf)
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(fut) => {
                let (mut fixed, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(wrote) => {
                        assert_eq!(fixed.inner().len(), wrote);
                        fixed.inner_mut().clear();
                        this.0.state = BufWriterState::Ready(fixed);
                    }

                    Err(err) => {
                        this.0.state = BufWriterState::Ready(fixed);
                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufWriterState::Ready(fixed) => {
                if !fixed.inner().is_empty() {
                    let BufWriterState::Ready(fixed) = std::mem::take(&mut this.0.state) else {
                        unreachable!();
                    };

                    let fut = this.0.sink.write_fixed(fixed);
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
