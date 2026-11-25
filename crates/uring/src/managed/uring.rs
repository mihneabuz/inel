use core::{cell::RefCell, task::Waker};

use rustix::{io::Errno, io_uring::IoringAsyncCancelFlags};

use crate::{
    managed::{
        SelectGroup, UringProxy,
        cancellation::Cancellation,
        completion::{CompletionResult, Completions, Key, mark_detached, parse_cqe},
        group::BufGroups,
        submission::{MultiOp, Op, Scratch, SingleOp},
    },
    sys::{IoUring, Sqe},
};

/// Configuration for a [`Uring`], created with [`Uring::options`].
///
/// The direct descriptor table has `manual_direct_files` slots at the start, which
/// ops install into by index, followed by `auto_direct_files` slots the kernel
/// allocates from (e.g. for [`OpenDirect`]).
///
/// [`OpenDirect`]: super::OpenDirect
#[derive(Debug, Clone)]
pub struct UringOptions {
    sq_entries: u32,
    cq_entries: u32,
    auto_direct_files: u32,
    manual_direct_files: u32,
}

impl Default for UringOptions {
    fn default() -> Self {
        Self {
            sq_entries: 128,
            cq_entries: 256,
            auto_direct_files: 1024,
            manual_direct_files: 0,
        }
    }
}

impl UringOptions {
    /// Sets the size of the submission queue, which also bounds the ops submitted
    /// between two waits.
    #[must_use]
    pub const fn with_sq_entries(mut self, entries: u32) -> Self {
        self.sq_entries = entries;
        self
    }

    /// Sets the size of the completion queue.
    #[must_use]
    pub const fn with_cq_entries(mut self, entries: u32) -> Self {
        self.cq_entries = entries;
        self
    }

    /// Sets the number of direct descriptor slots the kernel allocates from.
    #[must_use]
    pub const fn with_auto_direct_files(mut self, count: u32) -> Self {
        self.auto_direct_files = count;
        self
    }

    /// Sets the number of direct descriptor slots reserved for ops that install
    /// into a chosen slot (e.g. a linked open and read).
    #[must_use]
    pub const fn with_manual_direct_files(mut self, count: u32) -> Self {
        self.manual_direct_files = count;
        self
    }

    /// Builds the [`Uring`].
    ///
    /// Fails with `EINVAL` if the direct descriptor table would have more than
    /// `u32::MAX` slots, and with any error from setting up the ring or
    /// registering the table.
    pub fn build(self) -> Result<Uring, Errno> {
        let direct_files = self
            .manual_direct_files
            .checked_add(self.auto_direct_files)
            .ok_or(Errno::INVAL)?;

        let ring = IoUring::options()
            .with_sq_entries(self.sq_entries)
            .with_cq_entries(self.cq_entries)
            .with_single_issuer()
            .with_defer_taskrun()
            .with_coop_taskrun()
            .with_taskrun_flag()
            .with_submit_all()
            .build()?;

        // the kernel rejects an empty table
        if direct_files > 0 {
            ring.register_files_sparse(direct_files)?;
            ring.register_file_alloc_range(self.manual_direct_files, self.auto_direct_files)?;
        }

        let comp = Completions::with_capacity(ring.sq_size());

        Ok(Uring {
            core: RefCell::new(Core {
                ring,
                comp,
                active: 0,
                scratch: Scratch::default(),
                groups: BufGroups::new(),
            }),
        })
    }
}

/// A reactor powered by a managed [`IoUring`].
///
/// It handles submitting ops, waiting for events, getting completion results, as well as
/// other features provided by [`IoUring`] (direct files, fixed buffers, buffer rings).
/// Ops must implement the [`Op`] trait, as well as either [`SigleOp`] or [`MultiOp`] and
/// must be managed by a [`Submission`], which also handles results and cancellation.
///
/// Most other types and APIs should not use [`Uring`] directly, but instead prefer to be
/// generic over the [`UringProxy`] trait, which allows users to define their own logic to
/// provide access to the actual [`Uring`] instance.
pub struct Uring {
    core: RefCell<Core>,
}

struct Core {
    ring: IoUring,
    comp: Completions,
    active: u32,
    scratch: Scratch,
    groups: BufGroups,
}

impl Core {
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
                .expect(Uring::SUBMISSION_QUEUE_FULL_ERROR)
        };

        self.active += 1;
        key
    }

    /// # Safety
    /// See [`Uring::prep_detached`].
    unsafe fn prep_detached<F>(&mut self, prep: F)
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
                .expect(Uring::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }
}

impl Uring {
    const SUBMISSION_QUEUE_FULL_ERROR: &str =
        "Submission queue full. Consider configuring with more `sq_entries`.";
    const FAILED_TO_ENTER_ERROR: &str = "Failed to enter io_uring";

    /// Creates a ring with the default options.
    pub fn new() -> Result<Self, Errno> {
        Self::options().build()
    }

    /// Starts configuring a ring; finish with [`UringOptions::build`].
    #[must_use]
    pub fn options() -> UringOptions {
        UringOptions::default()
    }

    /// Returns true if there are no more in-flight ops.
    pub fn is_done(&self) -> bool {
        self.core.borrow().comp.is_empty()
    }

    /// Submits an SQE built by [`O::prep`] and creates a completion handler for it.
    /// Returns a `Key` which can be used to retrieve the result of the op.
    pub(crate) fn submit<U, O>(&self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: SingleOp<U>,
    {
        self.core.borrow_mut().push(op, |comp, sqe| {
            // SAFETY: SingleOp implementation guarantees it generates exactly one CQE.
            unsafe { comp.insert_single(sqe, waker) }
        })
    }

    /// Submits an SQE built by [`O::prep`] and creates a completion handler for it.
    /// Returns a `Key` which can be used to retrieve the results of the op.
    pub(crate) fn submit_multi<U, O>(&self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: MultiOp<U>,
    {
        self.core
            .borrow_mut()
            .push(op, |comp, sqe| comp.insert_multi(sqe, waker))
    }

    /// Submits an SQE built by `prep` without tracking it: a successful completion is
    /// skipped, a failed one ignored.
    ///
    /// # Safety
    /// Nothing waits for the completion, so the SQE must not reference any memory
    /// (beyond what the kernel copies while preparing it), nor select a buffer from
    /// a group, since the CQE carrying it is ignored.
    pub(crate) unsafe fn prep_detached<F>(&self, prep: F)
    where
        F: FnOnce(&mut Sqe),
    {
        // SAFETY: forwarded to the caller
        unsafe { self.core.borrow_mut().prep_detached(prep) }
    }

    pub(crate) fn update(&self, key: &Key, waker: &Waker) {
        self.core.borrow_mut().comp.update(key, waker);
    }

    pub(crate) fn result(&self, key: &Key) -> Option<CompletionResult> {
        self.core.borrow_mut().comp.result(key)
    }

    pub(crate) fn cancel(&self, key: &Key, handle: Cancellation, entry: bool) {
        let mut core = self.core.borrow_mut();
        let Core { comp, groups, .. } = &mut *core;

        let pending = comp.cancel(key, handle, |missed| groups.handle_lost(key, &missed));

        if pending && entry {
            // SAFETY: the cancel only carries the key, it references no memory
            unsafe {
                core.prep_detached(|sqe| {
                    sqe.prep_cancel(key.as_raw(), IoringAsyncCancelFlags::empty());
                });
            };
        }
    }

    pub fn wait(&self) {
        let mut core = self.core.borrow_mut();
        let Core {
            ring,
            comp,
            active,
            scratch,
            groups,
        } = &mut *core;

        ring.submit_and_wait(1).expect(Self::FAILED_TO_ENTER_ERROR);

        ring.for_each_cqe(|cqe| {
            let Some((key, result)) = parse_cqe(cqe) else {
                return;
            };

            let stored = comp.notify(&key, result);

            if !result.has_more() {
                *active -= 1;
            }

            if !stored {
                groups.handle_lost(&key, &result);
            }
        });

        scratch.reset();

        groups.cleanup_dead(ring);
    }

    pub fn shutdown(&self) {
        while self.core.borrow().active > 0 {
            // SAFETY: the cancel references no memory
            unsafe {
                self.prep_detached(|sqe| {
                    sqe.prep_cancel(0, IoringAsyncCancelFlags::ANY);
                });
            };
            self.wait();
        }
    }

    pub(crate) fn create_buf_group(&self, count: u16, size: u32) -> Result<SelectGroup, Errno> {
        let core = &mut *self.core.borrow_mut();
        core.groups.create(count, size, &core.ring)
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        let core = self.core.get_mut();
        core.groups.leak_alive(&core.ring);
        let _ = core.ring.unregister_buffers();
        let _ = core.ring.unregister_files();
    }
}
