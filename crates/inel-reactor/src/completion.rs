use std::{mem, task::Waker};

use slab::Slab;

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

    pub fn drop_raw(self) {
        unsafe { (self.drop)(self.data, self.metadata) }
    }
}

impl<T> From<Box<T>> for Cancellation {
    fn from(value: Box<T>) -> Self {
        Self {
            data: Box::into_raw(value) as *mut (),
            metadata: 0,
            drop: |data, _len| unsafe {
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

enum CompletionInner {
    Active(Waker),
    Finished(i32),
    Cancelled(Cancellation),
}

pub struct Completion {
    state: CompletionInner,
}

impl Completion {
    pub fn new(waker: Waker) -> Self {
        Self {
            state: CompletionInner::Active(waker),
        }
    }

    pub fn try_cancel(&mut self, cancel: Cancellation) -> bool {
        match self.state {
            CompletionInner::Active(_) => {
                self.state = CompletionInner::Cancelled(cancel);
                false
            }

            CompletionInner::Finished(_) => true,

            CompletionInner::Cancelled(_) => {
                panic!("Completion already cancelled");
            }
        }
    }

    pub fn try_notify(&mut self, ret: i32) -> bool {
        match mem::replace(&mut self.state, CompletionInner::Finished(ret)) {
            CompletionInner::Active(waker) => {
                waker.wake();
                false
            }

            CompletionInner::Cancelled(cancel) => {
                cancel.drop_raw();
                true
            }

            CompletionInner::Finished(_) => {
                panic!("Completion already finished");
            }
        }
    }

    pub fn result(&self) -> Option<i32> {
        match &self.state {
            CompletionInner::Finished(ret) => Some(*ret),
            _ => None,
        }
    }
}

pub struct CompletionSet {
    slab: Slab<Completion>,
}

impl CompletionSet {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slab: Slab::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, waker: Waker) -> Key {
        Key {
            value: self.slab.insert(Completion::new(waker)),
        }
    }

    fn with_completion<F>(&mut self, key: Key, f: F)
    where
        F: FnOnce(&mut Completion) -> bool,
    {
        let completion = unsafe { self.slab.get_unchecked_mut(key.as_usize()) };
        let should_remove = f(completion);
        if should_remove {
            self.slab.remove(key.as_usize());
        }
    }

    pub fn cancel(&mut self, key: Key, cancel: Cancellation) {
        self.with_completion(key, move |comp| comp.try_cancel(cancel));
    }

    pub fn notify(&mut self, key: Key, ret: i32) {
        self.with_completion(key, move |comp| comp.try_notify(ret));
    }

    pub fn result(&self, key: Key) -> Option<i32> {
        unsafe { self.slab.get_unchecked(key.as_usize()).result() }
    }
}

#[derive(Clone, Copy)]
pub struct Key {
    value: usize,
}

impl Key {
    fn as_usize(&self) -> usize {
        self.value
    }

    pub(crate) fn from_u64(value: u64) -> Self {
        Self {
            value: value as usize,
        }
    }

    pub(crate) fn as_u64(&self) -> u64 {
        self.value as u64
    }
}
