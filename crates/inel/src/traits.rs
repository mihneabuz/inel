use std::io::Result;
use std::os::fd::AsRawFd;

use crate::{GlobalReactor, StableBuffer, StableMutBuffer};
use inel_reactor::op::{self, Op};

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

#[allow(async_fn_in_trait)]
pub trait InelRead {
    async fn read_owned<B: StableMutBuffer>(&mut self, buf: B) -> (B, Result<usize>);
}

#[allow(async_fn_in_trait)]
pub trait InelWrite {
    async fn write_owned<B: StableBuffer>(&mut self, buf: B) -> (B, Result<usize>);
}

impl<T> InelRead for T
where
    T: ReadSource,
{
    async fn read_owned<B: StableMutBuffer>(&mut self, buf: B) -> (B, Result<usize>) {
        op::Read::new(self.as_raw_fd(), buf)
            .run_on(GlobalReactor)
            .await
    }
}

impl<T> InelWrite for T
where
    T: WriteSource,
{
    async fn write_owned<B: StableBuffer>(&mut self, buf: B) -> (B, Result<usize>) {
        op::Write::new(self.as_raw_fd(), buf)
            .run_on(GlobalReactor)
            .await
    }
}
