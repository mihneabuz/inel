use std::io::Result;
use std::os::fd::AsRawFd;

use crate::{GlobalReactor, StableBuffer};
use inel_reactor::op::{self, Op};

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

#[allow(async_fn_in_trait)]
pub trait InelRead {
    async fn read_owned<B: StableBuffer>(&mut self, buffer: B) -> (B, Result<usize>);
    async fn read_owned_at<B: StableBuffer>(
        &mut self,
        offset: u64,
        buffer: B,
    ) -> (B, Result<usize>);
}

#[allow(async_fn_in_trait)]
pub trait InelWrite {
    async fn write_owned<B: StableBuffer>(&mut self, buffer: B) -> (B, Result<usize>);
    async fn write_owned_at<B: StableBuffer>(
        &mut self,
        offset: u64,
        buffer: B,
    ) -> (B, Result<usize>);
}

impl<T> InelRead for T
where
    T: ReadSource,
{
    async fn read_owned<B: StableBuffer>(&mut self, buffer: B) -> (B, Result<usize>) {
        op::Read::new(self.as_raw_fd(), buffer)
            .run_on(GlobalReactor)
            .await
    }

    async fn read_owned_at<B: StableBuffer>(
        &mut self,
        offset: u64,
        buffer: B,
    ) -> (B, Result<usize>) {
        op::Read::new(self.as_raw_fd(), buffer)
            .offset(offset)
            .run_on(GlobalReactor)
            .await
    }
}

impl<T> InelWrite for T
where
    T: WriteSource,
{
    async fn write_owned<B: StableBuffer>(&mut self, buffer: B) -> (B, Result<usize>) {
        op::Write::new(self.as_raw_fd(), buffer)
            .run_on(GlobalReactor)
            .await
    }

    async fn write_owned_at<B: StableBuffer>(
        &mut self,
        offset: u64,
        buffer: B,
    ) -> (B, Result<usize>) {
        op::Write::new(self.as_raw_fd(), buffer)
            .offset(offset)
            .run_on(GlobalReactor)
            .await
    }
}
