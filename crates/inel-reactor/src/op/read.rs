use std::{io::Result, os::fd::RawFd};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{completion::Cancellation, op::Op};

pub trait StableBuffer: Into<Cancellation> {
    fn stable_ptr(&self) -> *const u8;
    fn size(&self) -> usize;
}

pub trait StableMutBuffer: StableBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8;
}

impl<const N: usize> StableBuffer for Box<[u8; N]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.as_ref().len()
    }
}

impl<const N: usize> StableMutBuffer for Box<[u8; N]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableMutBuffer for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

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
