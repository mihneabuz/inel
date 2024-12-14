/// Delayed memory reclamation
///
/// If an [crate::op::Op] which uses an internal buffer is dropped before
/// the sqe complets, we need to make sure not to drop the buffer,
/// otherwise the kernel might write to freed memory.
///
/// Instead, we turn that buffer into a Cancellation and give it
/// to the [crate::Ring], which will call [Cancellation::drop_raw]
/// when the sqe complets.
pub struct Cancellation {
    ptr: *mut (),
    metadata: usize,
    drop: Option<unsafe fn(*mut (), usize)>,
}

impl Cancellation {
    pub fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            metadata: 0,
            drop: None,
        }
    }

    pub fn combine(cancels: Vec<Self>) -> Self {
        let len = cancels.len();
        Self {
            ptr: Box::into_raw(cancels.into_boxed_slice()) as *mut (),
            metadata: len,
            drop: Some(|ptr, len| unsafe {
                Vec::from_raw_parts(ptr as *mut Cancellation, len, len)
                    .into_iter()
                    .for_each(|cancel| cancel.drop_raw());
            }),
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
        }
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
        }
    }
}

impl From<Box<str>> for Cancellation {
    fn from(value: Box<str>) -> Self {
        value.into_boxed_bytes().into()
    }
}

impl From<String> for Cancellation {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}
