mod completion;
mod register;

use std::task::Waker;
use std::{io::Result, os::fd::RawFd};

use io_uring::{cqueue, squeue::Entry, IoUring};
use tracing::debug;

use crate::{buffer::StableBuffer, Cancellation};

use completion::CompletionSet;
use register::{SlotRegister, WrapSlotKey};

pub use completion::Key;
pub use register::{BufferSlotKey, FileSlotKey};

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
    buffers: SlotRegister,
    files: SlotRegister,
}

impl Ring {
    pub fn with_capacity(capacity: u32) -> Self {
        let ring = IoUring::builder()
            .build(capacity)
            .expect("Failed to create io_uring");

        ring.submitter()
            .register_buffers_sparse(1024)
            .expect("Failed to register buffers sparse");

        ring.submitter()
            .register_files_sparse(1024)
            .expect("Failed to register files sparse");

        Self {
            ring,
            active: 0,
            canceled: 0,
            completions: CompletionSet::with_capacity(capacity as usize),
            buffers: SlotRegister::new(),
            files: SlotRegister::new(),
        }
    }

    /// Returns number of active ops.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// Returns true if all sqes have been completed and all cqes have been consumed.
    pub fn is_done(&self) -> bool {
        self.buffers.is_full()
            && self.files.is_full()
            && self.active == 0
            && self.completions.is_empty()
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
    pub fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferSlotKey>
    where
        B: StableBuffer,
    {
        let key = self.buffers.get();

        let iovec = libc::iovec {
            iov_base: buffer.stable_mut_ptr() as _,
            iov_len: buffer.size(),
        };

        unsafe {
            self.ring
                .submitter()
                .register_buffers_update(key.index(), &[iovec], None)?;
        }

        Ok(BufferSlotKey::wrap(key))
    }

    /// Unregister a buffer
    pub fn unregister_buffer(&mut self, key: BufferSlotKey) {
        self.buffers.remove(key.unwrap());
    }

    /// Attempt to get an io_uring file index
    pub fn register_file(&mut self, fd: Option<RawFd>) -> Result<FileSlotKey> {
        let key = self.files.get();

        if let Some(fd) = fd {
            self.ring
                .submitter()
                .register_files_update(key.index(), &[fd])?;
        }

        Ok(FileSlotKey::wrap(key))
    }

    /// Unregister a buffer
    pub fn unregister_file(&mut self, key: FileSlotKey) {
        self.files.remove(key.unwrap());
    }
}
