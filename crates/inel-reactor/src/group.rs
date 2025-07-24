use std::{cell::RefCell, io::Result};

use slab::Slab;

use inel_interface::Reactor;

use crate::{
    ring::{BufferGroupId, Ring},
    RingReactor,
};

pub trait AsBufferGroup {
    fn as_group(&self) -> &ReadBufferGroup;
}

impl AsBufferGroup for ReadBufferGroup {
    fn as_group(&self) -> &ReadBufferGroup {
        self
    }
}

pub struct ReadBufferGroup {
    id: BufferGroupId,
    storage: RefCell<BufferStorage>,
}

struct BufferStorage {
    buffers: Slab<Box<[u8]>>,
    cancelled: Vec<Box<[u8]>>,
}

impl ReadBufferGroup {
    pub fn register<R>(reactor: &mut R) -> Result<Self>
    where
        R: Reactor<Handle = Ring>,
    {
        let id = reactor.get_buffer_group()?;
        let storage = RefCell::new(BufferStorage {
            buffers: Slab::with_capacity(128),
            cancelled: Vec::with_capacity(16),
        });

        Ok(Self { id, storage })
    }

    pub fn release<R>(self, reactor: &mut R)
    where
        R: Reactor<Handle = Ring>,
    {
        reactor.release_buffer_group(self.id);
    }
}

impl ReadBufferGroup {
    pub(crate) fn id(&self) -> &BufferGroupId {
        &self.id
    }

    pub(crate) fn take(&self, id: u16) -> Box<[u8]> {
        self.storage.borrow_mut().buffers.remove(id as usize)
    }

    pub(crate) fn put(&self, buffer: Box<[u8]>) -> u16 {
        self.storage.borrow_mut().buffers.insert(buffer) as u16
    }

    pub(crate) fn clear(&self) {
        self.storage.borrow_mut().buffers.clear();
    }

    pub fn present(&self) -> usize {
        self.storage.borrow().buffers.len()
    }

    pub(crate) fn mark_cancelled(&self, id: u16) {
        let mut guard = self.storage.borrow_mut();
        let buffer = guard.buffers.remove(id as usize);
        guard.cancelled.push(buffer)
    }

    pub fn get_cancelled(&self) -> Option<Box<[u8]>> {
        self.storage.borrow_mut().cancelled.pop()
    }
}
