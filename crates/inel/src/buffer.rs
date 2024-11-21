use std::{io::Result, ops::RangeBounds};

use crate::GlobalReactor;

pub use inel_reactor::buffer::{FixedBuffer, StableBuffer, View};
use inel_reactor::{BufferKey, Cancellation};

pub trait StableBufferExt: StableBuffer {
    fn view<R>(self, range: R) -> View<Self, R>
    where
        R: RangeBounds<usize>,
        Self: Sized;

    fn fix(self) -> Result<Fixed<Self>>
    where
        Self: Sized;
}

impl<T> StableBufferExt for T
where
    T: StableBuffer,
{
    fn view<R: RangeBounds<usize>>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }

    fn fix(self) -> Result<Fixed<Self>>
    where
        Self: Sized,
    {
        Fixed::register(self)
    }
}

pub struct Fixed<B: StableBuffer>(inel_reactor::buffer::Fixed<B, GlobalReactor>);

impl<B: StableBuffer> Fixed<B> {
    pub fn register(buf: B) -> Result<Self> {
        let raw = inel_reactor::buffer::Fixed::register(buf, GlobalReactor)?;
        Ok(Self(raw))
    }

    pub fn inner(&self) -> &B {
        self.0.inner()
    }

    pub fn inner_mut(&mut self) -> &mut B {
        self.0.inner_mut()
    }

    pub fn into_inner(self) -> B {
        self.0.into_inner()
    }
}

impl<B: StableBuffer> From<Fixed<B>> for Cancellation {
    fn from(value: Fixed<B>) -> Self {
        value.0.into_inner().into()
    }
}

impl<B: StableBuffer> StableBuffer for Fixed<B> {
    fn stable_ptr(&self) -> *const u8 {
        self.0.stable_ptr()
    }

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.0.stable_mut_ptr()
    }

    fn size(&self) -> usize {
        self.0.size()
    }

    fn capacity(&self) -> usize {
        self.0.capacity()
    }
}

impl<B: StableBuffer> FixedBuffer for Fixed<B> {
    fn key(&self) -> &BufferKey {
        self.0.key()
    }
}
