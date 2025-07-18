use std::io::Result;

use super::generic::*;
use crate::{
    buffer::View,
    group::{ReadBufferSet, ReadBufferSetInner},
    io::ReadSource,
    GlobalReactor,
};

use inel_reactor::{
    op::{self, OpExt, ReadGroup},
    submission::Submission,
};

type GroupBuffer = Option<Box<[u8]>>;
type GroupFuture = Submission<ReadGroup<ReadBufferSetInner>, GlobalReactor>;

struct GroupAdapter(ReadBufferSet);

impl<S: ReadSource> BufReaderAdapter<S, GroupBuffer, GroupFuture> for GroupAdapter {
    fn create_future(&self, source: &mut S, buffer: GroupBuffer) -> GroupFuture {
        if let Some(buffer) = buffer {
            self.0.insert(buffer);
        }

        self.0.recycle();

        op::ReadGroup::new(source.read_source(), self.0.inner()).run_on(GlobalReactor)
    }

    fn post_consume(&self, view: &mut View<GroupBuffer, std::ops::Range<usize>>) {
        if view.is_empty() {
            if let Some(buffer) = view.inner_mut().take() {
                self.0.insert(buffer);
            }
        }
    }
}

pub struct GroupBufReader<S>(BufReaderInner<S, GroupBuffer, GroupFuture, GroupAdapter>);

impl<S> GroupBufReader<S> {
    pub fn new(source: S, set: ReadBufferSet) -> Self {
        Self(BufReaderInner::empty(None, source, GroupAdapter(set)))
    }
}

impl_bufreader!(GroupBufReader);
