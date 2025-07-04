use io_uring::types::Target as RawTarget;
use std::os::fd::RawFd;

use crate::DirectSlot;

#[derive(Clone, Debug)]
pub struct Source(RawTarget);

impl Source {
    pub fn fd(fd: RawFd) -> Self {
        Self(RawTarget::Fd(fd))
    }

    pub fn fixed(slot: u32) -> Self {
        Self(RawTarget::Fixed(slot))
    }

    pub(crate) fn as_raw(&self) -> RawTarget {
        self.0
    }
}

pub trait AsSource {
    fn as_source(&self) -> Source;
}

impl AsSource for Source {
    fn as_source(&self) -> Source {
        self.clone()
    }
}

impl AsSource for RawFd {
    fn as_source(&self) -> Source {
        Source::fd(*self)
    }
}

impl AsSource for DirectSlot {
    fn as_source(&self) -> Source {
        Source::fixed(self.index())
    }
}
