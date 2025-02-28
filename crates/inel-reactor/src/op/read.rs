use std::io::{Error, Result};

use io_uring::{opcode, squeue::Entry};

use crate::{
    buffer::{FixedBuffer, StableBuffer},
    op::Op,
    Cancellation, IntoTarget, Target,
};

pub struct Read<Buf: StableBuffer> {
    buf: Buf,
    target: Target,
    offset: u64,
}

impl<Buf> Read<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(target: impl IntoTarget, buf: Buf) -> Self {
        Self {
            buf,
            target: target.into_target(),
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for Read<Buf>
where
    Buf: StableBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::Read::new(
            self.target.as_raw(),
            self.buf.stable_mut_ptr(),
            self.buf.size() as u32,
        )
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

pub struct ReadFixed<Buf: FixedBuffer> {
    buf: Buf,
    target: Target,
    offset: u64,
}

impl<Buf> ReadFixed<Buf>
where
    Buf: FixedBuffer,
{
    pub fn new(target: impl IntoTarget, buf: Buf) -> Self {
        Self {
            buf,
            target: target.into_target(),
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for ReadFixed<Buf>
where
    Buf: FixedBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::ReadFixed::new(
            self.target.as_raw(),
            self.buf.stable_mut_ptr(),
            self.buf.size() as u32,
            self.buf.key().index() as u16,
        )
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

pub struct ReadVectored<Buf: StableBuffer> {
    bufs: Vec<Buf>,
    iovecs: Vec<libc::iovec>,
    target: Target,
    offset: u64,
}

impl<Buf> ReadVectored<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(target: impl IntoTarget, bufs: Vec<Buf>) -> Self {
        let iovecs = Vec::with_capacity(bufs.len());
        Self {
            bufs,
            iovecs,
            target: target.into_target(),
            offset: u64::MAX,
        }
    }

    pub fn from_iter<I>(target: impl IntoTarget, bufs: I) -> Self
    where
        I: Iterator<Item = Buf>,
    {
        let bufs = bufs.collect::<Vec<_>>();
        Self::new(target, bufs)
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for ReadVectored<Buf>
where
    Buf: StableBuffer,
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

        opcode::Readv::new(
            self.target.as_raw(),
            self.iovecs.as_ptr(),
            self.iovecs.len() as u32,
        )
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

pub struct ReadVectoredExact<const N: usize, Buf: StableBuffer> {
    bufs: [Buf; N],
    iovecs: [libc::iovec; N],
    target: Target,
    offset: u64,
}

impl<const N: usize, Buf> ReadVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    pub fn new(target: impl IntoTarget, bufs: [Buf; N]) -> Self {
        Self {
            bufs,
            iovecs: std::array::from_fn(|_| libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }),
            target: target.into_target(),
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<const N: usize, Buf> Op for ReadVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    type Output = ([Buf; N], Result<usize>);

    fn entry(&mut self) -> Entry {
        for i in 0..N {
            self.iovecs[i].iov_base = self.bufs[i].stable_mut_ptr() as *mut _;
            self.iovecs[i].iov_len = self.bufs[i].size();
        }

        opcode::Readv::new(
            self.target.as_raw(),
            self.iovecs.as_ptr(),
            self.iovecs.len() as u32,
        )
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
