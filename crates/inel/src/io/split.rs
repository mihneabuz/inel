use std::{
    ops::Deref,
    os::{fd::AsRawFd, unix::prelude::RawFd},
    rc::Rc,
};

use super::{ReadSource, WriteSource};

pub fn split<S>(source: S) -> (ReadHandle<S>, WriteHandle<S>)
where
    S: ReadSource + WriteSource,
{
    let rc1 = Rc::new(source);
    let rc2 = Rc::clone(&rc1);
    (ReadHandle { inner: rc1 }, WriteHandle { inner: rc2 })
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

impl<T> ReadSource for ReadHandle<T> where T: ReadSource {}
impl<T> WriteSource for WriteHandle<T> where T: WriteSource {}

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
