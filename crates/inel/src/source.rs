use std::{io::Result, mem::ManuallyDrop, os::fd::RawFd};

use inel_reactor::{
    op::{self, OpExt},
    AsSource, DirectFd, DirectSlot, Source,
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
    fd: ManuallyDrop<DirectFd<GlobalReactor>>,
    manual: bool,
}

impl AsSource for OwnedDirect {
    fn as_source(&self) -> Source {
        self.fd.as_source()
    }
}

impl OwnedDirect {
    pub fn reserve() -> Result<Self> {
        Ok(Self {
            fd: ManuallyDrop::new(DirectFd::get(GlobalReactor)?),
            manual: true,
        })
    }

    pub fn auto(slot: DirectSlot) -> Self {
        Self {
            fd: ManuallyDrop::new(DirectFd::from_slot(slot, GlobalReactor)),
            manual: false,
        }
    }

    pub fn slot(&self) -> &DirectSlot {
        self.fd.slot()
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        let direct = unsafe { ManuallyDrop::take(&mut self.fd) };
        if self.manual {
            direct.release();
        } else {
            crate::spawn(op::Close::new(&direct).run_on(GlobalReactor));
        }
    }
}
