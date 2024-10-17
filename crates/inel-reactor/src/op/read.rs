use std::{io::Result, os::fd::RawFd};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableMutBuffer, completion::Cancellation, op::Op};

pub struct Read<Buf: StableMutBuffer> {
    buf: Option<Buf>,
    fd: RawFd,
}

impl<Buf> Read<Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, buf: Buf) -> Self {
        Self { buf: Some(buf), fd }
    }
}

unsafe impl<Buf> Op for Read<Buf>
where
    Buf: StableMutBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        let buf = self.buf.as_mut().unwrap();
        opcode::Read::new(Fd(self.fd), buf.stable_mut_ptr(), buf.size() as u32).build()
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
