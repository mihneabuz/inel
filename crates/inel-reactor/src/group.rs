use std::{cell::RefCell, io::Result};

use inel_interface::Reactor;

use slab::Slab;

use crate::{
    op::{ProvideBuffer, ReadGroup, ReleaseGroup},
    ring::{BufferGroupId, Ring},
    source::AsSource,
    RingReactor,
};

pub struct ReadBufferGroup<R> {
    id: BufferGroupId,
    buffers: RefCell<Slab<Box<[u8]>>>,
    reactor: R,
}

impl<R> ReadBufferGroup<R>
where
    R: Reactor<Handle = Ring>,
{
    pub fn new(mut reactor: R) -> Result<Self> {
        let id = reactor.get_buffer_group()?;
        let buffers = RefCell::new(Slab::with_capacity(128));

        Ok(Self {
            id,
            buffers,
            reactor,
        })
    }

    pub(crate) unsafe fn release_id(mut self) {
        self.reactor.release_buffer_group(self.id);
    }
}

impl<R> ReadBufferGroup<R> {
    pub fn provide(&self, buffer: Box<[u8]>) -> ProvideBuffer<R> {
        ProvideBuffer::new(self, buffer)
    }

    pub fn read(&self, source: impl AsSource) -> ReadGroup<R> {
        ReadGroup::new(source, self)
    }

    pub fn release(self) -> ReleaseGroup<R> {
        ReleaseGroup::new(self)
    }

    pub(crate) fn id(&self) -> &BufferGroupId {
        &self.id
    }

    pub(crate) fn take(&self, id: u16) -> Box<[u8]> {
        self.buffers.borrow_mut().remove(id as usize)
    }

    pub(crate) fn put(&self, buffer: Box<[u8]>) -> u16 {
        self.buffers.borrow_mut().insert(buffer) as u16
    }

    pub(crate) fn present(&self) -> usize {
        self.buffers.borrow().len()
    }
}
