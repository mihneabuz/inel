use core::{cell::RefCell, task::Waker};

use rustix::{io::Errno, io_uring::IoringAsyncCancelFlags};

use crate::{
    managed::{
        SelectGroup, UringProxy,
        cancellation::Cancellation,
        completion::{CompletionResult, Completions, parse_cqe},
        group::BufGroups,
        key::{Hints, Key},
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
            core: RefCell::new(UringCore {
                ring,
                comp,
                active: 0,
                scratch: Scratch::default(),
                lost_fds: LostFdList::new(),
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
    core: RefCell<UringCore>,
}

struct UringCore {
    ring: IoUring,
    comp: Completions,
    active: u32,
    scratch: Scratch,
    lost_fds: LostFdList,
    groups: BufGroups,
}

impl UringCore {
    const SUBMISSION_QUEUE_FULL_ERROR: &str =
        "Submission queue full. Consider configuring with more `sq_entries`.";
    const FAILED_TO_ENTER_ERROR: &str = "Failed to enter io_uring";

    fn prep<O: Op>(&mut self, op: &mut O, key: &mut Key) {
        let mut hints = Hints::empty();

        // SAFETY: guaranteed by the `Op` impl
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe, &mut hints, &self.scratch);
                    key.set_hints(hints);
                    sqe.user_data(key.as_raw());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }

        self.active += 1;
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
                    sqe.user_data(Key::ignored().as_raw());
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    fn cancel(&mut self, key: &Key, handle: Cancellation, entry: bool) {
        let pending = self.comp.cancel(key, handle, |missed| {
            if let Some(gid) = key.hints().used_group_id() {
                self.groups.handle_lost(gid, &missed);
            }

            if key.hints().creates_any_fd() {
                self.lost_fds.push_lost(&key.hints(), &missed);
            }
        });

        if pending && entry {
            // SAFETY: the cancel only carries the key, it references no memory
            unsafe {
                self.prep_detached(|sqe| {
                    sqe.prep_cancel(key.as_raw(), IoringAsyncCancelFlags::empty());
                });
            };
        }
    }

    fn wait(&mut self) {
        self.close_lost_fds();

        self.ring
            .submit_and_wait(1)
            .expect(Self::FAILED_TO_ENTER_ERROR);

        self.ring.for_each_cqe(|cqe| {
            let Some((key, result)) = parse_cqe(cqe) else {
                return;
            };

            let stored = self.comp.notify(&key, result);

            if !result.has_more() {
                self.active -= 1;
            }

            if !stored {
                if let Some(gid) = key.hints().used_group_id() {
                    self.groups.handle_lost(gid, &result);
                }

                if key.hints().creates_any_fd() {
                    self.lost_fds.push_lost(&key.hints(), &result);
                }
            }
        });

        self.scratch.reset();

        self.groups.cleanup_dead(&self.ring);
    }

    fn shutdown(&mut self) {
        while self.active > 0 {
            // SAFETY: the cancel references no memory
            unsafe {
                self.prep_detached(|sqe| {
                    sqe.prep_cancel(0, IoringAsyncCancelFlags::ANY);
                });
            };
            self.wait();
        }
    }

    fn close_lost_fds(&mut self) {
        while let Some(lost) = self.lost_fds.pop() {
            // SAFETY: the close references no memory
            unsafe {
                self.prep_detached(|sqe| lost.prep_close(sqe));
            }
        }
    }
}

impl Drop for UringCore {
    fn drop(&mut self) {
        self.lost_fds.close_all_sync();
        self.groups.leak_alive(&self.ring);
        let _ = self.ring.unregister_buffers();
        let _ = self.ring.unregister_files();
    }
}

impl Uring {
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

    /// Enters the ring and waits for events.
    pub fn wait(&self) {
        self.core.borrow_mut().wait();
    }

    /// Attempts to shutdown the ring by cancelling all in-flight ops.
    pub fn shutdown(&self) {
        self.core.borrow_mut().shutdown();
    }

    /// Submits an SQE built by [`O::prep`] and creates a completion handler for it.
    /// Returns a `Key` which can be used to retrieve the result of the op.
    pub(crate) fn submit<U, O>(&self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: SingleOp<U>,
    {
        let mut core = self.core.borrow_mut();
        // SAFETY: guaranteed by the `SingleOp` impl
        let mut key = unsafe { core.comp.insert_single(waker) };
        core.prep(op, &mut key);
        key
    }

    /// Submits an SQE built by [`O::prep`] and creates a completion handler for it.
    /// Returns a `Key` which can be used to retrieve the results of the op.
    pub(crate) fn submit_multi<U, O>(&self, op: &mut O, waker: Waker) -> Key
    where
        U: UringProxy,
        O: MultiOp<U>,
    {
        let mut core = self.core.borrow_mut();
        let mut key = core.comp.insert_multi(waker);
        core.prep(op, &mut key);
        key
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
        self.core.borrow_mut().cancel(key, handle, entry);
    }

    pub(crate) fn create_buf_group(&self, count: u16, size: u32) -> Result<SelectGroup, Errno> {
        let core = &mut *self.core.borrow_mut();
        core.groups.create(count, size, &core.ring)
    }
}

enum LostFd {
    Raw(i32),
    Direct(u32),
}

impl LostFd {
    const fn prep_close(&self, sqe: &mut Sqe) {
        match *self {
            Self::Raw(fd) => sqe.prep_close(fd),
            Self::Direct(fd) => sqe.prep_close_direct(fd),
        }
    }
}

struct LostFdList {
    list: Vec<LostFd>,
}

impl LostFdList {
    const fn new() -> Self {
        Self { list: Vec::new() }
    }

    #[cold]
    fn push_lost(&mut self, hints: &Hints, result: &CompletionResult) {
        let raw = result.result();
        if raw < 0 {
            return;
        }

        if hints.creates_raw_fd() {
            self.list.push(LostFd::Raw(raw));
        } else if hints.creates_direct_fd() {
            self.list.push(LostFd::Direct(raw as u32))
        }
    }

    fn pop(&mut self) -> Option<LostFd> {
        self.list.pop()
    }

    fn close_all_sync(&mut self) {
        for lost in self.list.drain(..) {
            if let LostFd::Raw(fd) = lost {
                unsafe { rustix::io::close(fd) };
            }
        }
    }
}
