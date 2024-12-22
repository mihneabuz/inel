use std::{
    io::Result,
    ops::{Deref, DerefMut, RangeBounds},
};

use crate::GlobalReactor;

pub use inel_reactor::buffer::{FixedBuffer, StableBuffer, View};
use inel_reactor::{Cancellation, SlotKey};

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

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.0.stable_mut_ptr()
    }

    fn size(&self) -> usize {
        self.0.size()
    }
}

impl FixedBuffer for Fixed {
    fn key(&self) -> &SlotKey {
        self.0.key()
    }
}
