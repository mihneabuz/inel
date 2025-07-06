use std::{io::Result, os::fd::RawFd};

use io_uring::types::Target as RawTarget;

use crate::{DirectSlot, Ring, RingReactor};

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

pub struct DirectFd<R> {
    slot: DirectSlot,
    reactor: R,
}

impl<R> DirectFd<R>
where
    R: Reactor<Handle = Ring>,
{
    pub fn get(mut reactor: R) -> Result<Self> {
        let slot = reactor.get_direct_slot()?;
        Ok(Self { slot, reactor })
    }

    pub fn from_slot(slot: DirectSlot, reactor: R) -> Self {
        Self { slot, reactor }
    }

    pub fn slot(&self) -> &DirectSlot {
        &self.slot
    }

    pub fn release(mut self) {
        self.reactor.release_direct_slot(self.slot);
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

impl<R> AsSource for DirectFd<R> {
    fn as_source(&self) -> Source {
        Source::fixed(self.slot.index())
    }
}
