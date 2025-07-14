use std::{cell::RefCell, io::Result, mem::ManuallyDrop, ops::Deref, rc::Rc};

use futures::{stream::FuturesUnordered, StreamExt};
use inel_reactor::{
    group::ReadBufferGroup,
    op::{self, DetachOp, OpExt},
};

use crate::{
    io::{
        AsyncWriteOwned, GroupBufReader, GroupBufWriter, ReadHandle, ReadSource, Split,
        WriteHandle, WriteSource,
    },
    GlobalReactor,
};

const PROVIDE_BATCH_SIZE: usize = 32;
const DEFAULT_BUFFER_SIZE: usize = 4096;

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

    pub async fn with_buffers(count: u16, capacity: usize) -> Result<Self> {
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
        Self::with_buffers(0, DEFAULT_BUFFER_SIZE)
    }

    pub fn with_buffers(count: u16, capacity: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(WriteBufferSetInner::with_buffers(
                count, capacity,
            ))),
        }
    }

    pub fn insert(&self, buffer: Box<[u8]>) {
        self.inner.borrow_mut().insert(buffer);
    }

    pub fn get(&self) -> Box<[u8]> {
        self.inner.borrow_mut().get()
    }

    pub async fn write(&self, sink: &mut impl WriteSource, buffer: Box<[u8]>) -> Result<usize> {
        let (buf, res) = sink.write_owned(buffer).await;
        self.insert(buf);
        res
    }

    pub fn supply_to<S>(&self, sink: S) -> GroupBufWriter<S> {
        GroupBufWriter::new(sink, self.clone())
    }
}

struct WriteBufferSetInner {
    buffers: Vec<Box<[u8]>>,
    capacity: usize,
}

impl WriteBufferSetInner {
    pub fn with_buffers(count: u16, capacity: usize) -> Self {
        Self {
            buffers: (0..count)
                .map(|_| vec![0; capacity].into_boxed_slice())
                .collect(),
            capacity,
        }
    }

    pub fn insert(&mut self, buffer: Box<[u8]>) {
        self.buffers.push(buffer);
    }

    pub fn get(&mut self) -> Box<[u8]> {
        self.buffers
            .pop()
            .unwrap_or(vec![0; self.capacity].into_boxed_slice())
    }
}

pub struct BufferShareGroupOptions {
    buffer_capacity: usize,
    initial_read_count: u16,
    initial_write_count: u16,
}

impl Default for BufferShareGroupOptions {
    fn default() -> Self {
        Self {
            buffer_capacity: DEFAULT_BUFFER_SIZE,
            initial_read_count: 128,
            initial_write_count: 16,
        }
    }
}

impl BufferShareGroupOptions {
    pub fn buffer_capacity(mut self, capacity: usize) -> Self {
        self.buffer_capacity = capacity;
        self
    }

    pub fn initial_read_buffers(mut self, count: u16) -> Self {
        self.initial_read_count = count;
        self
    }

    pub fn initial_write_buffers(mut self, count: u16) -> Self {
        self.initial_write_count = count;
        self
    }

    pub async fn build(self) -> Result<BufferShareGroup> {
        let read =
            ReadBufferSet::with_buffers(self.initial_read_count, self.buffer_capacity).await?;
        let write = WriteBufferSet::with_buffers(self.initial_write_count, self.buffer_capacity);
        Ok(BufferShareGroup { read, write })
    }
}

pub struct BufferShareGroup {
    read: ReadBufferSet,
    write: WriteBufferSet,
}

pub type ShareBufReader<T> = GroupBufReader<ReadHandle<T>>;
pub type ShareBufWriter<T> = GroupBufWriter<WriteHandle<T>>;
pub type ShareBuffered<T> = GroupBufWriter<GroupBufReader<T>>;

impl BufferShareGroup {
    pub fn options() -> BufferShareGroupOptions {
        BufferShareGroupOptions::default()
    }

    pub async fn new() -> Result<Self> {
        Self::options().build().await
    }

    pub fn insert_read_buffer(&self, buffer: Box<[u8]>) {
        self.read.insert(buffer);
    }

    pub fn insert_write_buffer(&self, buffer: Box<[u8]>) {
        self.write.insert(buffer);
    }

    pub fn supply_to<S>(&self, source: S) -> ShareBuffered<S> {
        self.write.supply_to(self.read.supply_to(source))
    }

    pub fn supply_to_split<S>(&self, source: S) -> (ShareBufReader<S>, ShareBufWriter<S>)
    where
        S: ReadSource + WriteSource,
    {
        let (read_handle, write_handle) = source.split();
        let reader = self.read.supply_to(read_handle);
        let writer = self.write.supply_to(write_handle);
        (reader, writer)
    }
}
