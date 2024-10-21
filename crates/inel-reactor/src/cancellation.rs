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
                drop(Vec::from_raw_parts(ptr as *mut T, len, len).into_boxed_slice());
            }),
        }
    }
}

impl<T> From<Vec<T>> for Cancellation {
    fn from(value: Vec<T>) -> Self {
        Cancellation::from(value.into_boxed_slice())
    }
}
