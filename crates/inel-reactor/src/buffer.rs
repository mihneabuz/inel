use std::ops::{Bound, RangeBounds};

use crate::cancellation::Cancellation;

pub trait StableBuffer: Into<Cancellation> {
    fn stable_ptr(&self) -> *const u8;
    fn size(&self) -> usize;

    fn view<R>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }
}

pub trait StableMutBuffer: StableBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8;
}

impl<const N: usize> StableBuffer for Box<[u8; N]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.as_ref().len()
    }
}

impl<const N: usize> StableMutBuffer for Box<[u8; N]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for Box<[u8]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableMutBuffer for Box<[u8]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableMutBuffer for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

pub struct View<B, R> {
    inner: B,
    range: R,
}

impl<B, R> View<B, R>
where
    B: StableBuffer,
{
    fn new(buffer: B, range: R) -> Self {
        Self {
            inner: buffer,
            range,
        }
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    pub fn unview(self) -> B {
        self.inner
    }
}

impl<B, R> View<B, R>
where
    B: StableBuffer,
    R: RangeBounds<usize>,
{
    fn start(&self) -> usize {
        match self.range.start_bound() {
            Bound::Included(x) => *x,
            Bound::Excluded(x) => *x + 1,
            Bound::Unbounded => 0,
        }
    }

    fn end(&self) -> usize {
        match self.range.end_bound() {
            Bound::Included(x) => *x + 1,
            Bound::Excluded(x) => *x,
            Bound::Unbounded => self.inner.size(),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.stable_ptr(), self.size()) }
    }
}

impl<B, R> AsRef<[u8]> for View<B, R>
where
    B: StableBuffer,
    R: RangeBounds<usize>,
{
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl<B, R> View<B, R>
where
    B: StableMutBuffer,
    R: RangeBounds<usize>,
{
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.stable_mut_ptr(), self.size()) }
    }
}

impl<B, R> AsMut<[u8]> for View<B, R>
where
    B: StableMutBuffer,
    R: RangeBounds<usize>,
{
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl<B, R> StableBuffer for View<B, R>
where
    B: StableBuffer,
    R: RangeBounds<usize>,
{
    fn stable_ptr(&self) -> *const u8 {
        self.inner.stable_ptr().wrapping_add(self.start())
    }

    fn size(&self) -> usize {
        self.end().saturating_sub(self.start())
    }
}

impl<B, R> StableMutBuffer for View<B, R>
where
    B: StableMutBuffer,
    R: RangeBounds<usize>,
{
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.inner.stable_mut_ptr().wrapping_add(self.start())
    }
}
