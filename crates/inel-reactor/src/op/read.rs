use std::{io::Result, os::fd::RawFd};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableMutBuffer, cancellation::Cancellation, op::Op};

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

pub struct ReadVectored<Buf: StableMutBuffer> {
    bufs: Vec<Buf>,
    iovecs: Option<Vec<libc::iovec>>,
    fd: RawFd,
}

impl<Buf> ReadVectored<Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, bufs: Vec<Buf>) -> Self {
        let iovecs = Some(Vec::with_capacity(bufs.len()));
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
        let iovecs = self.iovecs.as_mut().unwrap();

        self.bufs
            .iter_mut()
            .map(|buf| libc::iovec {
                iov_base: buf.stable_mut_ptr() as *mut _,
                iov_len: buf.size(),
            })
            .for_each(|iovec| iovecs.push(iovec));

        opcode::Readv::new(Fd(self.fd), iovecs.as_ptr(), iovecs.len() as u32).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        (self.bufs, Ok(ret as usize))
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        let cancels: Vec<Cancellation> = self.bufs.drain(..).map(|buf| buf.into()).collect();

        Some((
            opcode::AsyncCancel::new(user_data).build(),
            Cancellation::combine(cancels),
        ))
    }
}

pub struct ReadVectoredExact<const N: usize, Buf: StableMutBuffer> {
    bufs: Option<[Buf; N]>,
    iovecs: Option<[libc::iovec; N]>,
    fd: RawFd,
}

impl<const N: usize, Buf> ReadVectoredExact<N, Buf>
where
    Buf: StableMutBuffer,
{
    pub fn new(fd: RawFd, bufs: [Buf; N]) -> Self {
        Self {
            bufs: Some(bufs),
            iovecs: None,
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
        let bufs = self.bufs.as_mut().unwrap();

        self.iovecs = Some(std::array::from_fn(|i| libc::iovec {
            iov_base: bufs[i].stable_mut_ptr() as *mut _,
            iov_len: bufs[i].size(),
        }));

        let iovecs = self.iovecs.as_ref().unwrap();

        opcode::Readv::new(Fd(self.fd), iovecs.as_ptr(), iovecs.len() as u32).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        (self.bufs.unwrap(), Ok(ret as usize))
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        let cancels: Vec<Cancellation> = self
            .bufs
            .take()
            .unwrap()
            .into_iter()
            .map(|buf| buf.into())
            .collect();

        Some((
            opcode::AsyncCancel::new(user_data).build(),
            Cancellation::combine(cancels),
        ))
    }
}
