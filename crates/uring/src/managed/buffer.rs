use core::{
    mem::ManuallyDrop,
    ops::{Bound, Deref, DerefMut, RangeBounds},
    ptr::{self, NonNull},
};

use crate::managed::cancellation::Cancellation;

/// A buffer that can be converted to/from a raw form.
///
/// Because of the nature of io_uring, futures cannot safely borrow buffers,
/// since dropping the future cannot synchronously cancel the associated op
/// (at least not in an efficient manner), which can lead to a situation where
/// the future is canceled, the buffer is dropped, but the kernel still has
/// a pending op with a pointer into that buffer.
///
/// This means the only way to make a safe API is to take buffers by ownership
/// and ensure they are not dropped until the op is completed - this is
/// handeld by the [Cancellation] returned by [raw_cancel].
///
/// Another soundness problem is pointer aliasing. While an op is in flight,
/// the buffer targeted by it can be accessed by the kernel, which breaks
/// Rust's aliasing rules. To avoid this problem, we store these buffers in
/// raw form, by converting them when the op is created, then restoring the
/// original type once the op is done, or transforming into a [Cancellation]
/// when the op is cancelled.
///
/// # Safety
/// - `from_raw(into_raw(buf))` must give back `buf`.
/// - `raw_cancel` must not access the buffer's memory.
///
pub unsafe trait RawBuf: Sized {
    /// The raw type that the buffer will have while the op is in flight.
    ///
    /// # Safety
    /// Must not contain a pointer type with strict aliasing like `Box<T>`.
    type Raw;

    /// Tansform `self` into it's `Self::Raw` form.
    fn into_raw(self) -> Self::Raw;

    /// Transform `raw` back into it's `Self` form.
    ///
    /// # Safety
    /// `raw` must come from [IoBuf::into_raw], and the kernel must be done
    /// with the buffer.
    unsafe fn from_raw(raw: Self::Raw) -> Self;

    /// Transform `raw` back into it's `Self` form.
    fn raw_cancel(raw: Self::Raw) -> Cancellation;
}

unsafe impl<T> RawBuf for Box<[T]> {
    type Raw = NonNull<[T]>;

    fn into_raw(self) -> Self::Raw {
        Self::into_non_null(self)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        // SAFETY: `raw` comes from a valid `Box<[T]>`.
        unsafe { Self::from_raw(raw.as_ptr()) }
    }

    fn raw_cancel(raw: Self::Raw) -> Cancellation {
        unsafe { Cancellation::from_assume_boxed_slice(raw.as_ptr()) }
    }
}

unsafe impl RawBuf for &'static [u8] {
    type Raw = Self;

    fn into_raw(self) -> Self::Raw {
        self
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        raw
    }

    fn raw_cancel(_raw: Self::Raw) -> Cancellation {
        Cancellation::default()
    }
}

unsafe impl RawBuf for &'static str {
    type Raw = Self;

    fn into_raw(self) -> Self::Raw {
        self
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        raw
    }

    fn raw_cancel(_raw: Self::Raw) -> Cancellation {
        Cancellation::default()
    }
}

/// Raw parts of a `Vec<u8>` which can also be used for `Vec`-like buffers.
pub struct RawVec {
    ptr: NonNull<u8>,
    len: usize,
    cap: usize,
}

impl RawVec {
    fn new(vec: Vec<u8>) -> Self {
        let mut vec = ManuallyDrop::new(vec);
        Self {
            ptr: unsafe { NonNull::new_unchecked(vec.as_mut_ptr()) },
            len: vec.len(),
            cap: vec.capacity(),
        }
    }

    fn into_vec(self) -> Vec<u8> {
        // SAFETY: `self` comes from a valid `Vec<u8>`.
        unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), self.len, self.cap) }
    }

    const fn into_cancel(self) -> Cancellation {
        // SAFETY: `self` comes from a valid `Vec<u8>`.
        unsafe { Cancellation::from_assume_vec(self.ptr.as_ptr(), self.cap) }
    }
}

unsafe impl RawBuf for Vec<u8> {
    type Raw = RawVec;

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        raw.into_vec()
    }

    fn raw_cancel(raw: Self::Raw) -> Cancellation {
        raw.into_cancel()
    }
}

unsafe impl RawBuf for String {
    type Raw = RawVec;

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self.into_bytes())
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        // SAFETY: the bytes came from a `String` and were not written to
        unsafe { Self::from_utf8_unchecked(raw.into_vec()) }
    }

    fn raw_cancel(raw: Self::Raw) -> Cancellation {
        raw.into_cancel()
    }
}

/// A buffer that can be used for write ops.
///
/// This trait provides immutable access to the bytes of the buffers.
///
/// Only types which implement [IoBuf] can be safely used for ops which read
/// from their bytes, see [RawBuf] for more details.
///
/// # Safety
/// `raw_buf` must return an the same value even if `raw` was moved.
/// It must also point to a initialized memory block which is valid for reads.
///
pub unsafe trait IoBuf: RawBuf {
    /// The pointer and size of the raw form.
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize);

    /// Restricts the buffer to `range` of its initialized bytes.
    fn view<R: RangeBounds<usize>>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }
}

/// A buffer that can be used for read ops.
///
/// The mutable extension to [IoBuf] which can be used to get mutable access
/// to the bytes of the buffer.
///
/// Only types which implement [IoBufMut] can be safely used for ops which
/// write into their bytes, see [RawBuf] for more details.
///
/// For buffers which also hold uninitialized data, the [set_init] method can
/// be implemented to allow the op to update the buffer with the amount of
/// bytes it has initialized.
///
/// # Safety
/// `raw_buf_mut` must return an the same value even if `raw` was moved.
/// It must also point to a initialized memory block which is valid for writes.
/// `set_init` must leave a `raw` in a valid state, such that it is also valid
/// when converted back into T.
///
pub unsafe trait IoBufMut: RawBuf {
    /// The mutable pointer and size of the raw form.
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize);

    /// The op initialized `n` bytes starting at `raw_buf_mut`.
    ///
    /// # Safety
    /// `n` must not exceed the length returned by `raw_buf_mut`, and those
    /// bytes must have been written.
    #[allow(unused_variables)]
    unsafe fn set_init(raw: &mut Self::Raw, n: usize) {}
}

unsafe impl IoBuf for &'static [u8] {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.as_ptr(), raw.len())
    }
}

unsafe impl IoBuf for &'static str {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.as_ptr(), raw.len())
    }
}

/// Reads fill the whole slice, from the start.
unsafe impl IoBuf for Box<[u8]> {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.cast().as_ptr(), raw.len())
    }
}

unsafe impl IoBufMut for Box<[u8]> {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        (raw.cast().as_ptr(), raw.len())
    }
}

/// Writes send `[0..len)`. Reads fill the whole capacity from the start and
/// set `len` to the number of bytes read, so the `Vec` holds exactly what was
/// read (previous contents are discarded). To append instead, use
/// [Accumulating].
unsafe impl IoBuf for Vec<u8> {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }
}

unsafe impl IoBufMut for Vec<u8> {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        (raw.ptr.as_ptr(), raw.cap)
    }

    unsafe fn set_init(raw: &mut Self::Raw, n: usize) {
        raw.len = n;
    }
}

/// Only for writes, reads could produce invalid UTF-8.
unsafe impl IoBuf for String {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }
}

/// A range of a buffer's initialized bytes.
///
/// Writes send the range, reads overwrite it in place (so a `Vec` keeps its
/// length, unlike reading into the `Vec` directly). A range that goes past the
/// end of the buffer is clamped to it.
pub struct View<B, R> {
    buf: B,
    range: R,
}

impl<B, R> View<B, R> {
    pub const fn new(buf: B, range: R) -> Self {
        Self { buf, range }
    }
}

/// Resolves `range` against a region of `len` bytes, clamped to `0..=len`
/// (`begin <= end <= len` always holds).
fn bounds<R: RangeBounds<usize>>(range: &R, len: usize) -> (usize, usize) {
    let begin = match range.start_bound() {
        Bound::Included(&x) => x,
        Bound::Excluded(&x) => x.saturating_add(1),
        Bound::Unbounded => 0,
    }
    .min(len);

    let end = match range.end_bound() {
        Bound::Included(&x) => x.saturating_add(1),
        Bound::Excluded(&x) => x,
        Bound::Unbounded => len,
    }
    .clamp(begin, len);

    (begin, end)
}

/// Applies the range of a [`View`] to the region `(ptr, len)` of its buffer.
///
/// # Safety
/// `ptr` must be valid for `len` bytes (as an in-bounds or one-past pointer).
unsafe fn window<R: RangeBounds<usize>>(range: &R, ptr: *mut u8, len: usize) -> (*mut u8, usize) {
    let (begin, end) = bounds(range, len);
    // SAFETY: `begin <= len`, so the pointer stays in (or one past) the region.
    (unsafe { ptr.add(begin) }, end - begin)
}

impl<B, R: RangeBounds<usize>> View<B, R> {
    /// The start of the range, before clamping to the buffer.
    pub fn begin(&self) -> usize {
        match self.range.start_bound() {
            Bound::Included(&x) => x,
            Bound::Excluded(&x) => x.saturating_add(1),
            Bound::Unbounded => 0,
        }
    }

    /// The end of the range, before clamping to the buffer (`None` if unbounded).
    pub fn end(&self) -> Option<usize> {
        match self.range.end_bound() {
            Bound::Included(&x) => Some(x.saturating_add(1)),
            Bound::Excluded(&x) => Some(x),
            Bound::Unbounded => None,
        }
    }

    pub const fn range(&self) -> &R {
        &self.range
    }

    pub const fn inner(&self) -> &B {
        &self.buf
    }

    pub fn into_inner(self) -> B {
        self.buf
    }
}

pub struct RawView<B, R> {
    raw: B,
    range: R,
}

unsafe impl<B: RawBuf, R> RawBuf for View<B, R> {
    type Raw = RawView<B::Raw, R>;

    fn into_raw(self) -> Self::Raw {
        RawView {
            raw: self.buf.into_raw(),
            range: self.range,
        }
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        Self {
            buf: unsafe { B::from_raw(raw.raw) },
            range: raw.range,
        }
    }

    fn raw_cancel(raw: Self::Raw) -> Cancellation {
        B::raw_cancel(raw.raw)
    }
}

unsafe impl<B: IoBuf, R: RangeBounds<usize>> IoBuf for View<B, R> {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        let (ptr, len) = B::raw_buf(&raw.raw);
        // SAFETY: `(ptr, len)` is a valid region of the buffer.
        let (ptr, len) = unsafe { window(&raw.range, ptr.cast_mut(), len) };
        (ptr.cast_const(), len)
    }
}

/// Reads overwrite the initialized bytes in place, `set_init` is a no-op, so
/// the length of the inner buffer never changes.
unsafe impl<B: IoBufMut, R: RangeBounds<usize>> IoBufMut for View<B, R> {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        let (ptr, len) = B::raw_buf_mut(&raw.raw);
        // SAFETY: `(ptr, len)` is a valid region of the buffer.
        let (ptr, len) = unsafe { window(&raw.range, ptr, len) };
        (ptr, len)
    }
}

/// A `Vec` that reads append to: they fill the spare capacity `[len..cap)` and
/// grow `len` by the number of bytes read. Writes send `[0..len)`.
///
/// Reserve capacity before reading, a full `Vec` reads 0 bytes, which looks
/// like EOF.
pub struct Accumulating(Vec<u8>);

impl Accumulating {
    pub const fn new(vec: Vec<u8>) -> Self {
        Self(vec)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for Accumulating {
    fn from(vec: Vec<u8>) -> Self {
        Self(vec)
    }
}

impl Deref for Accumulating {
    type Target = Vec<u8>;

    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl DerefMut for Accumulating {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

unsafe impl RawBuf for Accumulating {
    type Raw = RawVec;

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self.0)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        Self(raw.into_vec())
    }

    fn raw_cancel(raw: Self::Raw) -> Cancellation {
        raw.into_cancel()
    }
}

unsafe impl IoBuf for Accumulating {
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }
}

unsafe impl IoBufMut for Accumulating {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        // SAFETY: `len <= cap`, so the pointer stays in (or one past) the buffer
        (unsafe { raw.ptr.as_ptr().add(raw.len) }, raw.cap - raw.len)
    }

    unsafe fn set_init(raw: &mut Self::Raw, n: usize) {
        raw.len += n;
    }
}

/// Owns a buffer that may be shared with the kernel, in its raw form.
///
/// This is a helper type for handling the conversion between the normal
/// and raw form for [IoBuf] and [IoBufMut] buffers. Any ops which pass
/// buffers into the kernel should use this type to ensure safety.
///
pub struct InFlight<B: RawBuf> {
    raw: B::Raw,
}

impl<B: RawBuf> InFlight<B> {
    pub fn new(buf: B) -> Self {
        Self {
            raw: buf.into_raw(),
        }
    }

    fn into_raw(self) -> B::Raw {
        let this = ManuallyDrop::new(self);
        unsafe { ptr::read(&this.raw) }
    }

    /// # Safety
    /// The kernel must be done with the buffer.
    pub unsafe fn into_inner(self) -> B {
        unsafe { B::from_raw(self.into_raw()) }
    }

    pub fn into_cancel(self) -> Cancellation {
        B::raw_cancel(self.into_raw())
    }
}

impl<B: IoBuf> InFlight<B> {
    pub fn raw_buf(&self) -> (*const u8, usize) {
        B::raw_buf(&self.raw)
    }
}

impl<B: IoBufMut> InFlight<B> {
    pub fn raw_buf_mut(&self) -> (*mut u8, usize) {
        B::raw_buf_mut(&self.raw)
    }

    /// Rebuilds the buffer after the op wrote `n` bytes into it.
    ///
    /// # Safety
    /// The kernel must be done with the buffer, and must have written `n` bytes
    /// starting at `raw_buf_mut`.
    pub unsafe fn into_inner_init(self, n: usize) -> B {
        unsafe {
            let mut raw = self.into_raw();
            B::set_init(&mut raw, n);
            B::from_raw(raw)
        }
    }
}

impl<B: RawBuf> Drop for InFlight<B> {
    fn drop(&mut self) {
        // only reached for an op that was never submitted: every other path
        // goes through `into_raw` or the `Cancellation`
        drop(unsafe { B::from_raw(ptr::read(&self.raw)) });
    }
}
