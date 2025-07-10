use std::{io::Result, mem::ManuallyDrop};

use inel_interface::Reactor;
use io_uring::{
    opcode,
    squeue::{self, Entry},
};

use crate::{
    buffer::{StableBuffer, StableBufferMut},
    group::ReadBufferGroup,
    op::{util, DetachOp, MultiOp, Op},
    ring::{Ring, RingResult},
    source::{AsSource, Source},
};

pub struct ProvideBuffer<'a, R> {
    group: &'a ReadBufferGroup<R>,
    buffer: ManuallyDrop<Box<[u8]>>,
}

impl<'a, R> ProvideBuffer<'a, R> {
    pub fn new(group: &'a ReadBufferGroup<R>, buffer: Box<[u8]>) -> Self {
        Self {
            group,
            buffer: ManuallyDrop::new(buffer),
        }
    }
}

unsafe impl<R> Op for ProvideBuffer<'_, R> {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let ptr = self.buffer.stable_mut_ptr();
        let size = self.buffer.size() as i32;

        let buffer = unsafe { ManuallyDrop::take(&mut self.buffer) };
        let id = self.group.put(buffer);

        opcode::ProvideBuffers::new(ptr, size, 1, self.group.id().index(), id).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

impl<R> DetachOp for ProvideBuffer<'_, R> {}

pub struct ReleaseGroup<R> {
    group: ReadBufferGroup<R>,
}

impl<R> ReleaseGroup<R> {
    pub fn new(group: ReadBufferGroup<R>) -> Self {
        Self { group }
    }
}

unsafe impl<R> Op for ReleaseGroup<R>
where
    R: Reactor<Handle = Ring>,
{
    type Output = ();

    fn entry(&mut self) -> Entry {
        opcode::RemoveBuffers::new(self.group.present() as u16, self.group.id().index()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        let removed = util::expect_positive(&res).unwrap();
        assert_eq!(removed, self.group.present());
        unsafe { self.group.release_id() };
    }
}

pub struct ReadGroup<'a, R> {
    source: Source,
    group: &'a ReadBufferGroup<R>,
}

impl<'a, R> ReadGroup<'a, R> {
    pub fn new(source: impl AsSource, group: &'a ReadBufferGroup<R>) -> Self {
        Self {
            source: source.as_source(),
            group,
        }
    }
}

unsafe impl<R> Op for ReadGroup<'_, R> {
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Read::new(self.source.as_raw(), std::ptr::null_mut(), 0)
            .buf_group(self.group.id().index())
            .offset(u64::MAX)
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
    }

    fn result(self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct ReadGroupMulti<'a, R> {
    source: Source,
    group: &'a ReadBufferGroup<R>,
}

impl<'a, R> ReadGroupMulti<'a, R> {
    pub fn new(source: impl AsSource, group: &'a ReadBufferGroup<R>) -> Self {
        Self {
            source: source.as_source(),
            group,
        }
    }
}

unsafe impl<R> Op for ReadGroupMulti<'_, R> {
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::ReadMulti::new(self.source.as_raw(), self.group.id().index())
            .offset(u64::MAX)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }
}

impl<R> MultiOp for ReadGroupMulti<'_, R> {
    fn next(&self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }
}
