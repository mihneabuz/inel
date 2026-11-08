use core::{pin::Pin, ptr::NonNull, task::Waker};

use rustix::io::Errno;

use crate::{
    managed::{
        cancellation::Cancellation,
        completion::{CompletionEntry, CompletionResult, Completions, Key},
        group::{BufGroupId, BufGroups, BufId},
        submission::{AsyncCancel, DetachOp, MultiOp, Op, SingleOp},
    },
    sys::IoUring,
};

pub trait UringProxy: Clone {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T;

    fn submit<O: SingleOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        self.with(|uring| uring.submit(op, waker))
    }

    fn submit_multi<O: MultiOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        self.with(|uring| uring.submit_multi(op, waker))
    }

    fn submit_detached<O: DetachOp>(&mut self, op: O) {
        self.with(|uring| uring.submit_detached(op))
    }

    fn cancel(&self, key: Key, handle: Cancellation, entry: bool) {
        self.with(|uring| uring.cancel(key, handle, entry))
    }

    fn update(&self, key: Key, waker: Waker) {
        self.with(|uring| uring.update(key, waker))
    }

    fn result(&self, key: Key) -> Option<CompletionResult> {
        self.with(|uring| uring.result(key))
    }

    fn wait(&self) {
        self.with(|uring| uring.wait());
    }

    fn is_done(&self) -> bool {
        self.with(|uring| uring.is_done())
    }

    /// Allocates and registers a group of `count` buffers of `size` bytes.
    fn create_buf_group(&self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        self.with(|uring| uring.create_buf_group(count, size))
    }

    /// A handle to the group was cloned.
    fn clone_buf_group(&self, id: BufGroupId) {
        self.with(|uring| uring.clone_buf_group(id))
    }

    /// A handle to the group was dropped. The group is freed once no handle,
    /// op or buffer uses it anymore.
    fn drop_buf_group(&self, id: BufGroupId) {
        self.with(|uring| uring.drop_buf_group(id))
    }

    /// Hands out buffer `bid` of group `id`, keeping the group alive until it
    /// is returned with [`Self::return_buf`].
    fn take_buf(&self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        self.with(|uring| uring.take_buf(id, bid))
    }

    /// Puts a buffer from [`Self::take_buf`] back in its group's ring.
    fn return_buf(&self, id: BufGroupId, bid: BufId) {
        self.with(|uring| uring.return_buf(id, bid))
    }
}

impl<P: UringProxy> UringProxy for &P {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T,
    {
        (*self).with(f)
    }
}

pub struct Uring {
    ring: IoUring,
    comp: Completions,
    groups: BufGroups,
}

impl Uring {
    const SUBMISSION_QUEUE_FULL_ERROR: &str =
        "Submission queue full. Consider configuring with more `sq_entries`.";
    const FAILED_TO_ENTER_ERROR: &str = "Failed to enter io_uring";

    pub fn new() -> Result<Self, Errno> {
        let ring = IoUring::options()
            .with_sq_entries(128)
            .with_cq_entries(256)
            .with_single_issuer()
            .with_defer_taskrun()
            .with_coop_taskrun()
            .with_taskrun_flag()
            .with_submit_all()
            .build()?;

        let comp = Completions::new(ring.sq_size());

        Ok(Self {
            ring,
            comp,
            groups: BufGroups::new(),
        })
    }

    pub const fn is_done(&self) -> bool {
        self.comp.is_empty()
    }

    pub fn submit<O: SingleOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut key = self.comp.insert_single(waker);
        self.push(op, &mut key);
        key
    }

    pub fn submit_multi<O: MultiOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut key = self.comp.insert_multi(waker);
        self.push(op, &mut key);
        key
    }

    fn push<O: Op>(&mut self, op: Pin<&mut O>, key: &mut Key) {
        // SAFETY: Op implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    sqe.user_data(key.add_group_id(sqe).as_raw());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        if let Some(group) = key.group_id() {
            self.groups.acquire(group);
        }
    }

    pub fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        // SAFETY: DetachOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep_detached(sqe);
                    sqe.user_data(Key::IGNORE.as_raw());
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    pub fn update(&mut self, key: Key, waker: Waker) {
        self.comp.update(key, waker);
    }

    pub fn cancel(&mut self, key: Key, handle: Cancellation, entry: bool) {
        let pending = self.comp.cancel(key, handle, |result| {
            if let Some(bid) = result.buffer_id()
                && let Some(gid) = key.group_id()
            {
                self.groups.recycle(gid, bid);
            }
        });

        if !pending {
            return;
        }

        if entry {
            self.submit_detached(AsyncCancel::new(key));
        }
    }

    pub fn result(&mut self, key: Key) -> Option<CompletionResult> {
        self.comp.result(key)
    }

    pub fn wait(&mut self) {
        self.ring
            .submit_and_wait(1)
            .expect(Self::FAILED_TO_ENTER_ERROR);

        self.ring.for_each_cqe(|cqe| {
            let completion = CompletionEntry::from_cqe(cqe);
            if completion.should_ignore() {
                return;
            }
            let key = completion.key();

            let result = completion.result();
            let stored = self.comp.notify(key, result);

            if let Some(gid) = key.group_id() {
                if !stored && let Some(bid) = result.buffer_id() {
                    self.groups.recycle(gid, bid);
                }

                if !result.has_more() {
                    self.groups.release(gid);
                }
            }
        });

        self.groups.free_dead(&self.ring);
    }

    fn create_buf_group(&mut self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        let id = self.groups.insert(count, size)?;
        self.groups.register_or_drop(id, &self.ring)?;
        Ok(id)
    }

    fn clone_buf_group(&mut self, id: BufGroupId) {
        self.groups.clone_handle(id);
    }

    fn drop_buf_group(&mut self, id: BufGroupId) {
        self.groups.drop_handle(id);
        self.groups.free_dead(&self.ring);
    }

    fn take_buf(&mut self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        self.groups.take(id, bid)
    }

    fn return_buf(&mut self, id: BufGroupId, bid: BufId) {
        self.groups.recycle(id, bid);
        self.groups.release(id);
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        let _ = self.ring.unregister_buffers();
        let _ = self.ring.unregister_files();
    }
}
