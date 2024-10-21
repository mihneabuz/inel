use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableMutBuffer, cancellation::Cancellation, op::Op};

pub struct Read<Buf: StableMutBuffer> {
    buf: Buf,
    fd: RawFd,
}

impl<Buf> Read<Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, buf: Buf) -> Self {
        Self { buf, fd }
    }
}

unsafe impl<Buf> Op for Read<Buf>
where
    Buf: StableMutBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Read::new(
            Fd(self.fd),
            self.buf.stable_mut_ptr(),
            self.buf.size() as u32,
        )
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

pub struct ReadVectored<Buf: StableMutBuffer> {
    bufs: Vec<Buf>,
    iovecs: Vec<libc::iovec>,
    fd: RawFd,
}

impl<Buf> ReadVectored<Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, bufs: Vec<Buf>) -> Self {
        let iovecs = Vec::with_capacity(bufs.len());
        Self { bufs, iovecs, fd }
    }

    pub fn from_iter<I>(fd: RawFd, bufs: I) -> Self
    where
        I: Iterator<Item = Buf>,
    {
        let bufs = bufs.collect::<Vec<_>>();
        Self::new(fd, bufs)
    }
}

unsafe impl<Buf> Op for ReadVectored<Buf>
where
    Buf: StableMutBuffer,
{
    type Output = (Vec<Buf>, Result<usize>);

    fn entry(&mut self) -> Entry {
        self.bufs
            .iter_mut()
            .map(|buf| libc::iovec {
                iov_base: buf.stable_mut_ptr() as *mut _,
                iov_len: buf.size(),
            })
            .for_each(|iovec| self.iovecs.push(iovec));

        opcode::Readv::new(Fd(self.fd), self.iovecs.as_ptr(), self.iovecs.len() as u32).build()
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

pub struct ReadVectoredExact<const N: usize, Buf: StableMutBuffer> {
    bufs: [Buf; N],
    iovecs: [libc::iovec; N],
    fd: RawFd,
}

impl<const N: usize, Buf> ReadVectoredExact<N, Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, bufs: [Buf; N]) -> Self {
        Self {
            bufs,
            iovecs: std::array::from_fn(|_| libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }),
            fd,
        }
    }
}

unsafe impl<const N: usize, Buf> Op for ReadVectoredExact<N, Buf>
where
    Buf: StableMutBuffer,
{
    type Output = ([Buf; N], Result<usize>);

    fn entry(&mut self) -> Entry {
        for i in 0..N {
            self.iovecs[i].iov_base = self.bufs[i].stable_mut_ptr() as *mut _;
            self.iovecs[i].iov_len = self.bufs[i].size();
        }

        opcode::Readv::new(Fd(self.fd), self.iovecs.as_ptr(), self.iovecs.len() as u32).build()
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
