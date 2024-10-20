use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{buffer::StableBuffer, cancellation::Cancellation, op::Op};

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
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.buf.unwrap(), result)
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((
            opcode::AsyncCancel::new(user_data).build(),
            self.buf.take().unwrap().into(),
        ))
    }
}

pub struct WriteVectored<Buf: StableBuffer> {
    bufs: Vec<Buf>,
    iovecs: Option<Vec<libc::iovec>>,
    fd: RawFd,
}

impl<Buf> WriteVectored<Buf>
where
    Buf: StableBuffer,
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

unsafe impl<Buf> Op for WriteVectored<Buf>
where
    Buf: StableBuffer,
{
    type Output = (Vec<Buf>, Result<usize>);

    fn entry(&mut self) -> Entry {
        let iovecs = self.iovecs.as_mut().unwrap();

        self.bufs
            .iter_mut()
            .map(|buf| libc::iovec {
                iov_base: buf.stable_ptr() as *mut _,
                iov_len: buf.size(),
            })
            .for_each(|iovec| iovecs.push(iovec));

        opcode::Writev::new(Fd(self.fd), iovecs.as_ptr(), iovecs.len() as u32).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.bufs, result)
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        let cancels: Vec<Cancellation> = self.bufs.drain(..).map(|buf| buf.into()).collect();

        Some((
            opcode::AsyncCancel::new(user_data).build(),
            Cancellation::combine(cancels),
        ))
    }
}

pub struct WriteVectoredExact<const N: usize, Buf: StableBuffer> {
    bufs: Option<[Buf; N]>,
    iovecs: Option<[libc::iovec; N]>,
    fd: RawFd,
}

impl<const N: usize, Buf> WriteVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    pub fn new(fd: RawFd, bufs: [Buf; N]) -> Self {
        Self {
            bufs: Some(bufs),
            iovecs: None,
            fd,
        }
    }
}

unsafe impl<const N: usize, Buf> Op for WriteVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    type Output = ([Buf; N], Result<usize>);

    fn entry(&mut self) -> Entry {
        let bufs = self.bufs.as_mut().unwrap();

        self.iovecs = Some(std::array::from_fn(|i| libc::iovec {
            iov_base: bufs[i].stable_ptr() as *mut _,
            iov_len: bufs[i].size(),
        }));

        let iovecs = self.iovecs.as_ref().unwrap();

        opcode::Writev::new(Fd(self.fd), iovecs.as_ptr(), iovecs.len() as u32).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        let result = if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        };

        (self.bufs.unwrap(), result)
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
