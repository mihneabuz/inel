pub struct Cancellation {
    data: *mut (),
    metadata: usize,
    drop: unsafe fn(*mut (), usize),
}

impl Cancellation {
    pub fn empty() -> Self {
        Self {
            data: std::ptr::null_mut(),
            metadata: 0,
            drop: |_, _| {},
        }
    }

    pub fn combine(cancels: Vec<Self>) -> Self {
        let len = cancels.len();
        Self {
            data: Box::into_raw(cancels.into_boxed_slice()) as *mut (),
            metadata: len,
            drop: |data, len| unsafe {
                Vec::from_raw_parts(data as *mut Cancellation, len, len)
                    .into_iter()
                    .for_each(|cancel| cancel.drop_raw());
            },
        }
    }

    pub fn drop_raw(self) {
        unsafe { (self.drop)(self.data, self.metadata) }
    }
}

impl<T> From<Box<T>> for Cancellation {
    fn from(value: Box<T>) -> Self {
        Self {
            data: Box::into_raw(value) as *mut (),
            metadata: 0,
            drop: |data, _| unsafe {
                drop(Box::from_raw(data));
            },
        }
    }
}

impl<T> From<Box<[T]>> for Cancellation {
    fn from(value: Box<[T]>) -> Self {
        let len = value.len();
        Self {
            data: Box::into_raw(value) as *mut (),
            metadata: len,
            drop: |data, len| unsafe {
                drop(Vec::from_raw_parts(data as *mut T, len, len).into_boxed_slice());
            },
        }
    }
}

impl<T> From<Vec<T>> for Cancellation {
    fn from(value: Vec<T>) -> Self {
        Cancellation::from(value.into_boxed_slice())
    }
}
