use std::{io::Write, ops::RangeTo};

use inel_reactor::buffer::{StableBufferMut, View};

pub(crate) struct WBuffer<B: StableBufferMut> {
    buf: B,
    pos: usize,
}

impl<B: StableBufferMut> WBuffer<B> {
    pub(crate) fn new(buf: B, pos: usize) -> Self {
        Self { buf, pos }
    }

    pub(crate) fn buffer(&self) -> &[u8] {
        &self.buf.as_slice()[..self.pos]
    }

    pub(crate) fn is_empty(&mut self) -> bool {
        self.pos == 0
    }

    pub(crate) fn capacity(&self) -> usize {
        self.buf.size()
    }

    pub(crate) fn spare_capacity(&self) -> usize {
        self.buf.size() - self.pos
    }

    pub(crate) fn write(&mut self, data: &[u8]) -> usize {
        let mut writer = &mut self.buf.as_mut_slice()[self.pos..];
        let wrote = writer.write(data).unwrap();
        self.pos += wrote;
        wrote
    }

    pub(crate) fn into_inner(self) -> (B, usize) {
        (self.buf, self.pos)
    }

    pub(crate) fn into_view(self) -> View<B, RangeTo<usize>> {
        View::new(self.buf, ..self.pos)
    }
}

impl WBuffer<Box<[u8]>> {
    pub(crate) fn empty(capacity: usize) -> Self {
        Self::new(vec![0; capacity].into_boxed_slice(), 0)
    }
}

#[derive(Default)]
pub(crate) enum BufWriterState<B: StableBufferMut, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(WBuffer<B>),
}

pub(crate) struct BufWriterGeneric<S, B: StableBufferMut, F> {
    pub(crate) sink: S,
    pub(crate) state: BufWriterState<B, F>,
}

impl<S, B, F> BufWriterGeneric<S, B, F>
where
    B: StableBufferMut,
{
    pub(crate) fn ready(&self) -> Option<&WBuffer<B>> {
        match &self.state {
            BufWriterState::Ready(buf) => Some(buf),
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
        &self.sink
    }

    pub(crate) fn inner_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub(crate) fn into_inner(self) -> S {
        self.sink
    }

    pub(crate) fn into_raw_parts(self) -> (S, Option<(B, usize)>) {
        let Self { state, sink } = self;
        let buf = match state {
            BufWriterState::Ready(buffer) => Some(buffer.into_inner()),
            _ => None,
        };
        (sink, buf)
    }
}
