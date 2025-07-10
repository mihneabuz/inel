use std::{
    io::Result,
    ops::{Deref, DerefMut, RangeBounds},
};

use crate::GlobalReactor;

pub use inel_reactor::buffer::{FixedBuffer, StableBuffer, View};
use inel_reactor::{buffer::StableBufferMut, cancellation::Cancellation, ring::BufferSlot};

pub trait StableBufferExt: StableBuffer {
    fn view<R>(self, range: R) -> View<Self, R>
    where
        R: RangeBounds<usize>,
        Self: Sized;
}

impl<T> StableBufferExt for T
where
    T: StableBuffer,
{
    fn view<R: RangeBounds<usize>>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }
}

pub struct Fixed(inel_reactor::buffer::Fixed<GlobalReactor>);

impl Fixed {
    pub fn new(size: usize) -> Result<Self> {
        let raw = inel_reactor::buffer::Fixed::new(size, GlobalReactor)?;
        Ok(Self(raw))
    }

    pub fn register(buffer: Box<[u8]>) -> Result<Self> {
        let raw = inel_reactor::buffer::Fixed::register(buffer, GlobalReactor)?;
        Ok(Self(raw))
    }
}

impl AsRef<[u8]> for Fixed {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl AsMut<[u8]> for Fixed {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut_slice()
    }
}

impl Deref for Fixed {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DerefMut for Fixed {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

impl From<Fixed> for Cancellation {
    fn from(value: Fixed) -> Self {
        value.0.into()
    }
}

impl StableBuffer for Fixed {
    fn stable_ptr(&self) -> *const u8 {
        self.0.stable_ptr()
    }

    fn size(&self) -> usize {
        self.0.size()
    }
}

impl StableBufferMut for Fixed {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.0.stable_mut_ptr()
    }
}

impl FixedBuffer for Fixed {
    fn slot(&self) -> &BufferSlot {
        self.0.slot()
    }
}
