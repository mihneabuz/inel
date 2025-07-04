use std::{
    io::Result,
    ops::{Bound, Deref, DerefMut, RangeBounds},
    slice,
};

use inel_interface::Reactor;

use crate::{BufferSlotKey, Cancellation, Ring, RingReactor};

pub trait StableBuffer: Into<Cancellation> {
    fn stable_ptr(&self) -> *const u8;

    fn size(&self) -> usize;

    fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.stable_ptr(), self.size()) }
    }
}

pub trait StableBufferMut: StableBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8;

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.stable_mut_ptr(), self.size()) }
    }
}

pub trait FixedBuffer {
    fn key(&self) -> &BufferSlotKey;
}

impl StableBuffer for &'static str {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableBuffer for &'static [u8] {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl<const N: usize> StableBuffer for Box<[u8; N]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.as_ref().len()
    }
}

impl<const N: usize> StableBufferMut for Box<[u8; N]> {
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

impl StableBufferMut for Box<[u8]> {
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

impl StableBufferMut for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for String {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableBufferMut for String {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

#[derive(Debug)]
pub struct Fixed<R: Reactor<Handle = Ring>> {
    inner: Option<Box<[u8]>>,
    key: BufferSlotKey,
    reactor: R,
}

impl<R> Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    pub fn new(size: usize, reactor: R) -> Result<Self>
    where
        R: Reactor<Handle = Ring>,
    {
        let buffer = vec![0; size].into_boxed_slice();
        Self::register(buffer, reactor)
    }

    pub fn register(mut buffer: Box<[u8]>, mut reactor: R) -> Result<Self>
    where
        R: Reactor<Handle = Ring>,
    {
        let key = reactor.register_buffer(&mut buffer)?;

        Ok(Self {
            inner: Some(buffer),
            key,
            reactor,
        })
    }
}

impl<R> Drop for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    fn drop(&mut self) {
        self.reactor.unregister_buffer(self.key);
    }
}

impl<R> Deref for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<R> DerefMut for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

impl<R> From<Fixed<R>> for Cancellation
where
    R: Reactor<Handle = Ring>,
{
    fn from(mut value: Fixed<R>) -> Self {
        value.inner.take().unwrap().into()
    }
}

impl<R> StableBuffer for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    fn stable_ptr(&self) -> *const u8 {
        self.inner.as_ref().unwrap().stable_ptr()
    }

    fn size(&self) -> usize {
        self.inner.as_ref().unwrap().size()
    }
}

impl<R> StableBufferMut for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.inner.as_mut().unwrap().stable_mut_ptr()
    }
}

impl<R> FixedBuffer for Fixed<R>
where
    R: Reactor<Handle = Ring>,
{
    fn key(&self) -> &BufferSlotKey {
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
    pub fn new(buffer: B, range: R) -> Self {
        Self {
            inner: buffer,
            range,
        }
    }

    pub fn range(&self) -> &R {
        &self.range
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
    B: StableBufferMut,
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
        self.inner().stable_ptr().wrapping_add(self.start())
    }

    fn size(&self) -> usize {
        self.end().saturating_sub(self.start())
    }
}

impl<B, R> StableBufferMut for View<B, R>
where
    B: StableBufferMut,
    R: RangeBounds<usize>,
{
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.inner_mut().stable_mut_ptr().wrapping_add(self.start())
    }
}

impl<B, R> FixedBuffer for View<B, R>
where
    B: FixedBuffer,
    R: RangeBounds<usize>,
{
    fn key(&self) -> &BufferSlotKey {
        self.inner.key()
    }
}
