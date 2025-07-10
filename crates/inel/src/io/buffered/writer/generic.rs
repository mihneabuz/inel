use std::{
    future::Future,
    io::Result,
    ops::RangeTo,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};

use crate::{
    buffer::{StableBufferExt, StableBufferMut, View},
    io::WriteSource,
};

#[derive(Default)]
pub(crate) enum BufWriterState<B, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(View<B, RangeTo<usize>>),
}

pub(crate) struct BufWriterGeneric<S, B, F, A> {
    state: BufWriterState<B, F>,
    sink: S,
    adapter: A,
}

impl<S, B, F, A> BufWriterGeneric<S, B, F, A>
where
    B: StableBufferMut,
{
    pub(crate) fn empty(buffer: B, sink: S, adapter: A) -> Self {
        Self::new(buffer, sink, 0, adapter)
    }

    pub(crate) fn new(buffer: B, sink: S, pos: usize, adapter: A) -> Self {
        Self {
            state: BufWriterState::Ready(buffer.view(..pos)),
            sink,
            adapter,
        }
    }

    fn ready(&self) -> Option<&View<B, RangeTo<usize>>> {
        match &self.state {
            BufWriterState::Ready(buf) => Some(buf),
            _ => None,
        }
    }

    pub(crate) fn buffer(&self) -> Option<&[u8]> {
        self.ready().map(|buf| buf.buffer())
    }

    pub(crate) fn capacity(&self) -> Option<usize> {
        self.ready().map(|buf| buf.inner().size())
    }

    pub(crate) fn inner(&self) -> &S {
        &self.sink
    }

    pub(crate) fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub(crate) fn into_inner(self) -> S {
        self.sink
    }

    pub(crate) fn into_raw_parts(self) -> (S, Option<(B, usize)>) {
        let Self { state, sink, .. } = self;
        let buf = match state {
            BufWriterState::Ready(buffer) => Some(buffer.into_raw_parts()),
            _ => None,
        };
        (sink, buf)
    }
}

pub(crate) trait BufWriterAdapter<S, B, F> {
    fn create_future(&self, sink: &mut S, buffer: View<B, RangeTo<usize>>) -> F;
}

impl<S, B, F, Adapter> AsyncWrite for BufWriterGeneric<S, B, F, Adapter>
where
    S: WriteSource,
    B: StableBufferMut,
    F: Future<Output = (View<B, RangeTo<usize>>, Result<usize>)> + Unpin,
    Self: Unpin,
    Adapter: BufWriterAdapter<S, B, F>,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = Pin::into_inner(self);

        match &mut this.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(_) => {
                let mut pinned = Pin::new(this);
                ready!(pinned.as_mut().poll_flush(cx))?;
                pinned.poll_write(cx, buf)
            }

            BufWriterState::Ready(internal) => {
                let spare_capacity = internal.spare_capacity();
                if spare_capacity == 0 {
                    let mut pinned = Pin::new(this);
                    ready!(pinned.as_mut().poll_flush(cx))?;
                    pinned.poll_write(cx, buf)
                } else {
                    let len = spare_capacity.min(buf.len());
                    let res = internal.fill(&buf[..len]);
                    Poll::Ready(Ok(res))
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = Pin::into_inner(self);

        match &mut this.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(fut) => {
                let (mut view, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(wrote) => {
                        assert_eq!(wrote, view.range().end);
                        view.set_pos(0);
                        this.state = BufWriterState::Ready(view);
                    }

                    Err(err) => {
                        this.state = BufWriterState::Ready(view);
                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufWriterState::Ready(internal) => {
                if !internal.is_empty() {
                    let BufWriterState::Ready(internal) = std::mem::take(&mut this.state) else {
                        unreachable!();
                    };

                    let fut = this.adapter.create_future(&mut this.sink, internal);
                    this.state = BufWriterState::Pending(fut);

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

macro_rules! impl_bufwriter {
    ($bufwriter:ident) => {
        impl<S> $bufwriter<S> {
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

        impl<S> futures::AsyncWrite for $bufwriter<S>
        where
            S: WriteSource + Unpin,
        {
            fn poll_write(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &[u8],
            ) -> std::task::Poll<Result<usize>> {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).poll_write(cx, buf)
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<()>> {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).poll_flush(cx)
            }

            fn poll_close(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<()>> {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).poll_close(cx)
            }
        }
    };
}

pub(crate) use impl_bufwriter;
