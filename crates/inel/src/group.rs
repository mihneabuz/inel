use std::{cell::RefCell, io::Result, mem::ManuallyDrop, ops::RangeBounds, rc::Rc};

use futures::{stream::FuturesUnordered, StreamExt};
use inel_reactor::{
    buffer::View,
    group::{AsBufferGroup, ReadBufferGroup},
    op::{self, DetachOp, OpExt},
};

use crate::{
    io::{
        AsyncWriteOwned, GroupBufReader, GroupBufWriter, ReadHandle, ReadSource, Split,
        WriteHandle, WriteSource,
    },
    GlobalReactor,
};

const PROVIDE_BATCH_SIZE: usize = 16;
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

        let provides = (0..count)
            .map(|_| {
                op::ProvideBuffer::new(this.inner(), vec![0; capacity].into_boxed_slice())
                    .run_on(GlobalReactor)
            })
            .collect::<FuturesUnordered<_>>();

        provides
            .for_each_concurrent(PROVIDE_BATCH_SIZE, async |_| {})
            .await;

        Ok(this)
    }

    pub fn insert(&self, buffer: Box<[u8]>) {
        op::ProvideBuffer::new(self.inner(), buffer).run_detached(&mut GlobalReactor);
    }

    pub fn recycle(&self) {
        while let Some(buffer) = self.inner.group.get_cancelled() {
            self.insert(buffer);
        }
    }

    pub async fn read<S>(&self, source: &mut S) -> Result<(Box<[u8]>, usize)>
    where
        S: ReadSource,
    {
        self.recycle();

        let (buf, res) = op::ReadGroup::new(source.read_source(), self.inner())
            .run_on(GlobalReactor)
            .await;

        match res {
            Ok(read) => Ok((buf.unwrap(), read)),
            Err(err) => {
                if let Some(buf) = buf {
                    self.insert(buf);
                }

                Err(err)
            }
        }
    }

    pub fn supply_to<S>(&self, source: S) -> GroupBufReader<S> {
        GroupBufReader::new(source, self.clone())
    }

    pub(crate) fn inner(&self) -> &Rc<ReadBufferSetInner> {
        &self.inner
    }
}

impl AsBufferGroup for ReadBufferSetInner {
    fn as_group(&self) -> &ReadBufferGroup {
        self.group()
    }
}

pub(crate) struct ReadBufferSetInner {
    group: ManuallyDrop<ReadBufferGroup>,
}

impl ReadBufferSetInner {
    fn new() -> Result<Self> {
        ReadBufferGroup::register(&mut GlobalReactor).map(|group| Self {
            group: ManuallyDrop::new(group),
        })
    }

    fn group(&self) -> &ReadBufferGroup {
        &self.group
    }
}

impl Drop for ReadBufferSetInner {
    fn drop(&mut self) {
        let group = unsafe { ManuallyDrop::take(&mut self.group) };
        crate::spawn(async move {
            op::RemoveBuffers::new(group)
                .run_on(GlobalReactor)
                .await
                .release(&mut GlobalReactor);
        });
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

    pub async fn write<S, R>(&self, sink: &mut S, buffer: View<Box<[u8]>, R>) -> Result<usize>
    where
        S: WriteSource,
        R: RangeBounds<usize>,
    {
        let (buf, res) = sink.write_owned(buffer).await;
        self.insert(buf.unview());
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

#[derive(Clone)]
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

    pub fn get_write_buffer(&self) -> Box<[u8]> {
        self.write.get()
    }

    pub async fn read<S>(&self, source: &mut S) -> Result<(Box<[u8]>, usize)>
    where
        S: ReadSource,
    {
        self.read.read(source).await
    }

    pub async fn write<S, R>(&self, sink: &mut S, buffer: View<Box<[u8]>, R>) -> Result<usize>
    where
        S: WriteSource,
        R: RangeBounds<usize>,
    {
        self.write.write(sink, buffer).await
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
