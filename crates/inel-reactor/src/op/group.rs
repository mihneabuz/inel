use std::{io::Result, mem::ManuallyDrop};

use io_uring::{
    opcode,
    squeue::{self, Entry},
};

use crate::{
    buffer::StableBufferMut,
    op::{util, Op},
    ring::{BufferGroupId, RingResult},
    AsSource, Source,
};

pub struct ProvideBuffer<B> {
    buffer: ManuallyDrop<B>,
    buffer_id: u16,
    group_id: u16,
}

impl<B> ProvideBuffer<B> {
    /// # Safety
    /// Caller must not drop the buffer until it is removed from the group
    pub unsafe fn new(group: &BufferGroupId, buffer: B, id: u16) -> Self {
        Self {
            buffer: ManuallyDrop::new(buffer),
            buffer_id: id,
            group_id: group.index(),
        }
    }
}

unsafe impl<B> Op for ProvideBuffer<B>
where
    B: StableBufferMut,
{
    type Output = (B, Result<u16>);

    fn entry(&mut self) -> Entry {
        opcode::ProvideBuffers::new(
            self.buffer.stable_mut_ptr(),
            self.buffer.size() as i32,
            1,
            self.group_id,
            self.buffer_id,
        )
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        (
            ManuallyDrop::into_inner(self.buffer),
            util::expect_zero(&res).map(|_| self.buffer_id),
        )
    }
}

pub struct RemoveBuffers {
    group_id: u16,
    count: u16,
}

impl RemoveBuffers {
    pub fn new(group: &BufferGroupId, count: u16) -> Self {
        Self {
            group_id: group.index(),
            count,
        }
    }
}

unsafe impl Op for RemoveBuffers {
    type Output = Result<usize>;

    fn entry(&mut self) -> Entry {
        opcode::RemoveBuffers::new(self.count, self.group_id).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_positive(&res)
    }
}

pub struct ReadGroup {
    source: Source,
    group_id: u16,
}

impl ReadGroup {
    pub fn new(source: impl AsSource, group: &BufferGroupId) -> Self {
        Self {
            source: source.as_source(),
            group_id: group.index(),
        }
    }
}

unsafe impl Op for ReadGroup {
    type Output = Result<(u16, usize)>;

    fn entry(&mut self) -> Entry {
        opcode::Read::new(self.source.as_raw(), std::ptr::null_mut(), 0)
            .buf_group(self.group_id)
            .offset(u64::MAX)
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_positive(&res).map(|read| (res.buffer_id().unwrap(), read))
    }
}
