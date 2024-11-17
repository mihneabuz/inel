use std::{
    io::{Error, Result},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncWrite, FutureExt};

use super::{
    generic::{BufWriterGeneric, BufWriterState},
    FixedBufWriter,
};
use crate::{
    buffer::StableBufferExt,
    io::{owned::WriteOwned, AsyncWriteOwned},
};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type VecBuf = Vec<u8>;
type WriteFut = WriteOwned<VecBuf>;

pub struct BufWriter<S>(BufWriterGeneric<S, VecBuf, WriteFut>);

impl<S> BufWriter<S>
where
    S: AsyncWriteOwned,
{
    pub fn new(sink: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, sink)
    }

    pub fn with_capacity(capacity: usize, sink: S) -> Self {
        Self(BufWriterGeneric {
            state: BufWriterState::Ready(Vec::with_capacity(capacity)),
            sink,
        })
    }

    pub fn fix(self) -> Result<FixedBufWriter<S>> {
        let (source, buf) = self.0.into_raw_parts();

        let mut vec = buf.ok_or(Error::other("BufWriter was in use"))?;
        let len = vec.len();

        unsafe { vec.set_len(vec.capacity()) };
        let mut fixed = vec.fix()?;
        unsafe { fixed.inner_mut().set_len(len) };

        Ok(FixedBufWriter::from_raw(source, fixed))
    }

    pub fn capacity(&self) -> Option<usize> {
        self.0.ready().map(|vec| vec.capacity())
    }

    pub fn buffer(&self) -> Option<&[u8]> {
        self.0.buffer()
    }

    pub fn inner(&self) -> &S {
        self.0.inner()
    }

    pub fn inner_mut(&mut self) -> &mut S {
        self.0.inner_mut()
    }

    pub fn into_inner(self) -> S {
        self.0.into_inner()
    }
}

impl<S> AsyncWrite for BufWriter<S>
where
    S: AsyncWriteOwned + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(_) => {
                let mut pinned = Pin::new(this);
                ready!(pinned.as_mut().poll_flush(cx))?;
                pinned.as_mut().poll_write(cx, buf)
            }

            BufWriterState::Ready(vec) => {
                if vec.spare_capacity_mut().len() >= buf.len() {
                    let res = std::io::Write::write(vec, buf);

                    Poll::Ready(res)
                } else {
                    let mut pinned = Pin::new(this);
                    ready!(pinned.as_mut().poll_flush(cx))?;
                    pinned.as_mut().poll_write(cx, buf)
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let this = Pin::into_inner(self);

        match &mut this.0.state {
            BufWriterState::Empty => unreachable!(),

            BufWriterState::Pending(fut) => {
                let (mut buf, res) = ready!(fut.poll_unpin(cx));
                match res {
                    Ok(wrote) => {
                        assert_eq!(buf.len(), wrote);
                        buf.clear();
                        this.0.state = BufWriterState::Ready(buf);
                    }

                    Err(err) => {
                        this.0.state = BufWriterState::Ready(buf);
                        return Poll::Ready(Err(err));
                    }
                }
            }

            BufWriterState::Ready(vec) => {
                if !vec.is_empty() {
                    let BufWriterState::Ready(vec) = std::mem::take(&mut this.0.state) else {
                        unreachable!();
                    };

                    let fut = this.0.sink.write_owned(vec);
                    this.0.state = BufWriterState::Pending(fut);

                    return Pin::new(this).poll_flush(cx);
                }
            }
        }

        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }
}
