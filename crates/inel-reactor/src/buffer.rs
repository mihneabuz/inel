use std::{
    io::Result,
    ops::{Bound, RangeBounds},
};

use inel_interface::Reactor;

use crate::{BufferKey, Cancellation, Ring, RingReactor};

pub trait StableBuffer: Into<Cancellation> {
    fn stable_ptr(&self) -> *const u8;
    fn stable_mut_ptr(&mut self) -> *mut u8;

    fn size(&self) -> usize;

    fn view<R>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }
}

pub trait FixedBuffer: StableBuffer {
    fn key(&self) -> &BufferKey;
}

impl<const N: usize> StableBuffer for Box<[u8; N]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn size(&self) -> usize {
        self.as_ref().len()
    }
}

impl StableBuffer for Box<[u8]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableBuffer for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

#[derive(Debug)]
pub struct Fixed<B: StableBuffer, R: Reactor<Handle = Ring>> {
    inner: Option<B>,
    key: BufferKey,
    reactor: R,
}

impl<B, R> Fixed<B, R>
where
    B: StableBuffer,
    R: Reactor<Handle = Ring>,
{
    pub fn register(mut buffer: B, mut reactor: R) -> Result<Self>
    where
        R: Reactor<Handle = Ring>,
    {
        let key = unsafe { reactor.register_buffer(&mut buffer) }?;

        Ok(Self {
            inner: Some(buffer),
            key,
            reactor,
        })
    }

    fn unregister(&mut self) -> Result<()> {
        if let Some(buffer) = self.inner.as_mut() {
            unsafe { self.reactor.unregister_buffer(buffer, self.key) };
        };

        Ok(())
    }

    pub fn inner(&self) -> &B {
        self.inner.as_ref().unwrap()
    }

    pub fn inner_mut(&mut self) -> &mut B {
        self.inner.as_mut().unwrap()
    }

    pub fn into_inner(mut self) -> B {
        let _ = self.unregister();
        self.inner.take().unwrap()
    }
}

impl<B, R> Drop for Fixed<B, R>
where
    B: StableBuffer,
    R: Reactor<Handle = Ring>,
{
    fn drop(&mut self) {
        let _ = self.unregister();
    }
}

impl<B, R> From<Fixed<B, R>> for Cancellation
where
    B: StableBuffer,
    R: Reactor<Handle = Ring>,
{
    fn from(value: Fixed<B, R>) -> Self {
        value.into_inner().into()
    }
}

impl<B, R> StableBuffer for Fixed<B, R>
where
    B: StableBuffer,
    R: Reactor<Handle = Ring>,
{
    fn stable_ptr(&self) -> *const u8 {
        self.inner().stable_ptr()
    }

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.inner_mut().stable_mut_ptr()
    }

    fn size(&self) -> usize {
        self.inner().size()
    }
}

impl<B, R> FixedBuffer for Fixed<B, R>
where
    B: StableBuffer,
    R: Reactor<Handle = Ring>,
{
    fn key(&self) -> &BufferKey {
        &self.key
    }
}

#[derive(Debug)]
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

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.stable_mut_ptr(), self.size()) }
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

impl<B, R> AsMut<[u8]> for View<B, R>
where
    B: StableBuffer,
    R: RangeBounds<usize>,
{
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl<T, R> From<View<T, R>> for Cancellation
where
    T: StableBuffer,
{
    fn from(value: View<T, R>) -> Self {
        value.unview().into()
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

    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.inner.stable_mut_ptr().wrapping_add(self.start())
    }

    fn size(&self) -> usize {
        self.end().saturating_sub(self.start())
    }
}

impl<B, R> FixedBuffer for View<B, R>
where
    B: FixedBuffer,
    R: RangeBounds<usize>,
{
    fn key(&self) -> &BufferKey {
        self.inner.key()
    }
}
