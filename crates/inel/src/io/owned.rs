use std::{
    future::Future,
    io::Result,
    pin::Pin,
    task::{Context, Poll},
};

use futures::FutureExt;
use inel_reactor::{
    buffer::{FixedBuffer, StableBuffer},
    op::{self, Op},
    Submission,
};

use crate::{
    io::{ReadSource, WriteSource},
    GlobalReactor,
};

pub trait AsyncReadOwned {
    fn read_owned<B: StableBuffer>(&mut self, buffer: B) -> ReadOwned<B>;
    fn read_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> ReadOwned<B>;
}

pub trait AsyncWriteOwned {
    fn write_owned<B: StableBuffer>(&mut self, buffer: B) -> WriteOwned<B>;
    fn write_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> WriteOwned<B>;
}

pub trait AsyncReadFixed {
    fn read_fixed<B: FixedBuffer>(&mut self, buffer: B) -> ReadFixed<B>;
    fn read_fixed_at<B: FixedBuffer>(&mut self, offset: u64, buffer: B) -> ReadFixed<B>;
}

pub trait AsyncWriteFixed {
    fn write_fixed<B: FixedBuffer>(&mut self, buffer: B) -> WriteFixed<B>;
    fn write_fixed_at<B: FixedBuffer>(&mut self, offset: u64, buffer: B) -> WriteFixed<B>;
}

impl<T> AsyncReadOwned for T
where
    T: ReadSource,
{
    fn read_owned<B: StableBuffer>(&mut self, buffer: B) -> ReadOwned<B> {
        ReadOwned {
            sub: op::Read::new(self.as_raw_fd(), buffer).run_on(GlobalReactor),
        }
    }

    fn read_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> ReadOwned<B> {
        ReadOwned {
            sub: op::Read::new(self.as_raw_fd(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }
}

impl<T> AsyncReadFixed for T
where
    T: ReadSource,
{
    fn read_fixed<B: FixedBuffer>(&mut self, buffer: B) -> ReadFixed<B> {
        ReadFixed {
            sub: op::ReadFixed::new(self.as_raw_fd(), buffer).run_on(GlobalReactor),
        }
    }

    fn read_fixed_at<B: FixedBuffer>(&mut self, offset: u64, buffer: B) -> ReadFixed<B> {
        ReadFixed {
            sub: op::ReadFixed::new(self.as_raw_fd(), buffer)
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
            sub: op::Write::new(self.as_raw_fd(), buffer).run_on(GlobalReactor),
        }
    }

    fn write_owned_at<B: StableBuffer>(&mut self, offset: u64, buffer: B) -> WriteOwned<B> {
        WriteOwned {
            sub: op::Write::new(self.as_raw_fd(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }
}

impl<T> AsyncWriteFixed for T
where
    T: WriteSource,
{
    fn write_fixed<B: FixedBuffer>(&mut self, buffer: B) -> WriteFixed<B> {
        WriteFixed {
            sub: op::WriteFixed::new(self.as_raw_fd(), buffer).run_on(GlobalReactor),
        }
    }

    fn write_fixed_at<B: FixedBuffer>(&mut self, offset: u64, buffer: B) -> WriteFixed<B> {
        WriteFixed {
            sub: op::WriteFixed::new(self.as_raw_fd(), buffer)
                .offset(offset)
                .run_on(GlobalReactor),
        }
    }
}

pub struct ReadOwned<B: StableBuffer> {
    sub: Submission<op::Read<B>, GlobalReactor>,
}

impl<B: StableBuffer> Future for ReadOwned<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
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

pub struct ReadFixed<B: FixedBuffer> {
    sub: Submission<op::ReadFixed<B>, GlobalReactor>,
}

impl<B: FixedBuffer> Future for ReadFixed<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

pub struct WriteFixed<B: FixedBuffer> {
    sub: Submission<op::WriteFixed<B>, GlobalReactor>,
}

impl<B: FixedBuffer> Future for WriteFixed<B> {
    type Output = (B, Result<usize>);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}
