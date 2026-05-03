use crate::{sys::BufRing, utils::Slab};

pub struct BufRings {
    rings: Slab<BufRing>,
}

impl BufRings {
    pub const fn new() -> Self {
        Self { rings: Slab::new() }
    }

    pub fn insert(&mut self, max_entries: u16) -> BufGroupId {
        let id = self.rings.insert(BufRing::new(max_entries));
        BufGroupId(id as u16)
    }

    pub fn remove(&mut self, bgid: BufGroupId) {
        let _ = self.rings.remove(bgid.0 as u32);
    }

    pub fn get_mut(&mut self, bgid: BufGroupId) -> &mut BufRing {
        self.rings.get_mut(bgid.0 as u32)
    }

    pub fn get_buf(&mut self, _bgid: BufGroupId, _bid: BufId) -> Buf {
        todo!()
    }

    pub fn put_buf(&mut self, _bgid: BufGroupId, _buf: Buf) {
        todo!()
    }

    pub fn recycle(&mut self, bgid: BufGroupId, bid: BufId) {
        let buf = self.get_buf(bgid, bid);
        self.put_buf(bgid, buf);
    }
}

#[derive(Copy, Clone)]
pub struct BufGroupId(pub(crate) u16);

pub type BufId = u16;

pub struct Buf;
