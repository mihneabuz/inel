use inel_reactor::buffer::StableBuffer;

#[derive(Default)]
pub(crate) enum BufWriterState<B: StableBuffer, F> {
    #[default]
    Empty,
    Pending(F),
    Ready(B),
}

pub(crate) struct BufWriterGeneric<S, B: StableBuffer, F> {
    pub(crate) sink: S,
    pub(crate) state: BufWriterState<B, F>,
}

impl<S, B, F> BufWriterGeneric<S, B, F>
where
    B: StableBuffer,
{
    pub(crate) fn ready(&self) -> Option<&B> {
        match &self.state {
            BufWriterState::Empty => None,
            BufWriterState::Pending(_) => None,
            BufWriterState::Ready(buf) => Some(buf),
        }
    }

    pub(crate) fn buffer(&self) -> Option<&[u8]> {
        self.ready().map(|buf| buf.as_slice())
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

    pub(crate) fn into_raw_parts(self) -> (S, Option<B>) {
        let Self { state, sink } = self;
        let buf = match state {
            BufWriterState::Empty => None,
            BufWriterState::Pending(_) => None,
            BufWriterState::Ready(buffer) => Some(buffer),
        };
        (sink, buf)
    }
}
