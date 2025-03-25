use std::os::fd::AsRawFd;

use futures::FutureExt;
use inel_interface::Reactor;
use inel_reactor::{
    op::{self, Op},
    FileSlotKey,
};

use crate::GlobalReactor;

pub fn spawn_drop(fd: impl AsRawFd) {
    let fd = fd.as_raw_fd();
    if fd > 0 {
        crate::spawn(async move {
            let _ = op::Close::new(fd).run_on(GlobalReactor).await;
        });
    }
}

pub fn spawn_drop_direct(slot: FileSlotKey) {
    crate::spawn(async move {
        let _ = op::Close::new(slot)
            .run_on(GlobalReactor)
            .then(|_| async { GlobalReactor.with(|reactor| reactor.unregister_file(slot)) })
            .await;
    });
}
