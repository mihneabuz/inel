use std::{io::Result, ops::RangeBounds};

use crate::GlobalReactor;

pub use inel_reactor::buffer::{FixedBuffer, StableBuffer, View};

#[allow(private_interfaces)]
pub type Fixed<B> = inel_reactor::buffer::Fixed<B, GlobalReactor>;

pub trait StableBufferExt: StableBuffer {
    fn view<R>(self, range: R) -> View<Self, R>
    where
        R: RangeBounds<usize>,
        Self: Sized;

    #[allow(private_interfaces)]
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

    #[allow(private_interfaces)]
    fn fix(self) -> Result<Fixed<Self>>
    where
        Self: Sized,
    {
        Fixed::register(self, GlobalReactor)
    }
}
