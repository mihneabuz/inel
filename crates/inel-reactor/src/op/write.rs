use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableBuffer, cancellation::Cancellation, op::Op};

pub struct Write<Buf: StableBuffer> {
    buf: Buf,
    fd: RawFd,
    offset: u64,
}

impl<Buf> Write<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(fd: RawFd, buf: Buf) -> Self {
        Self {
            buf,
            fd,
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for Write<Buf>
where
    Buf: StableBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Write::new(Fd(self.fd), self.buf.stable_ptr(), self.buf.size() as u32)
            .offset(self.offset)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.buf, result)
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(opcode::AsyncCancel::new(user_data).build()),
            self.buf.into(),
        )
    }
}

pub struct WriteVectored<Buf: StableBuffer> {
    bufs: Vec<Buf>,
    iovecs: Vec<libc::iovec>,
    fd: RawFd,
    offset: u64,
}

impl<Buf> WriteVectored<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(fd: RawFd, bufs: Vec<Buf>) -> Self {
        let iovecs = Vec::with_capacity(bufs.len());
        Self {
            bufs,
            iovecs,
            fd,
            offset: u64::MAX,
        }
    }

    pub fn from_iter<I>(fd: RawFd, bufs: I) -> Self
    where
        I: Iterator<Item = Buf>,
    {
        let bufs = bufs.collect::<Vec<_>>();
        Self::new(fd, bufs)
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for WriteVectored<Buf>
where
    Buf: StableBuffer,
{
    type Output = (Vec<Buf>, Result<usize>);

    fn entry(&mut self) -> Entry {
        self.bufs
            .iter_mut()
            .map(|buf| libc::iovec {
                iov_base: buf.stable_ptr() as *mut _,
                iov_len: buf.size(),
            })
            .for_each(|iovec| self.iovecs.push(iovec));

        opcode::Writev::new(Fd(self.fd), self.iovecs.as_ptr(), self.iovecs.len() as u32)
            .offset(self.offset)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.bufs, result)
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        let mut bufs = self.bufs;
        let cancels: Vec<Cancellation> = bufs.drain(..).map(|buf| buf.into()).collect();

        (
            Some(opcode::AsyncCancel::new(user_data).build()),
            Cancellation::combine(cancels),
        )
    }
}

pub struct WriteVectoredExact<const N: usize, Buf: StableBuffer> {
    bufs: [Buf; N],
    iovecs: [libc::iovec; N],
    fd: RawFd,
    offset: u64,
}

impl<const N: usize, Buf> WriteVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    pub fn new(fd: RawFd, bufs: [Buf; N]) -> Self {
        Self {
            bufs,
            iovecs: std::array::from_fn(|_| libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }),
            fd,
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<const N: usize, Buf> Op for WriteVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    type Output = ([Buf; N], Result<usize>);

    fn entry(&mut self) -> Entry {
        for i in 0..N {
            self.iovecs[i].iov_base = self.bufs[i].stable_ptr() as *mut _;
            self.iovecs[i].iov_len = self.bufs[i].size();
        }

        opcode::Writev::new(Fd(self.fd), self.iovecs.as_ptr(), self.iovecs.len() as u32)
            .offset(self.offset)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.bufs, result)
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        let cancels: Vec<Cancellation> = self.bufs.into_iter().map(|buf| buf.into()).collect();

        (
            Some(opcode::AsyncCancel::new(user_data).build()),
            Cancellation::combine(cancels),
        )
    }
}
