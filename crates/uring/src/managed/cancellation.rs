use core::ptr;

use rustix::ffi::CString;

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
    pub fn empty() -> Self {
        Self {
            ptr: ptr::null_mut(),
            metadata: 0,
            drop: None,
        }
    }

    pub fn drop_raw(self) {
        unsafe { self.drop_by_ref() };
    }

    pub unsafe fn drop_by_ref(&self) {
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
                drop(Box::from_raw(ptr as *mut T));
            }),
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
        value.into_boxed_slice().into()
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
