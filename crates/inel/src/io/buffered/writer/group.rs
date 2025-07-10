use std::{io::Result, ops::RangeTo};

use super::generic::*;
use crate::{
    buffer::View,
    group::WriteBufferSet,
    io::{owned::WriteOwned, AsyncWriteOwned, WriteSource},
};

type GroupBuffer = Option<Box<[u8]>>;
type GroupFuture = WriteOwned<View<GroupBuffer, RangeTo<usize>>>;

struct GroupAdapter(WriteBufferSet);
impl<S: WriteSource> BufWriterAdapter<S, GroupBuffer, GroupFuture> for GroupAdapter {
    fn create_future(
        &self,
        sink: &mut S,
        buffer: View<GroupBuffer, RangeTo<usize>>,
    ) -> GroupFuture {
        sink.write_owned(buffer)
    }

    fn pre_write(&self, buffer: &mut GroupBuffer) {
        if buffer.is_none() {
            buffer.replace(self.0.get());
        }
    }

    fn post_flush(&self, buffer: &mut GroupBuffer) {
        if let Some(buffer) = buffer.take() {
            self.0.insert(buffer);
        }
    }
}

pub struct GroupBufWriter<S>(BufWriterInner<S, GroupBuffer, GroupFuture, GroupAdapter>);

impl<S> GroupBufWriter<S> {
    pub fn new(sink: S, set: WriteBufferSet) -> Self {
        Self(BufWriterInner::empty(None, sink, GroupAdapter(set)))
    }
}

impl_bufwriter!(GroupBufWriter);
