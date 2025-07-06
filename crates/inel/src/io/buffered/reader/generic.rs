use std::{
    cmp,
    future::Future,
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncBufRead, AsyncRead, FutureExt};

use crate::io::ReadSource;

pub(crate) struct DoubleCursor<B> {
    buf: B,
    pos: usize,
    filled: usize,
}

impl<B> DoubleCursor<B> {
    pub(crate) fn new(buf: B, pos: usize, filled: usize) -> Self {
        Self { buf, pos, filled }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos >= self.filled
    }

    pub(crate) fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.filled);
    }

    pub(crate) fn into_raw_parts(self) -> (B, usize, usize) {
        (self.buf, self.pos, self.filled)
    }
}

impl<B: AsRef<[u8]>> DoubleCursor<B> {
    pub(crate) fn buffer(&self) -> &[u8] {
        &self.buf.as_ref()[self.pos..self.filled]
    }

    pub(crate) fn capacity(&self) -> usize {
        self.buf.as_ref().len()
    }
}

#[derive(Default)]
pub(crate) enum BufReaderState<B, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(DoubleCursor<B>),
}

pub(crate) struct BufReaderGeneric<S, B, F, A> {
    pub(crate) state: BufReaderState<B, F>,
    pub(crate) source: S,
    pub(crate) adapter: A,
}

impl<S, B, F, A> BufReaderGeneric<S, B, F, A> {
    pub(crate) fn empty(buffer: B, source: S, adapter: A) -> Self {
        Self::new(buffer, source, 0, 0, adapter)
    }

    pub(crate) fn new(buffer: B, source: S, pos: usize, filled: usize, adapter: A) -> Self {
        Self {
            state: BufReaderState::Ready(DoubleCursor::new(buffer, pos, filled)),
            source,
            adapter,
        }
    }
}

impl<S, B, F, A> BufReaderGeneric<S, B, F, A>
where
    B: AsRef<[u8]>,
{
    fn ready(&self) -> Option<&DoubleCursor<B>> {
        match &self.state {
            BufReaderState::Ready(buf) => Some(buf),
            _ => None,
        }
    }

    pub(crate) fn buffer(&self) -> Option<&[u8]> {
        self.ready().map(|buf| buf.buffer())
    }

    pub(crate) fn capacity(&self) -> Option<usize> {
        self.ready().map(|buf| buf.capacity())
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

impl<S, B, F, A> AsyncRead for BufReaderGeneric<S, B, F, A>
where
    S: ReadSource,
    B: AsRef<[u8]>,
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

pub(crate) trait BufReaderAdapter<S, B, F> {
    fn create_future(&self, source: &mut S, buffer: B) -> F;
}

impl<S, B, F, Adapter> AsyncBufRead for BufReaderGeneric<S, B, F, Adapter>
where
    S: ReadSource,
    B: AsRef<[u8]>,
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
                        this.state = BufReaderState::Ready(DoubleCursor::new(buf, 0, filled));
                    }

                    Err(err) => {
                        this.state = BufReaderState::Ready(DoubleCursor::new(buf, 0, 0));
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
        if let BufReaderState::Ready(buf) = &mut Pin::into_inner(self).state {
            buf.consume(amt);
        }
    }
}
