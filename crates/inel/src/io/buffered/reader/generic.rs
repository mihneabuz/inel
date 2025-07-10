use std::{
    future::Future,
    io::Result,
    ops::Range,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncRead, FutureExt};

use crate::{
    buffer::{StableBufferExt, StableBufferMut, View},
    io::ReadSource,
};

#[derive(Default)]
pub(crate) enum BufReaderState<B, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(View<B, Range<usize>>),
}

pub(crate) struct BufReaderInner<S, B, F, A> {
    state: BufReaderState<B, F>,
    source: S,
    adapter: A,
}

impl<S, B, F, A> BufReaderInner<S, B, F, A>
where
    B: StableBufferMut,
{
    pub(crate) fn empty(buffer: B, source: S, adapter: A) -> Self {
        Self::new(buffer, source, 0, 0, adapter)
    }

    pub(crate) fn new(buffer: B, source: S, pos: usize, filled: usize, adapter: A) -> Self {
        Self {
            state: BufReaderState::Ready(buffer.view(pos..filled)),
            source,
            adapter,
        }
    }

    fn ready(&self) -> Option<&View<B, Range<usize>>> {
        match &self.state {
            BufReaderState::Ready(buf) => Some(buf),
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
        &self.source
    }

    pub(crate) fn inner_mut(&mut self) -> &mut S {
        &mut self.source
    }

    pub(crate) fn into_inner(self) -> S {
        self.source
    }

    pub(crate) fn into_raw_parts(self) -> (S, Option<(B, usize, usize)>) {
        let Self { state, source, .. } = self;
        let buf = match state {
            BufReaderState::Ready(buf) => Some(buf.into_raw_parts()),
            _ => None,
        };
        (source, buf)
    }
}

pub(crate) trait BufReaderAdapter<S, B, F> {
    fn create_future(&self, source: &mut S, buffer: B) -> F;
    fn post_consume(&self, _view: &mut View<B, Range<usize>>) {}
}

impl<S, B, F, A> AsyncRead for BufReaderInner<S, B, F, A>
where
    S: ReadSource,
    B: StableBufferMut,
    Self: AsyncBufRead,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        let mut inner = ready!(self.as_mut().poll_fill_buf(cx))?;
        let len = std::io::Read::read(&mut inner, buf)?;
        self.consume(len);
        Poll::Ready(Ok(len))
    }
}

impl<S, B, F, Adapter> AsyncBufRead for BufReaderInner<S, B, F, Adapter>
where
    S: ReadSource,
    B: StableBufferMut,
    F: Future<Output = (B, Result<usize>)> + Unpin,
    Self: Unpin,
    Adapter: BufReaderAdapter<S, B, F>,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        let this = Pin::into_inner(self);

        match &mut this.state {
            BufReaderState::Empty => unreachable!(),

            BufReaderState::Pending(fut) => {
                let (buf, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(filled) => {
                        this.state = BufReaderState::Ready(buf.view(0..filled));
                    }

                    Err(err) => {
                        this.state = BufReaderState::Ready(buf.view(0..0));
                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufReaderState::Ready(buf) => {
                if buf.is_empty() {
                    let BufReaderState::Ready(buf) = std::mem::take(&mut this.state) else {
                        unreachable!();
                    };

                    let fut = this
                        .adapter
                        .create_future(&mut this.source, buf.into_raw_parts().0);

                    this.state = BufReaderState::Pending(fut);

                    return Pin::new(this).poll_fill_buf(cx);
                }
            }
        };

        Poll::Ready(Ok(this.buffer().unwrap()))
    }

    fn consume(self: Pin<&mut Self>, amt: usize) {
        let this = Pin::into_inner(self);
        if let BufReaderState::Ready(buf) = &mut this.state {
            buf.consume(amt);
            this.adapter.post_consume(buf);
        }
    }
}

macro_rules! impl_bufreader {
    ($bufreader:ident) => {
        impl<S> $bufreader<S> {
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

        impl<S> futures::AsyncRead for $bufreader<S>
        where
            S: ReadSource + Unpin,
        {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &mut [u8],
            ) -> std::task::Poll<Result<usize>> {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).poll_read(cx, buf)
            }
        }

        impl<S> futures::AsyncBufRead for $bufreader<S>
        where
            S: ReadSource + Unpin,
        {
            fn poll_fill_buf(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<&[u8]>> {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).poll_fill_buf(cx)
            }

            fn consume(self: std::pin::Pin<&mut Self>, amt: usize) {
                std::pin::Pin::new(&mut std::pin::Pin::into_inner(self).0).consume(amt)
            }
        }
    };
}

pub(crate) use impl_bufreader;
