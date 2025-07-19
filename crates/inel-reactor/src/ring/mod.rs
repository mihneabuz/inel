mod completion;
mod register;

use std::io::Error;
use std::io::Result;
use std::task::Waker;

use io_uring::{cqueue, squeue::Entry, IoUring};
use tracing::{debug, warn};

use crate::{buffer::StableBuffer, cancellation::Cancellation};

use completion::CompletionSet;
use register::SlotRegister;

pub use completion::Key;
pub use register::{BufferGroupId, BufferSlot, DirectSlot};

const IGNORE_KEY: u64 = u64::MAX - 1;

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
        cqueue::more(self.flags())
    }

    pub fn buffer_id(&self) -> Option<u16> {
        cqueue::buffer_select(self.flags())
    }
}

/// Options for resource allocation
pub struct RingOptions {
    submissions: u32,
    fixed_buffers: u32,
    buffer_groups: u32,
    auto_direct_files: u32,
    manual_direct_files: u32,
}

impl Default for RingOptions {
    fn default() -> Self {
        Self {
            submissions: 2048,
            fixed_buffers: 256,
            buffer_groups: 256,
            auto_direct_files: 256,
            manual_direct_files: 16,
        }
    }
}

impl RingOptions {
    /// Sets the maximum capacity of the submission queue.
    pub fn submissions(mut self, capacity: u32) -> Self {
        self.submissions = capacity;
        self
    }

    /// Sets the number of available auto direct file descriptors
    /// These are descriptors which are returned by direct operations.
    pub fn auto_direct_files(mut self, capacity: u32) -> Self {
        self.auto_direct_files = capacity;
        self
    }

    /// Sets the number of available manual direct file descriptors.
    /// These descriptors are mainly used for chained operations.
    pub fn manual_direct_files(mut self, capacity: u32) -> Self {
        self.manual_direct_files = capacity;
        self
    }

    /// Sets the number of available fixed buffer slots.
    pub fn fixed_buffers(mut self, capacity: u32) -> Self {
        self.fixed_buffers = capacity;
        self
    }

    /// Sets the number of available buffer groups.
    pub fn buffer_groups(mut self, capacity: u32) -> Self {
        self.buffer_groups = capacity;
        self
    }

    /// Build the [Ring] instance with the specified options.
    pub fn build(self) -> Ring {
        if let Err(err) = crate::util::set_limits() {
            warn!(?err, "failed to set max rlimits");
        };

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
            detached: 0,
            canceled: 0,
            completions: CompletionSet::with_capacity(self.submissions as usize),
            direct_files: SlotRegister::new(self.manual_direct_files),
            fixed_buffers: SlotRegister::new(self.fixed_buffers),
            buffer_groups: SlotRegister::new(self.buffer_groups),
        }
    }
}

const SUBMISSION_QUEUE_FULL_ERROR_MESSAGE: &str =
    "Submission queue is full, consider allocating more submissions";

/// Wrapper over an [IoUring] instance.
///
/// Used to submit [Entry]s and be notified of completions and also manages results.
///
/// Also used to register other resources:
///  - direct file descriptors
///  - fixed buffers
///  - buffer groups
pub struct Ring {
    ring: IoUring,
    active: u32,
    detached: u32,
    canceled: u32,
    completions: CompletionSet,
    direct_files: SlotRegister<DirectSlot>,
    fixed_buffers: SlotRegister<BufferSlot>,
    buffer_groups: SlotRegister<BufferGroupId>,
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

    /// Returns number of active submissions.
    /// A submission is considered active if it has an associated [Waker]
    pub fn active(&self) -> u32 {
        self.active
    }

    /// Returns true if:
    ///  - all sqes have been completed
    ///  - all cqes have been consumed
    ///  - all direct file descriptors have been unregistered
    ///  - all fixed buffers have been unregistered
    ///  - all buffer groups have been unregistered
    pub fn is_done(&self) -> bool {
        self.active == 0
            && self.completions.is_empty()
            && self.direct_files.is_full()
            && self.fixed_buffers.is_full()
            && self.buffer_groups.is_full()
    }

    /// Submit an sqe with an associated [Waker] which will be called when the entry completes.
    /// Returns a [Key] which allows the caller to get the cqe result by calling [Ring::check_result].
    ///
    /// This submission will be considered active.
    ///
    /// # Safety
    /// Caller must ensure that the entry is valid until the [Waker] is called.
    pub(crate) unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        let key = self.completions.insert(waker);

        debug!(key = key.as_u64(), ?entry, "Submission");

        self.ring
            .submission()
            .push(&entry.user_data(key.as_u64()))
            .expect(SUBMISSION_QUEUE_FULL_ERROR_MESSAGE);

        self.active += 1;

        key
    }

    /// Submit an sqe that will be detached and have its completion ignored.
    /// This is only useful for entries where the result isn't relevant.
    ///
    /// This submission will be _not_ considered active.
    ///
    /// # Safety
    /// Caller must ensure that the entry is valid until it completes.
    pub(crate) unsafe fn submit_detached(&mut self, entry: Entry) {
        debug!(?entry, "Detached");

        self.ring
            .submission()
            .push(&entry.user_data(IGNORE_KEY))
            .expect(SUBMISSION_QUEUE_FULL_ERROR_MESSAGE);

        self.detached += 1;
    }

    /// Attempt to cancel an entry referenced by a [Key], by submitting a [Cancellation] and,
    /// optionally, another sqe entry which cancels the initial one.
    ///
    /// # Safety
    /// Caller must ensure that the cancel entry is valid until it completes.
    pub(crate) unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        if !self.completions.cancel(key, cancel) {
            return;
        }

        debug!(key = key.as_u64(), ?entry, "Cancel");

        if let Some(entry) = entry {
            self.ring
                .submission()
                .push(&entry.user_data(IGNORE_KEY))
                .expect(SUBMISSION_QUEUE_FULL_ERROR_MESSAGE);

            self.detached += 1;
            self.canceled += 1;
        }
    }

    /// Attempt to get the next result of an entry.
    pub(crate) fn check_result(&mut self, key: Key) -> Option<RingResult> {
        self.completions.result(key)
    }

    /// Blocks until one ore more completions and triggers their associated [Waker]s
    pub fn wait(&mut self) {
        self.submit_and_wait();
        self.handle_completions();
    }

    fn submit_and_wait(&mut self) {
        if self.active + self.detached == 0 {
            return;
        }

        let want = if self.active == self.canceled {
            2 * self.canceled
        } else {
            1 + 2 * self.canceled
        };

        debug!(
            active =? self.active,
            detached =? self.detached,
            canceled =? self.canceled,
            ?want, "Waiting"
        );

        self.ring
            .submit_and_wait(want as usize)
            .expect("Failed to wait for events");

        debug!("Woke up");

        self.canceled = 0;
    }

    fn handle_completions(&mut self) {
        for entry in self.ring.completion() {
            debug!(key = entry.user_data(), ?entry, "Completion");

            if entry.user_data() == IGNORE_KEY {
                self.detached -= 1;
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
    /// Returns a [BufferSlot] if there is one avaialble
    pub(crate) fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferSlot>
    where
        B: StableBuffer,
    {
        let slot = self
            .fixed_buffers
            .get()
            .ok_or(Error::other("No fixed buffer slots available"))?;

        let iovec = libc::iovec {
            iov_base: buffer.stable_ptr() as _,
            iov_len: buffer.size(),
        };

        unsafe {
            self.ring
                .submitter()
                .register_buffers_update(slot.index(), &[iovec], None)?;
        }

        Ok(slot)
    }

    /// Unregister a fixed buffer and release it's [BufferSlot] to be reused
    pub(crate) fn unregister_buffer(&mut self, slot: BufferSlot) {
        self.fixed_buffers.remove(slot);
    }

    /// Attempt to get an unused [DirectSlot] for use as an direct file descriptor.
    pub(crate) fn get_direct_slot(&mut self) -> Result<DirectSlot> {
        self.direct_files
            .get()
            .ok_or(Error::other("No manual direct file slots available"))
    }

    /// Release a direct file descriptor to be reused
    pub(crate) fn release_direct_slot(&mut self, slot: DirectSlot) {
        self.direct_files.remove(slot);
    }

    /// Attempt to get an unused [BufferGroupId]
    pub(crate) fn get_buffer_group(&mut self) -> Result<BufferGroupId> {
        self.buffer_groups
            .get()
            .ok_or(Error::other("No manual buffer group slots available"))
    }

    /// Release a buffer group to be reused
    pub(crate) fn release_buffer_group(&mut self, slot: BufferGroupId) {
        self.buffer_groups.remove(slot);
    }
}
