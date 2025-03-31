use std::marker::PhantomData;

use io_uring::types::DestinationSlot;

pub struct SlotRegister {
    vacant: Vec<u32>,
    len: u32,
    size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SlotKey {
    index: u32,
    _marker: PhantomData<*const ()>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferSlotKey(SlotKey);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BufferGroupKey(SlotKey);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileSlotKey(SlotKey);

pub(crate) trait WrapSlotKey {
    fn wrap(key: SlotKey) -> Self;
    fn unwrap(self) -> SlotKey;
}

impl WrapSlotKey for FileSlotKey {
    fn wrap(key: SlotKey) -> Self {
        FileSlotKey(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl WrapSlotKey for BufferSlotKey {
    fn wrap(key: SlotKey) -> Self {
        BufferSlotKey(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl WrapSlotKey for BufferGroupKey {
    fn wrap(key: SlotKey) -> Self {
        BufferGroupKey(key)
    }

    fn unwrap(self) -> SlotKey {
        self.0
    }
}

impl SlotRegister {
    pub fn new(size: u32) -> Self {
        Self {
            vacant: Vec::new(),
            len: 0,
            size,
        }
    }

    pub fn get(&mut self) -> Option<SlotKey> {
        if let Some(slot) = self.vacant.pop() {
            return Some(SlotKey::new(slot));
        };

        if self.len == self.size {
            return None;
        }

        let index = self.len;
        self.len += 1;

        Some(SlotKey::new(index))
    }

    pub fn remove(&mut self, key: SlotKey) {
        self.vacant.push(key.index());
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

impl FileSlotKey {
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

impl BufferSlotKey {
    pub fn index(&self) -> u32 {
        self.0.index()
    }
}

impl BufferGroupKey {
    pub fn index(&self) -> u16 {
        self.0.index() as u16
    }
}
