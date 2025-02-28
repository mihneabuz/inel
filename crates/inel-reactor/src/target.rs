use io_uring::types::Target as RawTarget;
use std::os::fd::RawFd;

use crate::FileSlotKey;

#[derive(Clone, Debug)]
pub struct Target(RawTarget);

impl Target {
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

pub trait IntoTarget {
    fn into_target(self) -> Target;
}

impl IntoTarget for RawFd {
    fn into_target(self) -> Target {
        Target::fd(self)
    }
}

impl IntoTarget for FileSlotKey {
    fn into_target(self) -> Target {
        Target::fixed(self.index())
    }
}

impl IntoTarget for &Target {
    fn into_target(self) -> Target {
        self.clone()
    }
}
