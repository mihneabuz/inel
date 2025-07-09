use std::{
    future::Future,
    io::Result,
    mem::ManuallyDrop,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use futures::{stream::FuturesUnordered, AsyncBufRead, AsyncBufReadExt, AsyncRead, StreamExt};

use crate::{
    io::{
        buffered::reader::generic::{BufReaderAdapter, BufReaderGeneric},
        ReadSource,
    },
    GlobalReactor,
};

use inel_reactor::{
    group::ReadBufferGroup,
    op::{DetachOp, OpExt},
    AsSource,
};

#[derive(Clone)]
pub struct ReadBuffers {
    inner: Rc<ReadBuffersInner>,
}

impl ReadBuffers {
    pub fn empty() -> Result<Self> {
        ReadBuffersInner::new().map(|inner| Self {
            inner: Rc::new(inner),
        })
    }

    pub async fn new(count: u16, capacity: usize) -> Result<Self> {
        let this = Self::empty()?;

        let mut provides = (0..count)
            .map(|_| {
                this.inner
                    .group()
                    .provide(vec![0; capacity].into_boxed_slice())
                    .run_on(GlobalReactor)
            })
            .collect::<FuturesUnordered<_>>();

        while let Some(provide) = provides.next().await {
            debug_assert!(provide.is_ok());
        }

        std::mem::drop(provides);

        Ok(this)
    }

    async fn read(&self, source: impl AsSource) -> (Option<Box<[u8]>>, Result<usize>) {
        self.inner.group().read(source).run_on(GlobalReactor).await
    }

    pub fn provide_to<S>(&self, source: S) -> GroupBufReader<S> {
        GroupBufReader::new(source, self.clone())
    }
}

struct ReadBuffersInner {
    group: ManuallyDrop<ReadBufferGroup<GlobalReactor>>,
}

impl ReadBuffersInner {
    fn new() -> Result<Self> {
        ReadBufferGroup::new(GlobalReactor).map(|group| Self {
            group: ManuallyDrop::new(group),
        })
    }

    fn group(&self) -> &ReadBufferGroup<GlobalReactor> {
        &self.group
    }
}

impl Drop for ReadBuffersInner {
    fn drop(&mut self) {
        let group = unsafe { ManuallyDrop::take(&mut self.group) };
        crate::spawn(group.release().run_on(GlobalReactor));
    }
}

struct GroupBuffer(Option<Box<[u8]>>);
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
        if let Some(buffer) = buffer.0 {
            self.0
                .inner
                .group()
                .provide(buffer)
                .run_detached(&mut GlobalReactor);
        }

        let group = self.0.clone();
        let source = source.read_source();

        let fut = async move {
            let (buf, res) = group.read(source).await;
            (GroupBuffer(buf), res)
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
