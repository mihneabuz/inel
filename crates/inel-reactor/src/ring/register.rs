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
}

impl BufferSlotKey {
    pub fn index(&self) -> u32 {
        self.0.index()
    }
}
