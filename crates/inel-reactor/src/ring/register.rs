use std::marker::PhantomData;

use io_uring::types::DestinationSlot;

pub struct SlotRegister<T> {
    vacant: Vec<u32>,
    len: u32,
    size: u32,
    _phantom: PhantomData<T>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SlotKey {
    index: u32,
    _marker: PhantomData<*const ()>,
}

#[derive(Debug, PartialEq)]
pub struct BufferSlot(SlotKey);

#[derive(Debug, PartialEq)]
pub struct BufferGroupId(SlotKey);

#[derive(Debug, PartialEq)]
pub struct DirectSlot(SlotKey);

pub(crate) trait WrapSlotKey {
    fn wrap(key: SlotKey) -> Self;
    fn unwrap(self) -> SlotKey;
}

impl WrapSlotKey for DirectSlot {
    fn wrap(key: SlotKey) -> Self {
        DirectSlot(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl WrapSlotKey for BufferSlot {
    fn wrap(key: SlotKey) -> Self {
        BufferSlot(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl WrapSlotKey for BufferGroupId {
    fn wrap(key: SlotKey) -> Self {
        BufferGroupId(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl<T: WrapSlotKey> SlotRegister<T> {
    pub fn new(size: u32) -> Self {
        Self {
            vacant: Vec::new(),
            len: 0,
            size,
            _phantom: PhantomData,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn get(&mut self) -> Option<T> {
        if let Some(slot) = self.vacant.pop() {
            return Some(T::wrap(SlotKey::new(slot)));
        };

        if self.len == self.size {
            return None;
        }

        let index = self.len;
        self.len += 1;

        Some(T::wrap(SlotKey::new(index)))
    }

    pub fn remove(&mut self, key: T) {
        let key = key.unwrap();
        if key.index() < self.size() {
            self.vacant.push(key.index());
        }
    }

    pub fn is_full(&self) -> bool {
        self.len as usize == self.vacant.len()
    }
}

impl SlotKey {
    pub fn new(index: u32) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }

    pub fn index(&self) -> u32 {
        self.index
    }
}

impl DirectSlot {
    pub fn index(&self) -> u32 {
        self.0.index()
    }

    pub(crate) fn from_raw_slot(slot: u32) -> Self {
        Self::wrap(SlotKey::new(slot))
    }

    pub(crate) fn as_destination_slot(&self) -> DestinationSlot {
        DestinationSlot::try_from_slot_target(self.index()).unwrap()
    }
}

impl BufferSlot {
    pub fn index(&self) -> u32 {
        self.0.index()
    }
}

impl BufferGroupId {
    pub fn index(&self) -> u16 {
        self.0.index() as u16
    }
}
