use core::{
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, future::FusedFuture};
use rustix::io_uring::IoringAsyncCancelFlags;

use crate::{
    managed::{
        UringProxy, UringProxyExt,
        cancellation::Cancellation,
        completion::{CompletionResult, Key},
    },
    sys::Sqe,
};

enum SubmissionState {
    Initial,
    Submitted(Key),
    Completed,
}

pub struct Submission<O: Op, U: UringProxy> {
    op: ManuallyDrop<O>,
    state: SubmissionState,
    ring: U,
}

// Nothing is structurally pinned: whatever the kernel references lives outside
// the op (see `Op`), so the op can move while in flight.
impl<O: Op, U: UringProxy> Unpin for Submission<O, U> {}

impl<O: Op, U: UringProxy> Drop for Submission<O, U> {
    fn drop(&mut self) {
        match self.state {
            SubmissionState::Initial => {
                // SAFETY: the op was never submitted so there are no active references
                unsafe { ManuallyDrop::drop(&mut self.op) };
            }
            SubmissionState::Submitted(key) => {
                // SAFETY: the op was submitted and there may be active references
                // but the Op implementation guarantees that they will be stored
                // in the Cancellation handle for deffered destruction
                let op = unsafe { ManuallyDrop::take(&mut self.op) };
                let (handle, entry) = op.cancel();
                self.ring.cancel(key, handle, entry);
            }
            SubmissionState::Completed => {
                // SAFETY: the op was already dropped
            }
        }
    }
}

impl<O: Op, U: UringProxy> Submission<O, U> {
    pub const fn new(op: O, ring: U) -> Self {
        Self {
            op: ManuallyDrop::new(op),
            state: SubmissionState::Initial,
            ring,
        }
    }

    /// # Safety
    /// The op must not be used afterwards.
    unsafe fn take_op(&mut self) -> O {
        unsafe { ManuallyDrop::take(&mut self.op) }
    }

    /// # Safety
    /// The op must not be used afterwards.
    unsafe fn drop_op(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.op) }
    }
}

impl<O: SingleOp, U: UringProxy> Future for Submission<O, U> {
    type Output = O::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.state {
            SubmissionState::Initial => {
                let key = this.ring.submit(&mut *this.op, cx.waker().clone());
                this.state = SubmissionState::Submitted(key);
            }

            SubmissionState::Submitted(key) => {
                if let Some(result) = this.ring.result(key) {
                    this.state = SubmissionState::Completed;
                    // SAFETY: we got the completion of the op so there are no more
                    // references to it in the kernel, and the state is Completed
                    let op = unsafe { this.take_op() };
                    return Poll::Ready(op.result(result));
                }

                // the future may have moved to another task since the last poll
                this.ring.update(key, cx.waker());
            }

            SubmissionState::Completed => panic!("Polled already completed submission"),
        };

        Poll::Pending
    }
}

impl<O: SingleOp, U: UringProxy> FusedFuture for Submission<O, U> {
    fn is_terminated(&self) -> bool {
        matches!(self.state, SubmissionState::Completed)
    }
}

impl<O: MultiOp, U: UringProxy> Stream for Submission<O, U> {
    type Item = O::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.state {
            SubmissionState::Initial => {
                let key = this.ring.submit_multi(&mut *this.op, cx.waker().clone());
                this.state = SubmissionState::Submitted(key);
                Poll::Pending
            }

            #[allow(clippy::option_if_let_else)]
            SubmissionState::Submitted(key) => match this.ring.result(key) {
                Some(result) => {
                    let has_more = result.has_more();
                    let res = this.op.result(result);
                    if !has_more {
                        this.state = SubmissionState::Completed;
                        // SAFETY: we got the last completion of the op so there are
                        // no more references to it in the kernel, and the state is
                        // Completed
                        unsafe { this.drop_op() };
                    }
                    Poll::Ready(Some(res))
                }
                None => {
                    // the stream may have moved to another task since the last poll
                    this.ring.update(key, cx.waker());
                    Poll::Pending
                }
            },

            SubmissionState::Completed => Poll::Ready(None),
        }
    }
}

/// An operation that can be submitted to the ring.
///
/// # Safety
/// Everything the SQE built by [`Self::prep`] references (buffers, paths,
/// timespecs, ...) must be owned by the op, but not stored inline in it: the
/// op may be moved while in flight. That memory must stay valid until the op's
/// final CQE, which is when the op is consumed by its `result`, or, if the op
/// is cancelled first, until the [`Cancellation`] returned by [`Self::cancel`]
/// is dropped.
pub unsafe trait Op {
    fn prep(&mut self, sqe: &mut Sqe);

    /// Hands over everything the kernel may still reference, to be freed once
    /// the op completes, and whether an `AsyncCancel` should be submitted.
    fn cancel(self) -> (Cancellation, bool)
    where
        Self: Sized,
    {
        (Cancellation::empty(), true)
    }

    fn run_on<U: UringProxy>(self, uring: U) -> Submission<Self, U>
    where
        Self: Sized,
    {
        Submission::new(self, uring)
    }
}

/// An op that completes with a single CQE.
///
/// # Safety
/// The SQE built by [`Op::prep`] must complete with exactly one CQE, so it
/// must not request a multishot variant.
pub unsafe trait SingleOp: Op {
    type Output;
    fn result(self, cqe: CompletionResult) -> Self::Output;
}

/// An op that may complete with several CQEs, the last one without
/// `IORING_CQE_F_MORE`.
pub trait MultiOp: Op {
    type Output;
    fn result(&mut self, cqe: CompletionResult) -> Self::Output;
}

/// An op submitted without waiting for its completion, which is ignored.
///
/// # Safety
/// The op is dropped right after [`Self::prep_detached`], so the SQE must not
/// reference any memory owned by it. It must also not select a buffer from a
/// buffer group, since the completion carrying it is ignored.
pub unsafe trait DetachOp: Op {
    fn run_detached<U: UringProxy>(self, mut uring: U)
    where
        Self: Sized,
    {
        uring.submit_detached(self);
    }
}

pub struct AsyncCancel {
    user_data: u64,
    flags: IoringAsyncCancelFlags,
}

impl AsyncCancel {
    /// Cancels the op submitted with `key`.
    pub const fn new(key: Key) -> Self {
        Self {
            user_data: key.as_raw(),
            flags: IoringAsyncCancelFlags::empty(),
        }
    }

    /// Cancels every op in flight.
    pub const fn all() -> Self {
        Self {
            user_data: 0,
            flags: IoringAsyncCancelFlags::ANY,
        }
    }
}

// SAFETY: the SQE only carries the key, it references no memory
unsafe impl Op for AsyncCancel {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_cancel(self.user_data, self.flags);
    }

    fn cancel(self) -> (Cancellation, bool) {
        unreachable!();
    }
}

// SAFETY: see `Op`
unsafe impl DetachOp for AsyncCancel {}
