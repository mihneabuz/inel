use std::{io::Result, mem::ManuallyDrop, os::fd::RawFd};

use inel_reactor::{
    op::{self, OpExt},
    AsSource, DirectSlot, RingReactor, Source,
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
            crate::spawn(op::Close::new(&self.fd).run_on(GlobalReactor));
        }
    }
}

pub struct OwnedDirect {
    slot: ManuallyDrop<DirectSlot>,
    manual: bool,
}

impl AsSource for OwnedDirect {
    fn as_source(&self) -> Source {
        self.slot.as_source()
    }
}

impl OwnedDirect {
    pub fn reserve() -> Result<Self> {
        let slot = GlobalReactor.get_direct_slot()?;
        Ok(Self {
            slot: ManuallyDrop::new(slot),
            manual: true,
        })
    }

    pub fn auto(slot: DirectSlot) -> Self {
        Self {
            slot: ManuallyDrop::new(slot),
            manual: false,
        }
    }

    pub fn as_slot(&self) -> &DirectSlot {
        &self.slot
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        let slot = unsafe { ManuallyDrop::take(&mut self.slot) };
        if self.manual {
            GlobalReactor.release_direct_slot(slot);
        } else {
            crate::spawn(op::Close::new(&slot).run_on(GlobalReactor));
        }
    }
}
