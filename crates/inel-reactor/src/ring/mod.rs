mod completion;
mod register;

use std::io::Error;
use std::io::Result;
use std::task::Waker;

use io_uring::{cqueue, squeue::Entry, IoUring};
use tracing::debug;

use crate::{buffer::StableBuffer, Cancellation};

use completion::CompletionSet;
use register::{SlotRegister, WrapSlotKey};

pub use completion::Key;
pub use register::{BufferSlotKey, FileSlotKey};

const CANCEL_KEY: u64 = 1_333_337;

pub struct RingOptions {
    submissions: u32,
    fixed_buffers: u32,
    auto_direct_files: u32,
    manual_direct_files: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RingResult {
    ret: i32,
    flags: u32,
}

impl RingResult {
    pub fn from_completion(entry: &cqueue::Entry) -> Self {
        Self {
            ret: entry.result(),
            flags: entry.flags(),
        }
    }

    pub fn ret(&self) -> i32 {
        self.ret
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn has_more(&self) -> bool {
        cqueue::more(self.flags)
    }
}

impl Default for RingOptions {
    fn default() -> Self {
        Self {
            submissions: 2048,
            fixed_buffers: 256,
            auto_direct_files: 256,
            manual_direct_files: 16,
        }
    }
}

impl RingOptions {
    pub fn submissions(mut self, capacity: u32) -> Self {
        self.submissions = capacity;
        self
    }

    pub fn fixed_buffers(mut self, capacity: u32) -> Self {
        self.fixed_buffers = capacity;
        self
    }

    pub fn auto_direct_files(mut self, capacity: u32) -> Self {
        self.auto_direct_files = capacity;
        self
    }

    pub fn manual_direct_files(mut self, capacity: u32) -> Self {
        self.manual_direct_files = capacity;
        self
    }

    pub fn build(self) -> Ring {
        let ring = IoUring::builder()
            .build(self.submissions)
            .expect("Failed to create io_uring");

        if self.fixed_buffers > 0 {
            ring.submitter()
                .register_buffers_sparse(self.fixed_buffers)
                .expect("Failed to register buffers sparse");
        }

        if self.auto_direct_files + self.manual_direct_files > 0 {
            ring.submitter()
                .register_files_sparse(self.auto_direct_files + self.manual_direct_files)
                .expect("Failed to register files sparse");
        }

        if self.auto_direct_files > 0 {
            ring.submitter()
                .register_file_alloc_range(self.manual_direct_files, self.auto_direct_files)
                .expect("Failed to register files auto allocation range");

            if self.manual_direct_files > 0 {
                ring.submitter()
                    .register_files_update(0, &vec![0; self.manual_direct_files as usize])
                    .expect("Failed to reserve manual direct files");
            }
        }

        Ring {
            ring,
            active: 0,
            canceled: 0,
            completions: CompletionSet::with_capacity(self.submissions as usize),
            buffers: SlotRegister::new(self.fixed_buffers),
            files: SlotRegister::new(self.manual_direct_files),
        }
    }
}

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

impl Default for Ring {
    fn default() -> Self {
        Ring::options().build()
    }
}

impl Ring {
    pub fn options() -> RingOptions {
        RingOptions::default()
    }

    /// Returns number of active ops.
    pub fn active(&self) -> u32 {
        self.active
    }

    /// Returns true if:
    ///  - all sqes have been completed
    ///  - all cqes have been consumed
    ///  - all buffers have been unregistered
    ///  - all files have been unregistered
    pub fn is_done(&self) -> bool {
        self.active == 0
            && self.completions.is_empty()
            && self.buffers.is_full()
            && self.files.is_full()
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
    pub fn check_result(&mut self, key: Key) -> Option<RingResult> {
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

            let result = RingResult::from_completion(&entry);
            if !result.has_more() {
                self.active -= 1;
            }

            self.completions
                .notify(Key::from_u64(entry.user_data()), result);
        }
    }

    /// Attempt to register a [StableBuffer] for use with fixed operations.
    pub fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferSlotKey>
    where
        B: StableBuffer,
    {
        let key = self
            .buffers
            .get()
            .ok_or(Error::other("No fixed buffer slots available"))?;

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
    pub fn get_file_slot(&mut self) -> Result<FileSlotKey> {
        let key = self
            .files
            .get()
            .ok_or(Error::other("No manual direct file slots available"))?;

        Ok(FileSlotKey::wrap(key))
    }

    /// Unregister a buffer
    pub fn release_file_slot(&mut self, key: FileSlotKey) {
        self.files.remove(key.unwrap());
    }
}
