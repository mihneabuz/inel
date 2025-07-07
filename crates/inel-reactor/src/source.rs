use std::{io::Result, os::fd::RawFd};

use io_uring::types::Target as RawTarget;

use crate::{ring::DirectSlot, Ring, RingReactor};

use inel_interface::Reactor;

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

pub struct DirectFd {
    slot: DirectSlot,
}

impl DirectFd {
    pub fn get<R>(reactor: &mut R) -> Result<Self>
    where
        R: Reactor<Handle = Ring>,
    {
        let slot = reactor.get_direct_slot()?;
        Ok(Self { slot })
    }

    pub fn release<R>(self, reactor: &mut R)
    where
        R: Reactor<Handle = Ring>,
    {
        reactor.release_direct_slot(self.slot);
    }
}

pub struct DirectAutoFd {
    slot: DirectSlot,
}

impl DirectAutoFd {
    pub(crate) fn from_raw_slot(slot: u32) -> Self {
        Self {
            slot: DirectSlot::from_raw_slot(slot),
        }
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

impl AsSource for DirectFd {
    fn as_source(&self) -> Source {
        Source::fixed(self.slot.index())
    }
}

impl AsSource for DirectAutoFd {
    fn as_source(&self) -> Source {
        Source::fixed(self.slot.index())
    }
}

pub trait AsDirectSlot {
    fn as_slot(&self) -> &DirectSlot;
}

impl AsDirectSlot for DirectFd {
    fn as_slot(&self) -> &DirectSlot {
        &self.slot
    }
}

impl AsDirectSlot for DirectAutoFd {
    fn as_slot(&self) -> &DirectSlot {
        &self.slot
    }
}
