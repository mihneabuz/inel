use std::time::Duration;

use io_uring::{opcode, squeue::Entry, types::Timespec};

use crate::{op::Op, ring::RingResult};

pub struct Timeout {
    abs: Timespec,
}

impl Timeout {
    pub fn new(time: Duration) -> Self {
        Self {
            abs: Timespec::from(time),
        }
    }
}

unsafe impl Op for Timeout {
    type Output = ();

    fn entry(&mut self) -> Entry {
        opcode::Timeout::new(&self.abs).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        debug_assert!(res.ret() == -libc::ETIME || res.ret() == -libc::ECANCELED)
    }
}
