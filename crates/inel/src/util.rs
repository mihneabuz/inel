use std::os::fd::AsRawFd;

use inel_reactor::op::{self, Op};

use crate::GlobalReactor;

pub fn spawn_drop(fd: impl AsRawFd) {
    let fd = fd.as_raw_fd();
    if fd > 0 {
        crate::spawn(async move {
            let _ = op::Close::new(fd).run_on(GlobalReactor).await;
        });
    }
}
