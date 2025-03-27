use std::{io::Result, os::fd::RawFd};

use futures::FutureExt;
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
            crate::spawn(async move {
                let _ = op::Close::new(fd).run_on(GlobalReactor).await;
            });
        }
    }
}

pub struct OwnedDirect(FileSlotKey);

impl OwnedDirect {
    pub fn from_slot(slot: FileSlotKey) -> Self {
        Self(slot)
    }

    pub fn as_slot(&self) -> FileSlotKey {
        self.0
    }

    pub fn get(fd: Option<RawFd>) -> Result<Self> {
        let raw = GlobalReactor
            .with(|reactor| reactor.register_file(fd))
            .unwrap()?;

        Ok(Self::from_slot(raw))
    }
}

impl Drop for OwnedDirect {
    fn drop(&mut self) {
        let slot = self.as_slot();
        crate::spawn(async move {
            let _ = op::Close::new(slot)
                .run_on(GlobalReactor)
                .then(|_| async {
                    GlobalReactor.with(|reactor| reactor.unregister_file(slot));
                })
                .await;
        });
    }
}
