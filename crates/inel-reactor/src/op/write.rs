use std::io::Result;

use io_uring::{opcode, squeue::Entry};

use crate::{
    buffer::{FixedBuffer, StableBuffer},
    cancellation::Cancellation,
    op::{util, Op},
    ring::RingResult,
    source::{AsSource, Source},
};

pub struct Write<Buf> {
    buf: Buf,
    src: Source,
    offset: u64,
}

impl<Buf> Write<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(source: &impl AsSource, buf: Buf) -> Self {
        Self {
            buf,
            src: source.as_source(),
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
        opcode::Write::new(
            self.src.as_raw(),
            self.buf.stable_ptr(),
            self.buf.size() as u32,
        )
        .offset(self.offset)
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        (self.buf, util::expect_positive(&res))
    }

    fn cancel(self) -> Cancellation {
        self.buf.into()
    }
}

pub struct WriteFixed<Buf> {
    buf: Buf,
    src: Source,
    offset: u64,
}

impl<Buf> WriteFixed<Buf>
where
    Buf: FixedBuffer + StableBuffer,
{
    pub fn new(source: &impl AsSource, buf: Buf) -> Self {
        Self {
            buf,
            src: source.as_source(),
            offset: u64::MAX,
        }
    }

    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl<Buf> Op for WriteFixed<Buf>
where
    Buf: FixedBuffer + StableBuffer,
{
    type Output = (Buf, Result<usize>);

    fn entry(&mut self) -> Entry {
        opcode::WriteFixed::new(
            self.src.as_raw(),
            self.buf.stable_ptr(),
            self.buf.size() as u32,
            self.buf.slot().index() as u16,
        )
        .offset(self.offset)
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        (self.buf, util::expect_positive(&res))
    }

    fn cancel(self) -> Cancellation {
        self.buf.into()
    }
}

pub struct WriteVectored<Buf> {
    bufs: Vec<Buf>,
    iovecs: Vec<libc::iovec>,
    src: Source,
    offset: u64,
}

impl<Buf> WriteVectored<Buf>
where
    Buf: StableBuffer,
{
    pub fn new(source: &impl AsSource, bufs: Vec<Buf>) -> Self {
        let iovecs = Vec::with_capacity(bufs.len());
        Self {
            bufs,
            iovecs,
            src: source.as_source(),
            offset: u64::MAX,
        }
    }

    pub fn from_iter<I>(source: &impl AsSource, bufs: I) -> Self
    where
        I: Iterator<Item = Buf>,
    {
        let bufs = bufs.collect::<Vec<_>>();
        Self::new(source, bufs)
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

        opcode::Writev::new(
            self.src.as_raw(),
            self.iovecs.as_ptr(),
            self.iovecs.len() as u32,
        )
        .offset(self.offset)
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        (self.bufs, util::expect_positive(&res))
    }

    fn cancel(mut self) -> Cancellation {
        Cancellation::combine(self.bufs.drain(..).map(|buf| buf.into()).collect())
    }
}

pub struct WriteVectoredExact<const N: usize, Buf> {
    bufs: [Buf; N],
    iovecs: [libc::iovec; N],
    src: Source,
    offset: u64,
}

impl<const N: usize, Buf> WriteVectoredExact<N, Buf>
where
    Buf: StableBuffer,
{
    pub fn new(source: &impl AsSource, bufs: [Buf; N]) -> Self {
        Self {
            bufs,
            iovecs: std::array::from_fn(|_| libc::iovec {
                iov_base: std::ptr::null_mut(),
                iov_len: 0,
            }),
            src: source.as_source(),
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

        opcode::Writev::new(
            self.src.as_raw(),
            self.iovecs.as_ptr(),
            self.iovecs.len() as u32,
        )
        .offset(self.offset)
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        (self.bufs, util::expect_positive(&res))
    }

    fn cancel(self) -> Cancellation {
        Cancellation::combine(self.bufs.into_iter().map(|buf| buf.into()).collect())
    }
}
