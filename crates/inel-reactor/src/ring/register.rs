pub struct SlotRegister {
    vacant: Vec<u16>,
    len: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct SlotKey(u16);

impl SlotRegister {
    pub fn new() -> Self {
        Self {
            vacant: Vec::new(),
            len: 0,
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
    pub fn index(&self) -> u16 {
        self.0
    }

    pub fn offset(&self) -> u32 {
        self.0 as u32
    }
}
