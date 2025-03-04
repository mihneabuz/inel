use io_uring::types::Target as RawTarget;
use std::os::fd::RawFd;

use crate::FileSlotKey;

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

pub trait IntoSource {
    fn into_source(self) -> Source;
}

impl IntoSource for Source {
    fn into_source(self) -> Source {
        self
    }
}

impl IntoSource for RawFd {
    fn into_source(self) -> Source {
        Source::fd(self)
    }
}

impl IntoSource for FileSlotKey {
    fn into_source(self) -> Source {
        Source::fixed(self.index())
    }
}

impl IntoSource for &Source {
    fn into_source(self) -> Source {
        self.clone()
    }
}
