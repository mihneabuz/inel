use std::io::Result;

use super::generic::*;
use crate::{
    group::{ReadBufferSet, ReadBufferSetPrivate},
    io::ReadSource,
    GlobalReactor,
};

use inel_reactor::{
    op::{self, DetachOp, OpExt, ReadGroup},
    submission::Submission,
};

type GroupBuffer = Option<Box<[u8]>>;
type GroupFuture = Submission<ReadGroup<ReadBufferSetPrivate, GlobalReactor>, GlobalReactor>;

struct GroupAdapter(ReadBufferSet);

impl<S: ReadSource> BufReaderAdapter<S, GroupBuffer, GroupFuture> for GroupAdapter {
    fn create_future(&self, source: &mut S, buffer: GroupBuffer) -> GroupFuture {
        if let Some(buffer) = buffer {
            op::ProvideBuffer::new(self.0.clone_private(), buffer).run_detached(&mut GlobalReactor);
        }

        op::ReadGroup::new(source.read_source(), self.0.clone_private()).run_on(GlobalReactor)
    }
}

pub struct GroupBufReader<S>(BufReaderInner<S, GroupBuffer, GroupFuture, GroupAdapter>);

impl<S> GroupBufReader<S> {
    pub fn new(source: S, set: ReadBufferSet) -> Self {
        Self(BufReaderInner::empty(None, source, GroupAdapter(set)))
    }
}

impl_bufreader!(GroupBufReader);
