use std::{io::Result, mem::ManuallyDrop, rc::Rc};

use io_uring::{
    opcode,
    squeue::{self, Entry},
};

use crate::{
    buffer::{StableBuffer, StableBufferMut},
    cancellation::{consuming, Cancellation},
    group::{AsBufferGroup, ReadBufferGroup},
    op::{util, DetachOp, MultiOp, Op},
    ring::RingResult,
    source::{AsSource, Source},
};

pub struct ProvideBuffer<G> {
    group: Rc<G>,
    buffer: ManuallyDrop<Box<[u8]>>,
}

impl<G> ProvideBuffer<G> {
    pub fn new(group: &Rc<G>, buffer: Box<[u8]>) -> Self {
        Self {
            group: group.clone(),
            buffer: ManuallyDrop::new(buffer),
        }
    }
}

unsafe impl<G> Op for ProvideBuffer<G>
where
    G: AsBufferGroup,
{
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let ptr = self.buffer.stable_mut_ptr();
        let size = self.buffer.size() as i32;

        let buffer = unsafe { ManuallyDrop::take(&mut self.buffer) };
        let id = self.group.as_group().put(buffer);

        opcode::ProvideBuffers::new(ptr, size, 1, self.group.as_group().id().index(), id).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

impl<G> DetachOp for ProvideBuffer<G> where G: AsBufferGroup {}

pub struct RemoveBuffers {
    group: ReadBufferGroup,
}

impl RemoveBuffers {
    pub fn new(group: ReadBufferGroup) -> Self {
        Self { group }
    }
}

unsafe impl Op for RemoveBuffers {
    type Output = ReadBufferGroup;

    fn entry(&mut self) -> Entry {
        opcode::RemoveBuffers::new(
            self.group.as_group().present() as u16,
            self.group.as_group().id().index(),
        )
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        let present = self.group.as_group().present();
        if present > 0 {
            let removed = util::expect_positive(&res).unwrap();
            assert_eq!(removed, present);
            self.group.clear();
        }
        self.group
    }
}

pub struct ReadGroup<G> {
    source: Source,
    group: Rc<G>,
}

impl<G> ReadGroup<G> {
    pub fn new(source: impl AsSource, group: &Rc<G>) -> Self {
        Self {
            source: source.as_source(),
            group: group.clone(),
        }
    }
}

unsafe impl<G> Op for ReadGroup<G>
where
    G: AsBufferGroup,
{
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Read::new(self.source.as_raw(), std::ptr::null_mut(), 0)
            .buf_group(self.group.as_group().id().index())
            .offset(u64::MAX)
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
    }

    fn result(self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.as_group().take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }

    fn cancel(self) -> Cancellation {
        consuming!(G, self.group, |group, result| {
            if let Some(id) = result.buffer_id() {
                group.as_group().mark_cancelled(id);
            }
        })
    }
}

pub struct ReadGroupMulti<G> {
    source: Source,
    group: Rc<G>,
}

impl<G> ReadGroupMulti<G> {
    pub fn new(source: impl AsSource, group: &Rc<G>) -> Self {
        Self {
            source: source.as_source(),
            group: group.clone(),
        }
    }
}

unsafe impl<G> Op for ReadGroupMulti<G>
where
    G: AsBufferGroup,
{
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::ReadMulti::new(self.source.as_raw(), self.group.as_group().id().index()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }

    fn cancel(self) -> Cancellation {
        consuming!(G, self.group, |group, result| {
            if let Some(id) = result.buffer_id() {
                group.as_group().mark_cancelled(id);
            }
        })
    }
}

impl<G> MultiOp for ReadGroupMulti<G>
where
    G: AsBufferGroup,
{
    fn next(&self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.as_group().take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }
}
