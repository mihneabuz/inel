use std::{ffi::CString, rc::Rc};

use crate::ring::RingResult;

/// A cancellation handle used to safely manage I/O operations.
///
/// When an [crate::op::Op] (wrapped in a [crate::submission::Submission]) is dropped or cancelled
/// before its completion, it is important to ensure that any underlying resources (such as buffers)
/// remain valid until the kernel completes the operation. The [`Cancellation`] type captures the
/// resource (or resources) and the associated cleanup or consumption logic that must be invoked
/// once the operation is complete.
///
/// There are two primary scenarios in which a cancellation is needed:
///
/// 1. **Buffer Ownership:**
///    When an [crate::op::Op] owns a memory buffer and that operation is dropped before completion,
///    the buffer must not be deallocated immediately. Instead, the buffer is transformed into a
///    [`Cancellation`] such that its cleanup is deferred until the kernel finishes the operation.
///
/// 2. **Completion Handling:**
///    Some operations require that specific code (e.g., error handling, resource finalization) is
///    executed even if the corresponding [crate::submission::Submission] is dropped. In these cases,
///    a [`Cancellation`] can be used to "consume" the result via a user-provided callback.
///
/// The underlying implementation stores a raw pointer to the resource, along with metadata (typically
/// representing resource length or count), and function pointers for cleanup (`drop`) and result
/// consumption (`consume`).
pub struct Cancellation {
    /// Raw pointer to the underlying resource. The concrete type is erased.
    ptr: *mut (),

    /// Metadata that may indicate resource length or other type-specific information.
    metadata: usize,

    /// An optional cleanup function to run when the operation is completed. This should
    /// safely recover the original resource from `ptr` and clean it up.
    drop: Option<unsafe fn(*mut (), usize)>,

    /// An optional function to "consume" the result of the cancelled operation by passing
    /// the [RingResult] back to the owned resource.
    consume: Option<unsafe fn(*mut (), RingResult)>,
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::empty()
    }
}

/// Creates a [`Cancellation`] from a reference-counted resource (`Rc<T>`) and a callback function
/// to be invoked to consume a [RingResult].
///
/// This macro is designed to safely generate `Cancellation` instances for types that:
/// - Are stored in an `Rc<T>`,
/// - Require deferred handling of the completion results, even when cancelled
///
/// It wraps the unsafe logic of pointer conversion and callback setup into a reusable and
/// safe interface.
///
/// You should prefer using this macro over calling `Cancellation::consuming` directly.
macro_rules! consuming {
    ($ty:ty, $value:expr, $consume:expr) => {{
        unsafe {
            Cancellation::consuming($value, |ptr, result: RingResult| {
                let consume: fn(&$ty, RingResult) = $consume;
                let value = ptr as *const $ty;
                consume(&*value, result);
            })
        }
    }};
}

pub(crate) use consuming;

impl Cancellation {
    /// Creates an empty [`Cancellation`] that performs no cleanup.
    pub fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            metadata: 0,
            drop: None,
            consume: None,
        }
    }

    /// Creates a [`Cancellation`] from an `Rc<T>` value along with a custom consume callback.
    ///
    /// # Safety
    ///
    /// This method is unsafe because it transfers ownership of the `Rc<T>` by converting it into
    /// a raw pointer. Users should prefer using the [`consuming!`] macro to avoid manually writing
    /// unsafe code.
    pub unsafe fn consuming<T>(value: Rc<T>, consume: unsafe fn(*mut (), RingResult)) -> Self {
        Self {
            ptr: Rc::into_raw(value) as *mut _,
            metadata: 0,
            drop: Some(|ptr, _| unsafe {
                let _ = Rc::from_raw(ptr as *const T);
            }),
            consume: Some(consume),
        }
    }

    /// Combines multiple [`Cancellation`] objects into a single one.
    ///
    /// Note: the returned [Cancellation] will ignore all consume callbacks
    pub fn combine(cancels: Vec<Self>) -> Self {
        let len = cancels.len();
        let ptr = cancels.as_ptr() as *mut ();
        std::mem::forget(cancels);
        Self {
            ptr,
            metadata: len,
            drop: Some(|ptr, len| unsafe {
                Vec::from_raw_parts(ptr as *mut Cancellation, len, len)
                    .into_iter()
                    .for_each(|cancel| cancel.drop_raw());
            }),
            consume: None,
        }
    }

    pub fn consume(&self, result: RingResult) {
        if let Some(consume) = self.consume {
            unsafe { (consume)(self.ptr, result) }
        }
    }

    pub fn drop_raw(self) {
        if let Some(drop) = self.drop {
            unsafe { (drop)(self.ptr, self.metadata) }
        }
    }
}

impl<T> From<Box<T>> for Cancellation {
    fn from(value: Box<T>) -> Self {
        Self {
            ptr: Box::into_raw(value) as *mut (),
            metadata: 0,
            drop: Some(|ptr, _| unsafe {
                drop(Box::from_raw(ptr));
            }),
            consume: None,
        }
    }
}

impl<T> From<Option<Box<T>>> for Cancellation {
    fn from(value: Option<Box<T>>) -> Self {
        value.map(|value| value.into()).unwrap_or_default()
    }
}

impl<T> From<Box<[T]>> for Cancellation {
    fn from(value: Box<[T]>) -> Self {
        let len = value.len();
        Self {
            ptr: Box::into_raw(value) as *mut (),
            metadata: len,
            drop: Some(|ptr, len| unsafe {
                drop(Vec::from_raw_parts(ptr as *mut T, len, len));
            }),
            consume: None,
        }
    }
}

impl<T> From<Option<Box<[T]>>> for Cancellation {
    fn from(value: Option<Box<[T]>>) -> Self {
        value.map(|value| value.into()).unwrap_or_default()
    }
}

impl<T> From<Vec<T>> for Cancellation {
    fn from(value: Vec<T>) -> Self {
        let len = value.len();
        let ptr = value.as_ptr() as *mut ();
        std::mem::forget(value);
        Self {
            ptr,
            metadata: len,
            drop: Some(|ptr, len| unsafe {
                drop(Vec::from_raw_parts(ptr as *mut T, len, len));
            }),
            consume: None,
        }
    }
}

impl From<String> for Cancellation {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}

impl From<CString> for Cancellation {
    fn from(value: CString) -> Self {
        value.into_bytes().into()
    }
}

impl From<&'static [u8]> for Cancellation {
    fn from(_: &'static [u8]) -> Self {
        Self::empty()
    }
}

impl From<&'static str> for Cancellation {
    fn from(_: &'static str) -> Self {
        Self::empty()
    }
}
