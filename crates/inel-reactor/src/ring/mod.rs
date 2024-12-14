mod completion;
mod register;

use std::io::Result;
use std::task::Waker;

use io_uring::{cqueue, squeue::Entry, IoUring};
use tracing::debug;

use crate::{buffer::StableBuffer, Cancellation};

use completion::CompletionSet;
use register::BufferRegister;

pub use completion::Key;
pub use register::BufferKey;

const CANCEL_KEY: u64 = 1_333_337;

/// Reactor implemented over a [IoUring].
///
/// Used by [Submission](crate::Submission) to submit sqes and get notified
/// of completions by use of [Waker].
///
/// Also used to register other resources, such as fixed buffers.
pub struct Ring {
    ring: IoUring,
    active: u32,
    canceled: u32,
    completions: CompletionSet,
    buffers: BufferRegister,
    buffers_registered: bool,
}

impl Ring {
    pub fn with_capacity(capacity: u32) -> Self {
        let ring = IoUring::builder()
            .build(capacity)
            .expect("Failed to create io_uring");

        Self {
            ring,
            active: 0,
            canceled: 0,
            completions: CompletionSet::with_capacity(capacity as usize),
            buffers: BufferRegister::new(),
            buffers_registered: false,
        }
    }

    /// Returns number of active ops.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// Returns true if all sqes have been completed and all cqes have been consumed.
    pub fn is_done(&self) -> bool {
        self.active == 0 && self.completions.is_empty()
    }

    /// Submit an sqe with an associated [Waker] which will be called
    /// when the entry completes.
    /// Returns a [Key] which allowes the caller to get the cqe result
    /// by calling [Ring::check_result].
    ///
    /// # Safety
    /// Caller must ensure that the entry is valid until the waker is called.
    pub unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        let key = self.completions.insert(waker);

        debug!(key = key.as_u64(), ?entry, "Submission");

        self.ring
            .submission()
            .push(&entry.user_data(key.as_u64()))
            .expect("Submission queue is full");

        self.active += 1;

        key
    }

    /// Attempt to get the result of an entry.
    pub fn check_result(&mut self, key: Key) -> Option<(i32, bool)> {
        self.completions.result(key)
    }

    /// Attempt to cancel the entry referenced by `key`, by submitting a [Cancellation] and,
    /// optionally, another sqe entry which cancels the initial one.
    ///
    /// # Safety
    /// Caller must ensure that the cancel entry is valid until it completes.
    pub unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        if !self.completions.cancel(key, cancel) {
            return;
        }

        debug!(key = key.as_u64(), ?entry, "Cancel");

        if let Some(entry) = entry {
            self.ring
                .submission()
                .push(&entry.user_data(CANCEL_KEY))
                .expect("Submission queue is full");

            self.canceled += 1;
        }
    }

    /// Blocks until at least 1 completion
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

        debug!(active =? self.active, canceled = ?self.canceled, ?want, "Waiting");

        self.ring
            .submit_and_wait(want as usize)
            .expect("Failed to wait for events");

        debug!("Woke up");

        self.canceled = 0;
    }

    fn handle_completions(&mut self) {
        for entry in self.ring.completion() {
            debug!(key = entry.user_data(), ?entry, "Completion");

            if entry.user_data() == CANCEL_KEY {
                continue;
            }

            let has_more = cqueue::more(entry.flags());

            self.completions
                .notify(Key::from_u64(entry.user_data()), entry.result(), has_more);

            if !has_more {
                self.active -= 1;
            }
        }
    }

    /// Attempt to register a [StableBuffer] for use with fixed operations.
    ///
    /// # Safety
    /// Caller must ensure that the buffer is valid until the ring is destroyed
    pub unsafe fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferKey>
    where
        B: StableBuffer,
    {
        if self.buffers_registered {
            self.ring.submitter().unregister_buffers()?;
        } else {
            self.buffers_registered = true;
        }

        let key = self.buffers.insert(buffer);

        self.ring
            .submitter()
            .register_buffers(self.buffers.iovecs())?;

        Ok(key)
    }

    /// Unregister a [StableBuffer]
    pub fn unregister_buffer<B>(&mut self, buffer: &mut B, key: BufferKey)
    where
        B: StableBuffer,
    {
        self.buffers.remove(buffer, key);
    }
}
