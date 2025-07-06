use std::{
    cell::RefCell,
    future::Future,
    io::Result,
    mem::ManuallyDrop,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures::{stream::FuturesOrdered, AsyncBufRead, AsyncBufReadExt, AsyncRead, StreamExt};

use crate::{
    io::{
        buffered::reader::generic::{BufReaderAdapter, BufReaderGeneric},
        ReadSource,
    },
    GlobalReactor,
};

use inel_reactor::{
    op::{self, OpExt},
    BufferGroupId, RingReactor, Source,
};

#[derive(Clone)]
pub struct ReadBuffers {
    inner: Rc<RefCell<ReadBuffersInner>>,
}

impl ReadBuffers {
    pub async fn new(capacity: usize, count: u16) -> Result<Self> {
        let inner = ReadBuffersInner::new(capacity, count).await?;
        Ok(Self {
            inner: Rc::new(RefCell::new(inner)),
        })
    }

    pub fn provide_to<S>(&self, source: S) -> GroupBufReader<S> {
        GroupBufReader::new(source, self.clone())
    }

    fn take(&self, id: u16) -> ReadBuffer {
        let buffer = self.inner.borrow_mut().take(id);
        ReadBuffer {
            id,
            inner: buffer.unwrap(),
        }
    }

    async fn read(&self, source: Source) -> Result<(ReadBuffer, usize)> {
        let (id, read) = {
            let fut = op::ReadGroup::new(source, &self.inner.borrow().id).run_on(GlobalReactor);

            fut.await?
        };

        Ok((self.take(id), read))
    }

    async fn release(&self, buffer: ReadBuffer) {
        let ReadBuffer { id, inner } = buffer;

        let (buffer, res) = unsafe {
            let fut =
                op::ProvideBuffer::new(&self.inner.borrow().id, inner, id).run_on(GlobalReactor);

            fut.await
        };

        if let Ok(id) = res {
            self.inner.borrow_mut().put(id, buffer);
        }
    }
}

struct ReadBuffer {
    id: u16,
    inner: Box<[u8]>,
}

impl AsRef<[u8]> for ReadBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.inner
    }
}

struct ReadBuffersInner {
    id: ManuallyDrop<BufferGroupId>,
    buffers: Box<[Option<Box<[u8]>>]>,
}

impl ReadBuffersInner {
    async fn new(capacity: usize, count: u16) -> Result<Self> {
        let mut buffers = Box::new_uninit_slice(count as usize);

        let group_id = GlobalReactor.get_buffer_group()?;

        // TODO: - batch these ops to not overrun the ring
        //       - proper error handling
        let mut futures = (0..count)
            .map(|buffer_id| unsafe {
                op::ProvideBuffer::new(&group_id, vec![0; capacity].into_boxed_slice(), buffer_id)
                    .run_on(GlobalReactor)
            })
            .collect::<FuturesOrdered<_>>();

        while let Some((buffer, res)) = futures.next().await {
            let id = res.expect("can this ever fail?");
            buffers[id as usize].write(Some(buffer));
        }

        Ok(Self {
            id: ManuallyDrop::new(group_id),
            buffers: unsafe { buffers.assume_init() },
        })
    }

    fn take(&mut self, id: u16) -> Option<Box<[u8]>> {
        self.buffers[id as usize].take()
    }

    fn put(&mut self, id: u16, buffer: Box<[u8]>) {
        let old = self.buffers[id as usize].replace(buffer);
        debug_assert!(old.is_none());
    }
}

impl Drop for ReadBuffersInner {
    fn drop(&mut self) {
        let id = unsafe { ManuallyDrop::take(&mut self.id) };
        let count = self.buffers.len();
        crate::spawn(async move {
            let _ = op::RemoveBuffers::new(&id, count as u16)
                .run_on(GlobalReactor)
                .await;
            GlobalReactor.release_buffer_group(id);
        });
    }
}

struct GroupBuffer(Option<ReadBuffer>);
type GroupFuture = Pin<Box<dyn Future<Output = (GroupBuffer, Result<usize>)>>>;

impl AsRef<[u8]> for GroupBuffer {
    fn as_ref(&self) -> &[u8] {
        match &self.0 {
            Some(buffer) => buffer.as_ref(),
            None => &[],
        }
    }
}

struct GroupAdapter(ReadBuffers);

impl<S: ReadSource> BufReaderAdapter<S, GroupBuffer, GroupFuture> for GroupAdapter {
    fn create_future(&self, source: &mut S, buffer: GroupBuffer) -> GroupFuture {
        let group = self.0.clone();
        let source = source.read_source();

        let fut = async move {
            if let Some(buffer) = buffer.0 {
                group.release(buffer).await;
            }

            match group.read(source).await {
                Ok((buf, read)) => (GroupBuffer(Some(buf)), Ok(read)),
                Err(err) => (GroupBuffer(None), Err(err)),
            }
        };

        Box::pin(fut)
    }
}

pub struct GroupBufReader<S>(BufReaderGeneric<S, GroupBuffer, GroupFuture, GroupAdapter>);

impl<S> GroupBufReader<S> {
    pub fn new(source: S, group: ReadBuffers) -> Self {
        Self(BufReaderGeneric::empty(
            GroupBuffer(None),
            source,
            GroupAdapter(group),
        ))
    }

    pub fn inner(&self) -> &S {
        self.0.inner()
    }

    pub fn inner_mut(&mut self) -> &mut S {
        self.0.inner_mut()
    }
}

impl<S> AsyncRead for GroupBufReader<S>
where
    S: ReadSource + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        Pin::new(&mut Pin::into_inner(self).0).poll_read(cx, buf)
    }
}

impl<S> AsyncBufRead for GroupBufReader<S>
where
    S: ReadSource + Unpin,
{
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<&[u8]>> {
        Pin::new(&mut Pin::into_inner(self).0).poll_fill_buf(cx)
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        self.0.consume_unpin(amt);
    }
}
