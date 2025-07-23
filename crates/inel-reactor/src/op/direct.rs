use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{opcode, squeue::Entry, types::Fixed};

use crate::{
    op::Op,
    ring::{DirectSlot, RingResult},
    source::{AsDirectSlot, DirectAutoFd},
};

pub struct RegisterFile {
    fd: RawFd,
}

impl RegisterFile {
    pub fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

unsafe impl Op for RegisterFile {
    type Output = Result<DirectAutoFd>;

    fn entry(&mut self) -> Entry {
        opcode::FilesUpdate::new(&self.fd as *const _, 1)
            .offset(None)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        match res.ret() {
            1 => Ok(DirectAutoFd::from_raw_slot(self.fd as u32)),
            ..0 => Err(Error::from_raw_os_error(res.ret())),
            0 => unreachable!(),
            _ => unreachable!(),
        }
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct InstallSlot<'a> {
    slot: &'a DirectSlot,
}

impl<'a> InstallSlot<'a> {
    pub fn new<T: AsDirectSlot>(slot: &'a T) -> Self {
        Self {
            slot: slot.as_slot(),
        }
    }
}

unsafe impl Op for InstallSlot<'_> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::FixedFdInstall::new(Fixed(self.slot.index()), 0).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        super::util::expect_fd(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}
