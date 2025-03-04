use std::{
    ops::Deref,
    os::{fd::AsRawFd, unix::prelude::RawFd},
    rc::Rc,
};

use inel_reactor::Source;

use super::{BufReader, BufWriter, ReadSource, WriteSource};

pub trait Split {
    fn split(self) -> (ReadHandle<Self>, WriteHandle<Self>)
    where
        Self: Sized;

    fn split_buffered(self) -> (BufReader<ReadHandle<Self>>, BufWriter<WriteHandle<Self>>)
    where
        Self: Sized;
}

impl<T> Split for T
where
    T: ReadSource + WriteSource,
{
    fn split(self) -> (ReadHandle<Self>, WriteHandle<Self>) {
        let rc1 = Rc::new(self);
        let rc2 = Rc::clone(&rc1);
        (ReadHandle { inner: rc1 }, WriteHandle { inner: rc2 })
    }

    fn split_buffered(self) -> (BufReader<ReadHandle<Self>>, BufWriter<WriteHandle<Self>>) {
        let (read, write) = Self::split(self);
        (BufReader::new(read), BufWriter::new(write))
    }
}

pub struct ReadHandle<T> {
    inner: Rc<T>,
}

pub struct WriteHandle<T> {
    inner: Rc<T>,
}

impl<T> AsRawFd for ReadHandle<T>
where
    T: AsRawFd,
{
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl<T> AsRawFd for WriteHandle<T>
where
    T: AsRawFd,
{
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl<T> ReadSource for ReadHandle<T>
where
    T: ReadSource,
{
    fn read_source(&self) -> Source {
        self.inner.read_source()
    }
}

impl<T> WriteSource for WriteHandle<T>
where
    T: WriteSource,
{
    fn write_source(&self) -> Source {
        self.inner.write_source()
    }
}

impl<T> Deref for ReadHandle<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl<T> Deref for WriteHandle<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}
