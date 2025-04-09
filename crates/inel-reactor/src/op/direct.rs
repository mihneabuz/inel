use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{opcode, squeue::Entry};

use crate::{op::Op, ring::RingResult, FileSlotKey};

pub struct RegisterFile {
    fd: RawFd,
}

impl RegisterFile {
    pub fn new(fd: RawFd) -> Self {
        Self { fd }
    }
}

unsafe impl Op for RegisterFile {
    type Output = Result<FileSlotKey>;

    fn entry(&mut self) -> Entry {
        opcode::FilesUpdate::new(&self.fd as *const _, 1)
            .offset(None)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        match res.ret() {
            1 => Ok(FileSlotKey::from_raw_slot(self.fd as u32)),
            ..0 => Err(Error::from_raw_os_error(res.ret())),
            0 => unreachable!(),
            _ => unreachable!(),
        }
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}
