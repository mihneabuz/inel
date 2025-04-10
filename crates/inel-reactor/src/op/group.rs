use std::io::Result;

use io_uring::{
    opcode,
    squeue::{self, Entry},
};

use crate::{
    buffer::StableBuffer,
    op::{util, Op},
    ring::{BufferGroupKey, RingResult},
    AsSource, Cancellation, Source,
};

pub struct ProvideBuffer<B> {
    group: BufferGroupKey,
    buffer: B,
    id: u16,
}

impl<B> ProvideBuffer<B> {
    pub fn new(group: BufferGroupKey, buffer: B, id: u16) -> Self {
        Self { buffer, group, id }
    }
}

unsafe impl<B> Op for ProvideBuffer<B>
where
    B: StableBuffer,
{
    type Output = Result<(B, u16)>;

    fn entry(&mut self) -> Entry {
        opcode::ProvideBuffers::new(
            self.buffer.stable_mut_ptr(),
            self.buffer.size() as i32,
            1,
            self.group.index(),
            self.id,
        )
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res).map(|_| (self.buffer, self.id))
    }

    fn cancel(self) -> Cancellation {
        self.buffer.into()
    }
}

pub struct RemoveBuffers {
    group: BufferGroupKey,
    count: u16,
}

impl RemoveBuffers {
    pub fn new(group: BufferGroupKey, count: u16) -> Self {
        Self { group, count }
    }
}

unsafe impl Op for RemoveBuffers {
    type Output = Result<usize>;

    fn entry(&mut self) -> Entry {
        opcode::RemoveBuffers::new(self.count, self.group.index()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_positive(&res)
    }
}

pub struct ReadGroup {
    source: Source,
    group: BufferGroupKey,
}

impl ReadGroup {
    pub fn new(source: impl AsSource, group: BufferGroupKey) -> Self {
        Self {
            source: source.as_source(),
            group,
        }
    }
}

unsafe impl Op for ReadGroup {
    type Output = Result<(u16, usize)>;

    fn entry(&mut self) -> Entry {
        opcode::Read::new(self.source.as_raw(), std::ptr::null_mut(), 0)
            .buf_group(self.group.index())
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_positive(&res).map(|read| (res.buffer_id().unwrap(), read))
    }
}
