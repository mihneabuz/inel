use core::ptr;

use std::{ffi::CString, rc::Rc};

/// Type-erased handle for drop.
///
/// Dropping a future can cancel an in flight op which the kernel has already
/// seen, so we must not drop any buffers until the kernel is done with them.
///
/// To achieve this, instead of dropping them along with the future, we turn
/// them into a [Cancellation] and pass it to the kernel, which will then
/// call [Cancellation::drop_raw] once associated op is completed.
///
/// [Cancellation] intentionally has a single `usize` field for metadata,
/// meaning it can properly handle any buffers represented by a (ptr, len)
/// pair. But for something like a [Vec<T>], which also has a capacity and
/// uninitialized items, we lose some information in the conversion, so we
/// can only drop the memory but not drop any of the items.
/// This is fine in practice since most [Vec<T>] buffers we are dealing with
/// have some trivial `T: Copy`.
///
/// This struct layout keeps the size small - since this is technically the
/// "slow" path, we don't want to bloat the completion entries with it's size.
#[derive(Debug)]
pub struct Cancellation {
    ptr: *mut (),
    metadata: usize,
    drop: Option<unsafe fn(*mut (), usize)>,
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::empty()
    }
}

impl Cancellation {
    /// Create a no-op cancellation token.
    pub fn empty() -> Self {
        Self {
            ptr: ptr::null_mut(),
            metadata: 0,
            drop: None,
        }
    }

    /// Turn a raw `Box<T>` into a cancellation token.
    ///
    /// # Safety
    /// `ptr` must come from a valid `Box<T>`.
    pub const unsafe fn from_assume_boxed<T>(ptr: *mut T) -> Self {
        Self {
            ptr: ptr as *mut _,
            metadata: 0,
            drop: Some(|ptr, _| unsafe { drop(Box::from_raw(ptr as *mut T)) }),
        }
    }

    /// Turn a raw `Box<[T]>` into a cancellation token.
    ///
    /// # Safety
    /// `ptr` must come from a valid `Box<[T]>`.
    pub const unsafe fn from_assume_boxed_slice<T>(ptr: *mut [T]) -> Self {
        Self {
            ptr: ptr as *mut _,
            metadata: ptr.len(),
            drop: Some(|ptr, len| unsafe {
                drop(Box::from_raw(ptr::slice_from_raw_parts_mut(
                    ptr as *mut T,
                    len,
                )))
            }),
        }
    }

    /// Turn a raw `Vec<T>` into a cancellation token.
    /// This only drops the memory, but does *not* drop any `T`s.
    ///
    /// # Safety
    /// `ptr` and `capacity` must come from a valid `Vec<T>`.
    pub const unsafe fn from_assume_vec<T>(ptr: *mut T, capacity: usize) -> Self {
        Self {
            ptr: ptr as *mut _,
            metadata: capacity,
            drop: Some(|ptr, cap| unsafe { drop(Vec::from_raw_parts(ptr as *mut T, 0, cap)) }),
        }
    }

    /// Turn a raw `Rc<T>` into a cancellation token.
    ///
    /// # Safety
    /// `ptr` must come from a valid `Rc<T>`.
    pub const unsafe fn from_assume_rc<T>(ptr: *const T) -> Self {
        Self {
            ptr: ptr as *mut _,
            metadata: 0,
            drop: Some(|ptr, _| unsafe { drop(Rc::from_raw(ptr as *const T)) }),
        }
    }

    /// Turn a raw `Rc<[T]>` into a cancellation token.
    ///
    /// # Safety
    /// `ptr` must come from a valid `Rc<[T]>`.
    pub const unsafe fn from_assume_rc_slice<T>(ptr: *const [T]) -> Self {
        Self {
            ptr: ptr as *mut _,
            metadata: ptr.len(),
            drop: Some(|ptr, len| unsafe {
                drop(Rc::from_raw(ptr::slice_from_raw_parts(
                    ptr as *const T,
                    len,
                )))
            }),
        }
    }

    pub fn drop_raw(self) {
        unsafe { self.drop_by_ref() };
    }

    /// # Safety
    /// Same as [`Self::from_raw`]: the op must have completed, and the handle
    /// must not be dropped again afterwards.
    pub unsafe fn drop_by_ref(&self) {
        if let Some(drop) = self.drop {
            unsafe { (drop)(self.ptr, self.metadata) }
        }
    }
}

impl<T: Into<Self>> From<Option<T>> for Cancellation {
    fn from(value: Option<T>) -> Self {
        value.map(|inner| inner.into()).unwrap_or_default()
    }
}

impl<T> From<Box<T>> for Cancellation {
    fn from(value: Box<T>) -> Self {
        // SAFETY: value is a valid `Box<T>`.
        unsafe { Self::from_assume_boxed(Box::into_raw(value)) }
    }
}

impl<T> From<Box<[T]>> for Cancellation {
    fn from(value: Box<[T]>) -> Self {
        // SAFETY: value is a valid `Box<[T]>`.
        unsafe { Self::from_assume_boxed_slice(Box::into_raw(value)) }
    }
}

impl From<Box<str>> for Cancellation {
    fn from(value: Box<str>) -> Self {
        // SAFETY: value is a valid `Box<str>`.
        unsafe { Self::from_assume_boxed_slice(Box::into_raw(value.into_boxed_bytes())) }
    }
}

impl<T: Copy> From<Vec<T>> for Cancellation {
    fn from(value: Vec<T>) -> Self {
        let (ptr, _, cap) = value.into_raw_parts();
        // SAFETY: value is a valid `Vec<T>`, and because `T: Copy` it does not need drop.
        unsafe { Self::from_assume_vec(ptr, cap) }
    }
}

impl From<String> for Cancellation {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}

impl From<CString> for Cancellation {
    fn from(value: CString) -> Self {
        value.into_bytes_with_nul().into()
    }
}

impl<T> From<Rc<T>> for Cancellation {
    fn from(value: Rc<T>) -> Self {
        // SAFETY: value is a valid `Rc<T>`.
        unsafe { Self::from_assume_rc(Rc::into_raw(value)) }
    }
}

impl<T> From<Rc<[T]>> for Cancellation {
    fn from(value: Rc<[T]>) -> Self {
        // SAFETY: value is a valid `Rc<[T]>`.
        unsafe { Self::from_assume_rc_slice(Rc::into_raw(value)) }
    }
}

impl From<Rc<str>> for Cancellation {
    fn from(value: Rc<str>) -> Self {
        // SAFETY: value is a valid `Rc<str>`.
        unsafe { Self::from_assume_rc_slice(Rc::into_raw(value) as *const [u8]) }
    }
}
