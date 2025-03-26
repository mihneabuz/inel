use io_uring::types::DestinationSlot;

pub struct SlotRegister {
    vacant: Vec<u32>,
    len: u32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SlotKey(u32);

#[derive(Clone, Copy, Debug)]
pub struct BufferSlotKey(SlotKey);

#[derive(Clone, Copy, Debug)]
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

impl SlotRegister {
    pub fn new() -> Self {
        Self {
            vacant: Vec::new(),
            len: 1,
        }
    }

    pub fn get(&mut self) -> SlotKey {
        let index = match self.vacant.pop() {
            Some(spot) => spot,
            None => {
                let index = self.len;
                self.len += 1;
                index
            }
        };

        SlotKey(index)
    }

    pub fn remove(&mut self, key: SlotKey) {
        self.vacant.push(key.0);
    }

    pub fn is_full(&self) -> bool {
        self.len as usize == self.vacant.len() + 1
    }
}

impl SlotKey {
    pub fn index(&self) -> u32 {
        self.0
    }
}

impl FileSlotKey {
    pub fn index(&self) -> u32 {
        self.0.index()
    }

    pub(crate) fn from_raw_slot(slot: u32) -> Self {
        Self::wrap(SlotKey(slot))
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
