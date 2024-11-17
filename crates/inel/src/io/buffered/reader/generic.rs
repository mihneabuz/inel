use std::cmp;

use inel_reactor::buffer::StableBuffer;

pub(crate) struct Buffer<B: StableBuffer> {
    buf: B,
    pos: usize,
    filled: usize,
}

impl<B: StableBuffer> Buffer<B> {
    pub(crate) fn new(buf: B, filled: usize) -> Self {
        Self {
            buf,
            pos: 0,
            filled,
        }
    }

    pub(crate) fn buffer(&self) -> &[u8] {
        &self.buf.as_slice()[self.pos..self.filled]
    }

    pub(crate) fn capacity(&self) -> usize {
        self.buf.size()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.pos >= self.filled
    }

    pub(crate) fn consume(&mut self, amt: usize) {
        self.pos = cmp::min(self.pos + amt, self.filled);
    }

    pub(crate) fn into_inner(self) -> B {
        self.buf
    }
}

impl Buffer<Box<[u8]>> {
    pub(crate) fn empty(capacity: usize) -> Self {
        Self::new(vec![0; capacity].into_boxed_slice(), 0)
    }
}

#[derive(Default)]
pub(crate) enum BufReaderState<B: StableBuffer, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(Buffer<B>),
}

pub(crate) struct BufReaderGeneric<S, B: StableBuffer, F> {
    pub(crate) state: BufReaderState<B, F>,
    pub(crate) source: S,
}

impl<S, B, F> BufReaderGeneric<S, B, F>
where
    B: StableBuffer,
{
    fn ready(&self) -> Option<&Buffer<B>> {
        match &self.state {
            BufReaderState::Empty => None,
            BufReaderState::Pending(_) => None,
            BufReaderState::Ready(buf) => Some(buf),
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

    pub(crate) fn into_raw_parts(self) -> (S, Option<B>) {
        let Self { state, source } = self;
        let buf = match state {
            BufReaderState::Empty => None,
            BufReaderState::Pending(_) => None,
            BufReaderState::Ready(buffer) => Some(buffer.into_inner()),
        };
        (source, buf)
    }
}
