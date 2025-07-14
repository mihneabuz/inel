use std::{io::Result, marker::PhantomData, mem::ManuallyDrop, ops::Deref};

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

pub struct ProvideBuffer<G, R> {
    group: G,
    buffer: ManuallyDrop<Box<[u8]>>,
    _phantom: PhantomData<R>,
}

impl<G, R> ProvideBuffer<G, R> {
    pub fn new(group: G, buffer: Box<[u8]>) -> Self {
        Self {
            group,
            buffer: ManuallyDrop::new(buffer),
            _phantom: PhantomData,
        }
    }
}

unsafe impl<G, R> Op for ProvideBuffer<G, R>
where
    G: Deref<Target = ReadBufferGroup<R>>,
{
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

impl<G, R> DetachOp for ProvideBuffer<G, R> where G: Deref<Target = ReadBufferGroup<R>> {}

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
        let present = self.group.present();
        if present > 0 {
            let removed = util::expect_positive(&res).unwrap();
            assert_eq!(removed, present);
        }
        unsafe { self.group.release_id() };
    }
}

pub struct ReadGroup<G, R> {
    source: Source,
    group: G,
    _phantom: PhantomData<R>,
}

impl<G, R> ReadGroup<G, R> {
    pub fn new(source: impl AsSource, group: G) -> Self {
        Self {
            source: source.as_source(),
            group,
            _phantom: PhantomData,
        }
    }
}

unsafe impl<G, R> Op for ReadGroup<G, R>
where
    G: Deref<Target = ReadBufferGroup<R>>,
{
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Read::new(self.source.as_raw(), std::ptr::null_mut(), 0)
            .buf_group(self.group.deref().id().index())
            .offset(u64::MAX)
            .build()
            .flags(squeue::Flags::BUFFER_SELECT)
    }

    fn result(self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.deref().take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }
}

pub struct ReadGroupMulti<G, R> {
    source: Source,
    group: G,
    _phantom: PhantomData<R>,
}

impl<G, R> ReadGroupMulti<G, R> {
    pub fn new(source: impl AsSource, group: G) -> Self {
        Self {
            source: source.as_source(),
            group,
            _phantom: PhantomData,
        }
    }
}

unsafe impl<G, R> Op for ReadGroupMulti<G, R>
where
    G: Deref<Target = ReadBufferGroup<R>>,
{
    type Output = (Option<Box<[u8]>>, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::ReadMulti::new(self.source.as_raw(), self.group.id().index()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }
}

impl<G, R> MultiOp for ReadGroupMulti<G, R>
where
    G: Deref<Target = ReadBufferGroup<R>>,
{
    fn next(&self, res: RingResult) -> Self::Output {
        let buffer = res.buffer_id().map(|id| self.group.take(id));
        let read = util::expect_positive(&res);
        (buffer, read)
    }
}
