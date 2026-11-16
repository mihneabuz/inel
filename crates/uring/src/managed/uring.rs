use core::{ptr::NonNull, task::Waker};

use rustix::{io::Errno, io_uring::IoringAsyncCancelFlags};

use crate::{
    managed::{
        UringProxy,
        cancellation::Cancellation,
        completion::{CompletionResult, Completions, Key, mark_detached, parse_cqe},
        group::{BufGroupId, BufGroups, BufId},
        submission::{MultiOp, Op, Scratch, SingleOp},
    },
    sys::{IoUring, Sqe},
};

pub struct Uring {
    ring: IoUring,
    comp: Completions,
    groups: BufGroups,
    scratch: Scratch,
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
            scratch: Scratch::default(),
            in_kernel: 0,
        })
    }

    pub const fn is_done(&self) -> bool {
        self.comp.is_empty()
    }

    pub(crate) fn submit<U, O>(&mut self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: SingleOp<U>,
    {
        self.push(op, |comp, sqe| unsafe { comp.insert_single(sqe, waker) })
    }

    pub(crate) fn submit_multi<U, O>(&mut self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: MultiOp<U>,
    {
        self.push(op, |comp, sqe| comp.insert_multi(sqe, waker))
    }

    /// Preps `op` into the next SQE, then registers it with `insert`, which
    /// sets the SQE's `user_data`.
    fn push<O, F>(&mut self, op: &mut O, insert: F) -> Key
    where
        O: Op,
        F: FnOnce(&mut Completions, &mut Sqe) -> Key,
    {
        // SAFETY: Op implementation must guarantee safety
        let key = unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe, &self.scratch);
                    insert(&mut self.comp, sqe)
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        };

        if let Some(group) = key.group_id() {
            self.groups.acquire(group);
        }

        self.in_kernel += 1;
        key
    }

    /// Submits an SQE built by `prep` without tracking it: a successful
    /// completion is skipped, a failed one ignored.
    ///
    /// # Safety
    /// Nothing waits for the completion, so the SQE must not reference any
    /// memory (beyond what the kernel copies while preparing it), nor select a
    /// buffer from a group, since the CQE carrying it is ignored.
    pub(crate) unsafe fn prep_detached<F>(&mut self, prep: F)
    where
        F: FnOnce(&mut Sqe),
    {
        // SAFETY: the caller guarantees the SQE references no memory
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    prep(sqe);
                    mark_detached(sqe);
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    pub(crate) fn update(&mut self, key: &Key, waker: &Waker) {
        self.comp.update(key, waker);
    }

    pub(crate) fn cancel(&mut self, key: &Key, handle: Cancellation, entry: bool) {
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
            // SAFETY: the cancel only carries the key, it references no memory
            unsafe {
                self.prep_detached(|sqe| {
                    sqe.prep_cancel(key.as_raw(), IoringAsyncCancelFlags::empty());
                });
            };
        }
    }

    pub(crate) fn result(&mut self, key: &Key) -> Option<CompletionResult> {
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
            let Some((key, result)) = parse_cqe(cqe) else {
                return;
            };

            let stored = self.comp.notify(&key, result);

            if !result.has_more() {
                self.in_kernel -= 1;
            }

            // a stored CQE keeps the op's hold on the group until it is read
            // (`result`) or discarded (`cancel`), since its buffer id points
            // into the group's memory
            if !stored && let Some(gid) = key.group_id() {
                if let Some(bid) = result.buffer_id() {
                    self.groups.recycle(gid, bid);
                }
                if !result.has_more() {
                    self.groups.release(gid);
                }
            }
        });

        self.scratch.reset();

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
            // SAFETY: the cancel references no memory
            unsafe {
                self.prep_detached(|sqe| {
                    sqe.prep_cancel(0, IoringAsyncCancelFlags::ANY);
                });
            };
            self.wait();
        }
    }

    pub(crate) fn create_buf_group(&mut self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        let id = self.groups.insert(count, size)?;
        self.groups.register_or_drop(id, &self.ring)?;
        Ok(id)
    }

    pub(crate) fn clone_buf_group(&mut self, id: BufGroupId) {
        self.groups.clone_handle(id);
    }

    pub(crate) fn drop_buf_group(&mut self, id: BufGroupId) {
        self.groups.drop_handle(id);
        self.groups.free_dead(&self.ring);
    }

    pub(crate) fn take_buf(&mut self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        self.groups.take(id, bid)
    }

    pub(crate) fn return_buf(&mut self, id: BufGroupId, bid: BufId) {
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
