use std::os::fd::RawFd;

use inel_interface::Reactor;
use inel_reactor::{
    op::{self, Op},
    FileSlotKey,
};

use crate::GlobalReactor;

pub struct OwnedFd(RawFd);

impl OwnedFd {
    pub fn from_raw(fd: RawFd) -> Self {
        Self(fd)
    }

    pub fn as_raw(&self) -> RawFd {
        self.0
    }

    pub fn into_raw(self) -> RawFd {
        let fd = self.0;
        std::mem::forget(self);
        fd
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        let fd = self.0;
        if fd > 0 {
            crate::spawn(op::Close::new(fd).run_on(GlobalReactor));
        }
    }
}

pub struct OwnedDirect {
    slot: FileSlotKey,
    auto: bool,
}

impl OwnedDirect {
    // pub fn get() -> Result<Self> {
    //     let raw = GlobalReactor
    //         .with(|reactor| reactor.get_file_slot())
    //         .unwrap()?;
    //
    //     Ok(Self {
    //         slot: raw,
    //         auto: false,
    //     })
    // }

    pub fn auto(slot: FileSlotKey) -> Self {
        Self { slot, auto: true }
    }

    pub fn as_slot(&self) -> FileSlotKey {
        self.slot
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        let slot = self.as_slot();
        let auto = self.auto;
        crate::spawn(async move {
            let _ = op::Close::new(slot).run_on(GlobalReactor).await;
            if !auto {
                GlobalReactor.with(|reactor| reactor.release_file_slot(slot));
            }
        });
    }
}
