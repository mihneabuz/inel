use core::{
    mem::ManuallyDrop,
    ops::{Bound, Deref, DerefMut, RangeBounds},
    ptr::{self, NonNull},
};

use crate::managed::cancellation::Cancellation;

/// A buffer that can be handed to the kernel.
///
/// While an op is in flight, the buffer targeted by it can be accessed by the
/// kernel, which breaks Rust's aliasing rules. To avoid this problem, we store
/// these buffers in raw form, by converting them when the op is created, then
/// restoring the original type once the op is done, or transforming into a
/// [Cancellation] when the op is cancelled.
///
/// The kernel only reads from an [IoBuf], so on its own it should only be used
/// for write operations (`write`, `send`, ...). Operations where the kernel
/// writes into the buffer (`read`, `recv`, ...) need an [IoBufMut].
///
/// # Safety
/// - `from_raw(into_raw(buf))` must give back `buf`
/// - the memory described by `raw_buf` (and `raw_buf_mut`) must stay valid, at
///   the same address, for as long as the buffer is in raw form
/// - `cancellation` must free the buffer, and not access it before it is
///   dropped
pub unsafe trait IoBuf: Sized {
    type Raw;

    fn size(&self) -> usize;

    fn into_raw(self) -> Self::Raw;

    /// # Safety
    /// `raw` must come from [IoBuf::into_raw], and the kernel must be done
    /// with the buffer.
    unsafe fn from_raw(raw: Self::Raw) -> Self;

    /// Pointer to the initialized bytes and their length, for writes.
    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize);

    fn cancellation(raw: Self::Raw) -> Cancellation;

    /// Restricts the buffer to `range` of its initialized bytes.
    fn view<R: RangeBounds<usize>>(self, range: R) -> View<Self, R> {
        View::new(self, range)
    }
}

/// A buffer the kernel can write into.
///
/// The mutable extension to [IoBuf] which can be used for read operations.
///
/// # Safety
/// Same as [IoBuf], and the memory returned by both `raw_buf_mut` and
/// `raw_buf` must be valid for writes.
pub unsafe trait IoBufMut: IoBuf {
    /// Pointer to the writable bytes and their length, for reads.
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize);

    /// The kernel initialized `n` bytes starting at `raw_buf_mut`.
    ///
    /// # Safety
    /// `n` must not exceed the length returned by `raw_buf_mut`, and those
    /// bytes must have been written.
    unsafe fn set_init(raw: &mut Self::Raw, n: usize);
}

/// Owns a buffer that may be shared with the kernel, in its raw form.
pub struct InFlight<B: IoBuf> {
    raw: B::Raw,
}

impl<B: IoBuf> InFlight<B> {
    pub fn new(buf: B) -> Self {
        Self {
            raw: buf.into_raw(),
        }
    }

    pub fn raw_buf(&self) -> (*const u8, usize) {
        B::raw_buf(&self.raw)
    }

    /// # Safety
    /// The kernel must be done with the buffer (final CQE seen).
    pub unsafe fn into_raw(self) -> B::Raw {
        let this = ManuallyDrop::new(self);
        unsafe { ptr::read(&this.raw) }
    }

    /// # Safety
    /// The kernel must be done with the buffer (final CQE seen).
    pub unsafe fn into_inner(self) -> B {
        unsafe { B::from_raw(self.into_raw()) }
    }
}

impl<B: IoBufMut> InFlight<B> {
    pub fn raw_buf_mut(&self) -> (*mut u8, usize) {
        B::raw_buf_mut(&self.raw)
    }

    /// Rebuilds the buffer after the kernel wrote `n` bytes into it.
    ///
    /// # Safety
    /// The kernel must be done with the buffer (final CQE seen), and must
    /// have written `n` bytes starting at `raw_buf_mut`.
    pub unsafe fn into_inner_init(self, n: usize) -> B {
        unsafe {
            let mut raw = self.into_raw();
            B::set_init(&mut raw, n);
            B::from_raw(raw)
        }
    }
}

impl<B: IoBuf> Drop for InFlight<B> {
    fn drop(&mut self) {
        // only reached for an op that was never submitted: every other path
        // goes through `into_raw` or the `Cancellation`
        drop(unsafe { B::from_raw(ptr::read(&self.raw)) });
    }
}

impl<B: IoBuf> From<InFlight<B>> for Cancellation {
    fn from(buf: InFlight<B>) -> Self {
        // the buffer stays in raw form, it is only rebuilt (and dropped)
        // after the final CQE
        let this = ManuallyDrop::new(buf);
        B::cancellation(unsafe { ptr::read(&this.raw) })
    }
}

unsafe impl IoBuf for &'static [u8] {
    type Raw = Self;

    fn size(&self) -> usize {
        self.len()
    }

    fn into_raw(self) -> Self::Raw {
        self
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        raw
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.as_ptr(), raw.len())
    }

    fn cancellation(_: Self::Raw) -> Cancellation {
        Cancellation::empty()
    }
}

unsafe impl IoBuf for &'static str {
    type Raw = Self;

    fn size(&self) -> usize {
        self.len()
    }

    fn into_raw(self) -> Self::Raw {
        self
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        raw
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.as_ptr(), raw.len())
    }

    fn cancellation(_: Self::Raw) -> Cancellation {
        Cancellation::empty()
    }
}

/// Reads fill the whole slice, from the start.
// `Self::from_raw` would read like a recursive call to `IoBuf::from_raw`
unsafe impl IoBuf for Box<[u8]> {
    type Raw = NonNull<[u8]>;

    fn size(&self) -> usize {
        self.len()
    }

    fn into_raw(self) -> Self::Raw {
        Self::into_non_null(self)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        unsafe { Self::from_raw(raw.as_ptr()) }
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.cast().as_ptr(), raw.len())
    }

    fn cancellation(raw: Self::Raw) -> Cancellation {
        unsafe {
            Cancellation::from_raw(raw.cast().as_ptr(), raw.len(), |ptr, len| {
                drop(Self::from_raw(ptr::slice_from_raw_parts_mut(
                    ptr.cast::<u8>(),
                    len,
                )));
            })
        }
    }
}

unsafe impl IoBufMut for Box<[u8]> {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        (raw.cast().as_ptr(), raw.len())
    }

    unsafe fn set_init(_: &mut Self::Raw, _: usize) {}
}

/// Raw parts of a `Vec<u8>`.
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

    unsafe fn into_vec(self) -> Vec<u8> {
        unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), self.len, self.cap) }
    }

    fn cancellation(self) -> Cancellation {
        // freeing only needs the capacity, `u8` has no drop
        unsafe {
            Cancellation::from_raw(self.ptr.as_ptr().cast(), self.cap, |ptr, cap| {
                drop(Vec::from_raw_parts(ptr.cast::<u8>(), 0, cap));
            })
        }
    }
}

/// Writes send `[0..len)`. Reads fill the whole capacity from the start and
/// set `len` to the number of bytes read, so the `Vec` holds exactly what was
/// read (previous contents are discarded). To append instead, use
/// [Accumulating].
unsafe impl IoBuf for Vec<u8> {
    type Raw = RawVec;

    fn size(&self) -> usize {
        self.len()
    }

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        unsafe { raw.into_vec() }
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }

    fn cancellation(raw: Self::Raw) -> Cancellation {
        raw.cancellation()
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
    type Raw = RawVec;

    fn size(&self) -> usize {
        self.len()
    }

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self.into_bytes())
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        // SAFETY: the bytes came from a `String` and were not written to
        unsafe { Self::from_utf8_unchecked(raw.into_vec()) }
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }

    fn cancellation(raw: Self::Raw) -> Cancellation {
        raw.cancellation()
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

/// Resolves `range` against a buffer of `len` bytes, clamped to `0..=len`.
fn bounds(range: &impl RangeBounds<usize>, len: usize) -> (usize, usize) {
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

impl<B: IoBuf, R: RangeBounds<usize>> View<B, R> {
    pub const fn new(buf: B, range: R) -> Self {
        Self { buf, range }
    }

    pub fn begin(&self) -> usize {
        bounds(&self.range, self.buf.size()).0
    }

    pub fn end(&self) -> usize {
        bounds(&self.range, self.buf.size()).1
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

unsafe impl<B: IoBuf, R: RangeBounds<usize>> IoBuf for View<B, R> {
    type Raw = RawView<B::Raw, R>;

    fn size(&self) -> usize {
        self.end() - self.begin()
    }

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

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        let (ptr, len) = B::raw_buf(&raw.raw);
        let (begin, end) = bounds(&raw.range, len);
        // SAFETY: `begin <= len`, so the pointer stays in (or one past) the buffer
        (unsafe { ptr.add(begin) }, end - begin)
    }

    fn cancellation(raw: Self::Raw) -> Cancellation {
        B::cancellation(raw.raw)
    }
}

unsafe impl<B: IoBufMut, R: RangeBounds<usize>> IoBufMut for View<B, R> {
    fn raw_buf_mut(raw: &Self::Raw) -> (*mut u8, usize) {
        // `B: IoBufMut`, so its initialized bytes are writable too
        let (ptr, len) = Self::raw_buf(raw);
        (ptr.cast_mut(), len)
    }

    // the range was already initialized, nothing to record
    unsafe fn set_init(_: &mut Self::Raw, _: usize) {}
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

unsafe impl IoBuf for Accumulating {
    type Raw = RawVec;

    fn size(&self) -> usize {
        self.0.len()
    }

    fn into_raw(self) -> Self::Raw {
        RawVec::new(self.0)
    }

    unsafe fn from_raw(raw: Self::Raw) -> Self {
        Self(unsafe { raw.into_vec() })
    }

    fn raw_buf(raw: &Self::Raw) -> (*const u8, usize) {
        (raw.ptr.as_ptr(), raw.len)
    }

    fn cancellation(raw: Self::Raw) -> Cancellation {
        raw.cancellation()
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
