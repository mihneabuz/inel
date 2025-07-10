use std::{io::Result, ops::RangeTo};

use super::generic::*;
use crate::{
    buffer::{Fixed, View},
    io::{owned::WriteFixed, AsyncWriteOwned, WriteSource},
};

type FixedWrite = WriteFixed<View<Fixed, RangeTo<usize>>>;

struct FixedAdapter;
impl<S: WriteSource> BufWriterAdapter<S, Fixed, FixedWrite> for FixedAdapter {
    fn create_future(&self, sink: &mut S, buffer: View<Fixed, RangeTo<usize>>) -> FixedWrite {
        sink.write_fixed(buffer)
    }
}

pub struct FixedBufWriter<S>(BufWriterGeneric<S, Fixed, FixedWrite, FixedAdapter>);

impl<S> FixedBufWriter<S> {
    pub(crate) fn from_raw(sink: S, buffer: Fixed, pos: usize) -> Self {
        Self(BufWriterGeneric::new(buffer, sink, pos, FixedAdapter))
    }
}

impl_bufwriter!(FixedBufWriter);
