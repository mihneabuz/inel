use std::task::Waker;

use io_uring::{squeue::Entry, IoUring};
use tracing::debug;

use crate::cancellation::Cancellation;
use crate::completion::{CompletionSet, Key};

const CANCEL_KEY: u64 = 1_333_337;

pub struct Ring {
    ring: IoUring,
    active: u32,
    canceled: u32,
    completions: CompletionSet,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    pub fn with_capacity(capacity: u32) -> Self {
        let ring = IoUring::builder()
            .build(capacity)
            .expect("Failed to create io_uring");

        Self {
            ring,
            active: 0,
            canceled: 0,
            completions: CompletionSet::with_capacity(capacity as usize),
        }
    }

    pub fn active(&self) -> u32 {
        self.active
    }

    pub fn is_done(&self) -> bool {
        self.active == 0 && self.completions.is_empty()
    }

    /// # Safety
    /// Caller must ensure that the entry is valid until the waker is called
    pub unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        let key = self.completions.insert(waker);

        debug!(key = key.as_u64(), ?entry, "Ring submission");

        self.ring
            .submission()
            .push(&entry.user_data(key.as_u64()))
            .expect("Submission queue is full");

        self.active += 1;

        key
    }

    pub fn check_result(&mut self, key: Key) -> Option<i32> {
        self.completions.result(key)
    }

    /// # Safety
    /// Caller must ensure that the entry is valid until the waker is called
    pub unsafe fn cancel(&mut self, entry: Entry, key: Key, cancel: Cancellation) {
        if !self.completions.cancel(key, cancel) {
            return;
        }

        debug!(key = key.as_u64(), ?entry, "Cancel submission");

        self.ring
            .submission()
            .push(&entry.user_data(CANCEL_KEY))
            .expect("Submission queue is full");

        self.canceled += 1;
    }

    pub fn wait(&mut self) {
        self.submit_and_wait();
        self.handle_completions();
    }

    fn submit_and_wait(&mut self) {
        if self.active == 0 {
            return;
        }

        let want = if self.active == self.canceled {
            self.active
        } else {
            1 + 2 * self.canceled
        };

        debug!(active =? self.active, canceled = ?self.canceled, ?want, "Waiting for events");

        self.ring
            .submit_and_wait(want as usize)
            .expect("Failed to wait for events");

        debug!("Woke up");

        self.canceled = 0;
    }

    fn handle_completions(&mut self) {
        for entry in self.ring.completion() {
            debug!(key = entry.user_data(), ?entry, "Ring completion");

            if entry.user_data() == CANCEL_KEY {
                continue;
            }

            self.completions
                .notify(Key::from_u64(entry.user_data()), entry.result());

            self.active -= 1;
        }
    }
}
