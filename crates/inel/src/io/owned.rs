use std::{
    future::Future,
    io::Result,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::FusedFuture, FutureExt};
use inel_reactor::{
    buffer::{FixedBuffer, StableBuffer, StableBufferMut},
    op::{self, OpExt},
    Submission,
};

use crate::{
    io::{ReadSource, WriteSource},
    GlobalReactor,
};

pub trait AsyncReadOwned {
    fn read_owned<B: StableBufferMut>(&mut self, buffer: B) -> ReadOwned<B>;
    fn read_owned_at<B: StableBufferMut>(&mut self, offset: u64, buffer: B) -> ReadOwned<B>;

    fn read_fixed<B>(&mut self, buffer: B) -> ReadFixed<B>
    where
        B: FixedBuffer + StableBufferMut;
    fn read_fixed_at<B>(&mut self, offset: u64, buffer: B) -> ReadFixed<B>
    where
        B: FixedBuffer + StableBufferMut;
}

pub trait AsyncWriteOwned {
    fn write_owned<B: StableBuffer>(&mut self, buffer: B) -> WriteOwned<B>;
    fn write_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> WriteOwned<B>;

    fn write_fixed<B>(&mut self, buffer: B) -> WriteFixed<B>
    where
        B: FixedBuffer + StableBuffer;
    fn write_fixed_at<B>(&mut self, offset: u64, buffer: B) -> WriteFixed<B>
    where
        B: FixedBuffer + StableBuffer;
}

impl<T> AsyncReadOwned for T
where
    T: ReadSource,
{
    fn read_owned<B: StableBufferMut>(&mut self, buffer: B) -> ReadOwned<B> {
        ReadOwned {
            sub: op::Read::new(self.read_source(), buffer).run_on(GlobalReactor),
        }
    }

    fn read_owned_at<B: StableBufferMut>(&mut self, offset: u64, buffer: B) -> ReadOwned<B> {
        ReadOwned {
            sub: op::Read::new(self.read_source(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }

    fn read_fixed<B>(&mut self, buffer: B) -> ReadFixed<B>
    where
        B: FixedBuffer + StableBufferMut,
    {
        ReadFixed {
            sub: op::ReadFixed::new(self.read_source(), buffer).run_on(GlobalReactor),
        }
    }

    fn read_fixed_at<B>(&mut self, offset: u64, buffer: B) -> ReadFixed<B>
    where
        B: FixedBuffer + StableBufferMut,
    {
        ReadFixed {
            sub: op::ReadFixed::new(self.read_source(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }
}

impl<T> AsyncWriteOwned for T
where
    T: WriteSource,
{
    fn write_owned<B: StableBuffer>(&mut self, buffer: B) -> WriteOwned<B> {
        WriteOwned {
            sub: op::Write::new(self.write_source(), buffer).run_on(GlobalReactor),
        }
    }

    fn write_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> WriteOwned<B> {
        WriteOwned {
            sub: op::Write::new(self.write_source(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }

    fn write_fixed<B>(&mut self, buffer: B) -> WriteFixed<B>
    where
        B: FixedBuffer + StableBuffer,
    {
        WriteFixed {
            sub: op::WriteFixed::new(self.write_source(), buffer).run_on(GlobalReactor),
        }
    }

    fn write_fixed_at<B>(&mut self, offset: u64, buffer: B) -> WriteFixed<B>
    where
        B: FixedBuffer + StableBuffer,
    {
        WriteFixed {
            sub: op::WriteFixed::new(self.write_source(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }
}

pub struct ReadOwned<B: StableBufferMut> {
    sub: Submission<op::Read<B>, GlobalReactor>,
}

impl<B: StableBufferMut> Future for ReadOwned<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl<B: StableBufferMut> FusedFuture for ReadOwned<B> {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}

pub struct WriteOwned<B: StableBuffer> {
    sub: Submission<op::Write<B>, GlobalReactor>,
}

impl<B: StableBuffer> Future for WriteOwned<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl<B: StableBuffer> FusedFuture for WriteOwned<B> {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}

pub struct ReadFixed<B: FixedBuffer + StableBufferMut> {
    sub: Submission<op::ReadFixed<B>, GlobalReactor>,
}

impl<B: FixedBuffer + StableBufferMut> Future for ReadFixed<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl<B: FixedBuffer + StableBufferMut> FusedFuture for ReadFixed<B> {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}

pub struct WriteFixed<B: FixedBuffer + StableBuffer> {
    sub: Submission<op::WriteFixed<B>, GlobalReactor>,
}

impl<B: FixedBuffer + StableBuffer> Future for WriteFixed<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl<B: FixedBuffer + StableBuffer> FusedFuture for WriteFixed<B> {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}
