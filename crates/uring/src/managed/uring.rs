use core::{ptr::NonNull, task::Waker};

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

/// Access to a [`Uring`], wrapped in whatever the consumer likes (a
/// `ThreadLocal<RefCell<_>>`, an `Rc<RefCell<_>>`, ...). The managed types
/// only reach the ring through this trait.
///
/// # Safety
/// Every call to [`Self::with`], on this proxy or any of its clones, must pass
/// `f` the same [`Uring`]. In particular, it must not be replaced by another
/// one while anything created through the proxy (ops, buffer groups,
/// buffers) is alive: keys and buffer ids are only meaningful to the ring
/// that handed them out.
pub unsafe trait UringProxy: Clone {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T;

    fn wait(&self) {
        self.with(|uring| uring.wait());
    }

    fn is_done(&self) -> bool {
        self.with(|uring| uring.is_done())
    }

    fn shutdown(&self) {
        self.with(|uring| uring.shutdown());
    }
}

// SAFETY: forwards to `P`, so it reaches the same ring
unsafe impl<P: UringProxy> UringProxy for &P {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T,
    {
        (*self).with(f)
    }
}

/// The ring protocol used by the managed types. Crate-private and implemented
/// for every proxy, so it can neither be called nor overridden from outside:
/// its methods trust their keys and buffer ids.
pub trait UringProxyExt: UringProxy {
    fn submit<O: SingleOp>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit(op, waker))
    }

    fn submit_multi<O: MultiOp>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit_multi(op, waker))
    }

    fn submit_detached<O: DetachOp>(&mut self, op: O) {
        self.with(|uring| uring.submit_detached(op))
    }

    fn cancel(&self, key: Key, handle: Cancellation, entry: bool) {
        self.with(|uring| uring.cancel(key, handle, entry))
    }

    fn update(&self, key: Key, waker: &Waker) {
        self.with(|uring| uring.update(key, waker))
    }

    fn result(&self, key: Key) -> Option<CompletionResult> {
        self.with(|uring| uring.result(key))
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

impl<P: UringProxy> UringProxyExt for P {}

pub struct Uring {
    ring: IoUring,
    comp: Completions,
    groups: BufGroups,
    /// Submitted ops whose final CQE has not been reaped yet, so the kernel
    /// may still access their memory. Detached ops reference none and are
    /// not counted.
    in_kernel: u32,
}

impl Uring {
    const SUBMISSION_QUEUE_FULL_ERROR: &str =
        "Submission queue full. Consider configuring with more `sq_entries`.";
    const FAILED_TO_ENTER_ERROR: &str = "Failed to enter io_uring";

    /// Size of the direct descriptor table.
    const DIRECT_FILES: u32 = 1024;
    /// Slots at the start of the table that the kernel doesn't allocate from,
    /// for ops that install into a chosen slot (e.g. a linked open and read).
    /// None yet.
    const RESERVED_DIRECT_FILES: u32 = 0;

    pub fn new() -> Result<Self, Errno> {
        Self::with_direct_files(Self::DIRECT_FILES)
    }

    /// With a direct descriptor table of `direct_files` slots.
    pub(crate) fn with_direct_files(direct_files: u32) -> Result<Self, Errno> {
        let ring = IoUring::options()
            .with_sq_entries(128)
            .with_cq_entries(256)
            .with_single_issuer()
            .with_defer_taskrun()
            .with_coop_taskrun()
            .with_taskrun_flag()
            .with_submit_all()
            .build()?;

        ring.register_files_sparse(direct_files)?;
        ring.register_file_alloc_range(
            Self::RESERVED_DIRECT_FILES,
            direct_files - Self::RESERVED_DIRECT_FILES,
        )?;

        let comp = Completions::new(ring.sq_size());

        Ok(Self {
            ring,
            comp,
            groups: BufGroups::new(),
            in_kernel: 0,
        })
    }

    pub const fn is_done(&self) -> bool {
        self.comp.is_empty()
    }

    pub(crate) fn submit<O: SingleOp>(&mut self, op: &mut O, waker: Waker) -> Key {
        let mut key = self.comp.insert_single(waker);
        self.push(op, &mut key);
        key
    }

    pub(crate) fn submit_multi<O: MultiOp>(&mut self, op: &mut O, waker: Waker) -> Key {
        let mut key = self.comp.insert_multi(waker);
        self.push(op, &mut key);
        key
    }

    fn push<O: Op>(&mut self, op: &mut O, key: &mut Key) {
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

        self.in_kernel += 1;
    }

    pub(crate) fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        // SAFETY: DetachOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    sqe.user_data(Key::IGNORE.as_raw());
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    pub(crate) fn update(&mut self, key: Key, waker: &Waker) {
        self.comp.update(key, waker);
    }

    pub(crate) fn cancel(&mut self, key: Key, handle: Cancellation, entry: bool) {
        // stored CQEs that are discarded unread give back their buffer and
        // their hold on the group
        let pending = self.comp.cancel(key, handle, |result| {
            if let Some(gid) = key.group_id() {
                if let Some(bid) = result.buffer_id() {
                    self.groups.recycle(gid, bid);
                }
                if !result.has_more() {
                    self.groups.release(gid);
                }
            }
        });

        if !pending {
            return;
        }

        if entry {
            self.submit_detached(AsyncCancel::new(key));
        }
    }

    pub(crate) fn result(&mut self, key: Key) -> Option<CompletionResult> {
        let result = self.comp.result(key)?;

        // the CQE held the group alive until now. `ReadSelect::result` still
        // owns a group handle, so the group can't die before `take` adds the
        // `SelectBuf`'s user
        if !result.has_more()
            && let Some(gid) = key.group_id()
        {
            self.groups.release(gid);
        }

        Some(result)
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

            if !result.has_more() {
                self.in_kernel -= 1;
            }

            // a stored CQE keeps the op's hold on the group until it is read
            // (`result`) or discarded (`cancel`), since its buffer id points
            // into the group's memory
            if let Some(gid) = key.group_id()
                && !stored
            {
                if let Some(bid) = result.buffer_id() {
                    self.groups.recycle(gid, bid);
                }
                if !result.has_more() {
                    self.groups.release(gid);
                }
            }
        });

        self.groups.free_dead(&self.ring);
    }

    /// Cancels every op in flight and waits until the kernel is done with all
    /// of them, so dropping the ring afterwards frees everything. Ops that are
    /// still awaited complete, usually with `ECANCELED`.
    ///
    /// Without it, dropping the ring leaks the memory the kernel may still be
    /// using: buffers of cancelled ops and buffer groups that are in use.
    pub fn shutdown(&mut self) {
        while self.in_kernel > 0 {
            // resubmitted every round: requests run in any order, so a cancel
            // can miss an op that the kernel picks up after it. Each round
            // posts at least one CQE, the cancel's -ENOENT if nothing matched.
            self.submit_detached(AsyncCancel::all());
            self.wait();
        }
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
        // closing the ring is next (fields drop in order), the kernel tears it
        // down asynchronously, then `BufGroups` leaks every group still in use.
        // Unregistering the files closes the direct descriptors still open.
    }
}
