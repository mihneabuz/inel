use core::ptr;

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

    /// # Safety
    /// `drop` must be safe to call with `ptr` and `metadata`, once, after the
    /// op completes.
    pub const unsafe fn from_raw(
        ptr: *mut (),
        metadata: usize,
        drop: unsafe fn(*mut (), usize),
    ) -> Self {
        Self {
            ptr,
            metadata,
            drop: Some(drop),
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
