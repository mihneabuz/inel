use core::{
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, future::FusedFuture};

use crate::{
    managed::{
        UringProxy, UringProxyExt,
        cancellation::Cancellation,
        completion::{CompletionResult, Key},
    },
    sys::Sqe,
};

/// An operation that can be submitted to the ring.
///
/// This trait describes how to submit and cancel an op in the ring.
///
/// These should not be run manually, but instead wrapped in a [`Submission`] and driven
/// through the [`Future`] or [`Stream`] traits.
///
/// As such, implementing just Op on a type is not very usefull, it must be followed by
/// a [SingleOp] or [MultiOp] implementation (or both).
///
/// # Safety
/// 
/// Everything that the SQE built by [`Self::prep`] references by pointer must be owned
/// by the op, but not stored inline in it: the op may be moved while in flight.
///
/// That memory must stay valid until the op's completion, which is when the last result
/// will be posted on the ring, or when it is cancelled and turned into a [`Cancellation`].
/// The [`Cancellation`] handle must also keep that memory alive until it's [`drop_raw`]
/// method is called.
///
/// Implementations of Op must also be carefull of aliasing rules, as it is not safe for
/// the kernel to touch memory that the op owns under strict aliasing. To avoid this
/// possibility, buffers can be wrapped in an [`InFlight`], which converts it into a raw
/// form, which is safe to share with the kernel, then converts it back to the original
/// type.
/// 
pub unsafe trait Op {
    /// Prep the op into `sqe`
    fn prep(&mut self, sqe: &mut Sqe);

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

/// An op that completes with a single result.
///
/// `result` gets the ring the op ran on, so outputs tied to it (like a
/// [`DirectFd`](super::DirectFd)) can hold it without the op carrying its own.
///
pub unsafe trait SingleOp<U: UringProxy>: Op {
    type Output;
    fn result(self, cqe: CompletionResult, ring: &U) -> Self::Output;
}

/// An op that may complete with multiple results.
pub trait MultiOp<U: UringProxy>: Op {
    type Output;
    fn result(&mut self, cqe: CompletionResult, ring: &U) -> Self::Output;
}

/// The state of an op.
///
/// Initial -> Submitted -> Completed
///
/// Initial - the future has not been polled yet, so the entry is not yet
/// registered in the kernel.
/// Submitted - the entry was registered in the kernel and we have a key
/// which we can use to identify it.
/// Completed - the entry was completed, all results taken, and the future
/// is done, it should not be polled anymore.
#[derive(Debug)]
enum SubmissionState {
    Initial,
    Submitted(Key),
    Completed,
}

/// Future which drives an op when polled.
///
/// When driving an `O: SingleOp`, it implementes Future.
/// When driving an `O: MultiOp`, it implements Stream.
///
/// Constructing it does not submit the op to the ring, as with other futures,
/// it must be polled before it does any work.
///
/// Polling it drives the state machine, by submitting the op and checking for
/// results in the ring. Dropping it while it still has an entry submitted to
/// the ring will cancel it, by converting the op into a [Cancellation].
///
/// This is a fully-safe interface, with the op implementation guaranteeing it.
#[derive(Debug)]
pub struct Submission<O: Op, U: UringProxy> {
    op: ManuallyDrop<O>,
    state: SubmissionState,
    ring: U,
}

impl<O: Op, U: UringProxy> Unpin for Submission<O, U> {}

impl<O: Op, U: UringProxy> Drop for Submission<O, U> {
    fn drop(&mut self) {
        match self.state {
            SubmissionState::Initial => {
                // SAFETY: the op was never submitted so there are no active references.
                unsafe { ManuallyDrop::drop(&mut self.op) };
            }
            SubmissionState::Submitted(key) => {
                // SAFETY: the op was submitted and there may be active references but
                // the Op implementation guarantees that they will be stored in the
                // Cancellation handle for deffered destruction
                let op = unsafe { self.take_op() };
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

impl<O: SingleOp<U>, U: UringProxy> Future for Submission<O, U> {
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
                    return Poll::Ready(op.result(result, &this.ring));
                }

                // the future may have moved to another task since the last poll
                this.ring.update(key, cx.waker());
            }

            SubmissionState::Completed => panic!("Polled already completed submission"),
        };

        Poll::Pending
    }
}

impl<O: SingleOp<U>, U: UringProxy> FusedFuture for Submission<O, U> {
    fn is_terminated(&self) -> bool {
        matches!(self.state, SubmissionState::Completed)
    }
}

impl<O: MultiOp<U>, U: UringProxy> Stream for Submission<O, U> {
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
                    let res = this.op.result(result, &this.ring);
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
