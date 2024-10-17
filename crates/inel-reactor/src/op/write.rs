use std::{io::Result, os::fd::RawFd};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableBuffer, completion::Cancellation, op::Op};

pub struct Write<Buf: StableBuffer> {
    buf: Option<Buf>,
    fd: RawFd,
}

impl<Buf> Write<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(fd: RawFd, buf: Buf) -> Self {
        Self { buf: Some(buf), fd }
    }
}

unsafe impl<Buf> Op for Write<Buf>
where
    Buf: StableBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        let buf = self.buf.as_mut().unwrap();
        opcode::Write::new(Fd(self.fd), buf.stable_ptr(), buf.size() as u32).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        (self.buf.unwrap(), Ok(ret as usize))
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((
            opcode::AsyncCancel::new(user_data).build(),
            self.buf.take().unwrap().into(),
        ))
    }
}
