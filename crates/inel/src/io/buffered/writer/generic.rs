use std::{
    future::Future,
    io::Result,
    ops::RangeTo,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};
use inel_reactor::{
    op::{OpExt, Shutdown},
    submission::Submission,
};

use crate::{
    buffer::{StableBufferExt, StableBufferMut, View},
    io::WriteSource,
    GlobalReactor,
};

#[derive(Default)]
pub(crate) enum BufWriterState<B, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(View<B, RangeTo<usize>>),
}

pub(crate) struct BufWriterInner<S, B, F, A> {
    state: BufWriterState<B, F>,
    pub(crate) sink: S,
    adapter: A,
    shutdown: Option<Submission<Shutdown, GlobalReactor>>,
}

impl<S, B, F, A> BufWriterInner<S, B, F, A>
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
            shutdown: None,
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

    pub(crate) fn into_raw_parts(self) -> (S, Option<(B, usize)>) {
        let Self { state, sink, .. } = self;
        let buf = match state {
            BufWriterState::Ready(buffer) => Some(buffer.into_raw_parts()),
            _ => None,
        };
        (sink, buf)
    }
}

impl<S, B, F, Adapter> BufWriterInner<S, B, F, Adapter>
where
    S: WriteSource,
    B: StableBufferMut,
    F: Future<Output = (View<B, RangeTo<usize>>, Result<usize>)> + Unpin,
    Self: Unpin,
    Adapter: BufWriterAdapter<S, B, F>,
{
    pub(crate) fn poll_flush_and_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
        ready!(self.as_mut().poll_flush(cx))?;
        self.poll_write(cx, buf)
    }
}

pub(crate) trait BufWriterAdapter<S, B, F> {
    fn create_future(&self, sink: &mut S, buffer: View<B, RangeTo<usize>>) -> F;
    fn pre_write(&self, _buffer: &mut B) {}
    fn post_flush(&self, _buffer: &mut B) {}
}

impl<S, B, F, Adapter> AsyncWrite for BufWriterInner<S, B, F, Adapter>
where
    S: WriteSource,
    B: StableBufferMut,
    F: Future<Output = (View<B, RangeTo<usize>>, Result<usize>)> + Unpin,
    Self: Unpin,
    Adapter: BufWriterAdapter<S, B, F>,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = self.get_mut();

        match &mut this.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(_) => Pin::new(this).poll_flush_and_write(cx, buf),

            BufWriterState::Ready(internal) => {
                this.adapter.pre_write(internal.inner_mut());
                let spare_capacity = internal.spare_capacity();
                if spare_capacity == 0 {
                    Pin::new(this).poll_flush_and_write(cx, buf)
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
                        this.adapter.post_flush(view.inner_mut());
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

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if !self.sink.need_shutdown() {
            return Poll::Ready(Ok(()));
        }

        if self.shutdown.is_none() {
            self.shutdown = Some(
                Shutdown::new(&self.sink.write_source(), std::net::Shutdown::Write)
                    .run_on(GlobalReactor),
            );
        }

        Pin::new(self.shutdown.as_mut().unwrap()).poll(cx)
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
                &self.0.sink
            }

            pub fn inner_mut(&mut self) -> &mut S {
                &mut self.0.sink
            }

            pub fn into_inner(self) -> S {
                self.0.sink
            }

            fn sink(self: std::pin::Pin<&mut Self>) -> std::pin::Pin<&mut S>
            where
                S: Unpin,
            {
                std::pin::Pin::new(&mut self.get_mut().0.sink)
            }
        }

        impl<S> futures::AsyncRead for $bufwriter<S>
        where
            S: futures::AsyncRead + Unpin,
        {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &mut [u8],
            ) -> std::task::Poll<Result<usize>> {
                self.sink().poll_read(cx, buf)
            }
        }

        impl<S> futures::AsyncBufRead for $bufwriter<S>
        where
            S: futures::AsyncBufRead + Unpin,
        {
            fn poll_fill_buf(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<&[u8]>> {
                self.sink().poll_fill_buf(cx)
            }

            fn consume(self: std::pin::Pin<&mut Self>, amt: usize) {
                self.sink().consume(amt)
            }
        }

        impl<S> crate::io::ReadSource for $bufwriter<S>
        where
            S: crate::io::ReadSource,
        {
            fn read_source(&self) -> inel_reactor::source::Source {
                self.inner().read_source()
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
                std::pin::Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<()>> {
                std::pin::Pin::new(&mut self.get_mut().0).poll_flush(cx)
            }

            fn poll_close(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<()>> {
                std::pin::Pin::new(&mut self.get_mut().0).poll_close(cx)
            }
        }
    };
}

pub(crate) use impl_bufwriter;
