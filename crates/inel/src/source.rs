use std::os::fd::RawFd;

use inel_reactor::{
    op::{self, OpExt},
    AsSource, FileSlotKey, Source,
};

use crate::GlobalReactor;

pub struct OwnedFd {
    fd: RawFd,
}

impl OwnedFd {
    pub fn from_raw(fd: RawFd) -> Self {
        Self { fd }
    }

    pub fn as_raw(&self) -> RawFd {
        self.fd
    }

    pub fn into_raw(self) -> RawFd {
        let fd = self.fd;
        std::mem::forget(self);
        fd
    }
}

impl AsSource for OwnedFd {
    fn as_source(&self) -> Source {
        self.fd.as_source()
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        if self.fd > 0 {
            crate::spawn(op::Close::new(self.fd).run_on(GlobalReactor));
        }
    }
}

pub struct OwnedDirect {
    slot: FileSlotKey,
}

impl AsSource for OwnedDirect {
    fn as_source(&self) -> Source {
        self.slot.as_source()
    }
}

impl OwnedDirect {
    pub fn auto(slot: FileSlotKey) -> Self {
        Self { slot }
    }

    pub fn as_slot(&self) -> FileSlotKey {
        self.slot
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        crate::spawn(op::Close::new(self.as_slot()).run_on(GlobalReactor));
    }
}
