use std::{io::Result, mem::ManuallyDrop, os::fd::RawFd};

use inel_reactor::{
    op::{self, DetachOp},
    ring::DirectSlot,
    source::{AsDirectSlot, AsSource, DirectAutoFd, DirectFd, Source},
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
            op::Close::new(&self.fd).run_detached(&mut GlobalReactor);
        }
    }
}

pub enum OwnedDirect {
    Manual(ManuallyDrop<DirectFd>),
    Auto(DirectAutoFd),
}

impl AsSource for OwnedDirect {
    fn as_source(&self) -> Source {
        match self {
            Self::Manual(fd) => fd.as_source(),
            Self::Auto(fd) => fd.as_source(),
        }
    }
}

impl AsDirectSlot for OwnedDirect {
    fn as_slot(&self) -> &DirectSlot {
        match self {
            Self::Manual(fd) => fd.as_slot(),
            Self::Auto(fd) => fd.as_slot(),
        }
    }
}

impl OwnedDirect {
    pub fn reserve() -> Result<Self> {
        let direct = DirectFd::get(&mut GlobalReactor)?;
        Ok(Self::Manual(ManuallyDrop::new(direct)))
    }

    pub fn auto(direct: DirectAutoFd) -> Self {
        Self::Auto(direct)
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        match self {
            Self::Manual(fd) => {
                unsafe { ManuallyDrop::take(fd) }.release(&mut GlobalReactor);
            }
            Self::Auto(fd) => {
                op::Close::new(fd).run_detached(&mut GlobalReactor);
            }
        }
    }
}
