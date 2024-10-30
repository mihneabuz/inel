use std::{mem, task::Waker};

use slab::Slab;

use crate::cancellation::Cancellation;

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
                unreachable!("Completion already cancelled");
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
                unreachable!("Completion already finished");
            }
        }
    }

    pub fn take_result(&self) -> Option<i32> {
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

    pub fn is_empty(&self) -> bool {
        self.slab.is_empty()
    }

    pub fn insert(&mut self, waker: Waker) -> Key {
        Key {
            value: self.slab.insert(Completion::new(waker)),
        }
    }

    fn with_completion<T, F, G>(&mut self, key: Key, fun: F, should_remove: G) -> T
    where
        F: FnOnce(&mut Completion) -> T,
        G: FnOnce(&T) -> bool,
    {
        let completion = self.slab.get_mut(key.as_usize()).expect("this is bad");
        let value = fun(completion);
        if should_remove(&value) {
            self.slab.remove(key.as_usize());
        }
        value
    }

    pub fn cancel(&mut self, key: Key, cancel: Cancellation) -> bool {
        !self.with_completion(key, move |comp| comp.try_cancel(cancel), |&success| success)
    }

    pub fn notify(&mut self, key: Key, ret: i32) -> bool {
        !self.with_completion(key, move |comp| comp.try_notify(ret), |&remove| remove)
    }

    pub fn result(&mut self, key: Key) -> Option<i32> {
        self.with_completion(key, move |comp| comp.take_result(), |res| res.is_some())
    }
}

#[derive(Clone, Copy, Debug)]
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
