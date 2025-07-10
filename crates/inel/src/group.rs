use std::{cell::RefCell, io::Result, mem::ManuallyDrop, ops::Deref, rc::Rc};

use futures::{stream::FuturesUnordered, StreamExt};
use inel_reactor::{
    group::ReadBufferGroup,
    op::{self, DetachOp, OpExt},
};

use crate::{
    io::{AsyncWriteOwned, GroupBufReader, GroupBufWriter, ReadSource, WriteSource},
    GlobalReactor,
};

const PROVIDE_BATCH_SIZE: usize = 32;

#[derive(Clone)]
pub struct ReadBufferSet {
    inner: Rc<ReadBufferSetInner>,
}

impl ReadBufferSet {
    pub fn empty() -> Result<Self> {
        ReadBufferSetInner::new().map(|inner| Self {
            inner: Rc::new(inner),
        })
    }

    pub async fn new(count: u16, capacity: usize) -> Result<Self> {
        let this = Self::empty()?;

        let mut provides = FuturesUnordered::new();

        for _ in 0..count {
            let fut =
                op::ProvideBuffer::new(this.clone_private(), vec![0; capacity].into_boxed_slice())
                    .run_on(GlobalReactor);

            provides.push(fut);

            if provides.len() >= PROVIDE_BATCH_SIZE {
                while let Some(provide) = provides.next().await {
                    debug_assert!(provide.is_ok());
                }
            }
        }

        while let Some(provide) = provides.next().await {
            debug_assert!(provide.is_ok());
        }

        std::mem::drop(provides);

        Ok(this)
    }

    pub fn insert(&self, buffer: Box<[u8]>) {
        op::ProvideBuffer::new(self.clone_private(), buffer).run_detached(&mut GlobalReactor);
    }

    pub async fn read(&self, source: &mut impl ReadSource) -> (Option<Box<[u8]>>, Result<usize>) {
        op::ReadGroup::new(source.read_source(), self.clone_private())
            .run_on(GlobalReactor)
            .await
    }

    pub fn supply_to<S>(&self, source: S) -> GroupBufReader<S> {
        GroupBufReader::new(source, self.clone())
    }

    pub(crate) fn clone_private(&self) -> ReadBufferSetPrivate {
        ReadBufferSetPrivate {
            inner: self.inner.clone(),
        }
    }
}

pub(crate) struct ReadBufferSetPrivate {
    inner: Rc<ReadBufferSetInner>,
}

impl Deref for ReadBufferSetPrivate {
    type Target = ReadBufferGroup<GlobalReactor>;

    fn deref(&self) -> &Self::Target {
        self.inner.group()
    }
}

struct ReadBufferSetInner {
    group: ManuallyDrop<ReadBufferGroup<GlobalReactor>>,
}

impl ReadBufferSetInner {
    fn new() -> Result<Self> {
        ReadBufferGroup::new(GlobalReactor).map(|group| Self {
            group: ManuallyDrop::new(group),
        })
    }

    fn group(&self) -> &ReadBufferGroup<GlobalReactor> {
        &self.group
    }
}

impl Drop for ReadBufferSetInner {
    fn drop(&mut self) {
        let group = unsafe { ManuallyDrop::take(&mut self.group) };
        crate::spawn(op::ReleaseGroup::new(group).run_on(GlobalReactor));
    }
}

#[derive(Clone)]
pub struct WriteBufferSet {
    inner: Rc<RefCell<WriteBufferSetInner>>,
}

impl WriteBufferSet {
    pub fn empty() -> Self {
        Self {
            inner: Rc::new(RefCell::new(WriteBufferSetInner::empty())),
        }
    }

    pub fn insert(&self, buffer: Box<[u8]>) {
        self.inner.borrow_mut().insert(buffer);
    }

    pub fn get(&self) -> Box<[u8]> {
        self.inner.borrow_mut().get()
    }

    pub async fn write(&self, sink: &mut impl WriteSource) -> (Box<[u8]>, Result<usize>) {
        sink.write_owned(self.get()).await
    }

    pub fn supply_to<S>(&self, sink: S) -> GroupBufWriter<S> {
        GroupBufWriter::new(sink, self.clone())
    }
}

struct WriteBufferSetInner {
    buffers: Vec<Box<[u8]>>,
}

impl WriteBufferSetInner {
    pub fn empty() -> Self {
        Self {
            buffers: Vec::with_capacity(32),
        }
    }

    pub fn insert(&mut self, buffer: Box<[u8]>) {
        self.buffers.push(buffer);
    }

    pub fn get(&mut self) -> Box<[u8]> {
        self.buffers
            .pop()
            .unwrap_or(vec![0; 2048].into_boxed_slice())
    }
}
