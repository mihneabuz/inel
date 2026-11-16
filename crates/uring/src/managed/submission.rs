use core::{
    ffi::{CStr, c_char},
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, future::FusedFuture};

use bumpalo::Bump;

use crate::{
    managed::{
        UringProxy, UringProxyExt,
        cancellation::Cancellation,
        completion::{CompletionResult, Key},
    },
    sys::Sqe,
};

/// Bump allocator for [`Op::prep`], reset after every submit.
///
/// For memory that the kernel only reads while during `io_uring_enter`, and copies, like
/// the the path of an open, the address of a connect. It must not be used for anything
/// the kernel reads after submit (such as buffers), since it is reused by the next ops.
/// Values allocated here are never dropped.
#[derive(Debug, Default)]
pub struct Scratch {
    bump: Bump,
}

impl Scratch {
    /// Copy `s` (with its nul terminator) into scratch, for e.g. a path.
    pub fn alloc_cstr(&self, s: &CStr) -> *const c_char {
        self.bump
            .alloc_slice_copy(s.to_bytes_with_nul())
            .as_ptr()
            .cast()
    }

    pub(crate) fn reset(&mut self) {
        self.bump.reset();
    }
}

/// An op that can be submitted to the ring.
///
/// This trait describes how to submit and cancel an op in the ring, but taking out the
/// result requires also implementing either [`SingleOp`] or [`MultiOp`].
///
/// These should not be run manually, but instead wrapped in a [`Submission`] and driven
/// through the [`Future`] or [`Stream`] traits.
///
/// # Safety
///
/// Memory referenced by pointers written into the sqe by [`Self::prep`] must not be stored
/// inline in the op: the op may be moved after `prep`, even before the sqe is submitted. It
/// must also stay valid for as long as the kernel may access it, which is one of two cases:
///
/// - Memory the kernel accesses while the op runs (usually buffers) must be owned by the op
///   and stay valid until its final cqe is posted. If the op is cancelled, [`Self::cancel`]
///   must move it into the returned [`Cancellation`], which is kept until that cqe arrives.
///
/// - Memory the kernel only reads while preparing the request in `io_uring_enter`, and
///   copies (e.g. the path of an open, the address of a connect), only has to stay valid
///   until the next submit. It can be allocated from `scratch`, which lives exactly that
///   long, and [`Self::cancel`] doesn't need to keep it.
///
/// Implementations of Op must also be careful of aliasing rules, as it is not safe for
/// the kernel to touch memory that the op owns under strict aliasing. To avoid this
/// possibility, owned buffers can be wrapped in an [`InFlight`], which converts it into
/// a raw form, which is safe to share with the kernel, then converts it back to the
/// original type.
///
/// [`InFlight`]: super::buffer::InFlight
pub unsafe trait Op {
    /// Prep the op into `sqe`, and `scratch` can be used to allocate temporary storage
    /// which lives until `sqe` is submitted to the kernel.
    fn prep(&mut self, sqe: &mut Sqe, scratch: &Scratch);

    /// Turn the op into a [`Cancellation`] that keeps alive everything the kernel may
    /// still access, and return whether the ring should also submit an `AsyncCancel` for
    /// it (usually not worth it for ops that complete quickly).
    ///
    /// The default returns an empty [`Cancellation`] and `true`, which is only correct
    /// for ops that reference no owned memory after submit (see the trait's `# Safety`).
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
/// This trait only describes how an op handles its cqe results. The submission and
/// cancellation logic lives in the [`Op`] implementation.
///
/// [`Self::result`] gets the completion result and the ring the op ran on, so outputs
/// can be tied to their respective rings - which is useful for things like descriptors.
///
/// These should not be run manually, but instead wrapped in a [`Submission`] and driven
/// through the [`Future`] trait.
///
/// # Safety
///
/// The sqe prepped by the [`Op`] implementation must post exactly one cqe. The op, and
/// the memory it owns, is dropped once that cqe is consumed by [`Self::result`], so any
/// later completion would let the kernel access freed memory.
pub unsafe trait SingleOp<U: UringProxy>: Op {
    type Output;
    fn result(self, cqe: CompletionResult, ring: &U) -> Self::Output;
}

/// An op that may complete with multiple results.
///
/// This trait only describes how an op handles its cqe results. The submission and
/// cancellation logic lives in the [`Op`] implementation. This is the multishot
/// variant of [`SingleOp`].
///
/// [`Self::result`] gets the next completion result and the ring the op ran on, so
/// outputs can be tied to their respective rings - which is useful for things like
/// descriptors.
///
/// These should not be run manually, but instead wrapped in a [`Submission`] and driven
/// through the [`Stream`] trait. This trait is fully safe, as the [`Submission`] can
/// handle any number of completion results.
pub trait MultiOp<U: UringProxy>: Op {
    type Output;
    fn result(&mut self, cqe: CompletionResult, ring: &U) -> Self::Output;
}

/// The state of an op.
///
/// Initial -> Submitted -> Completed
///
/// - Initial - the future has not been polled yet, so the entry is not yet registered in
///   the kernel.
/// - Submitted - the entry was registered in the kernel and we have a key which can be
///   used to identify it.
/// - Completed - the entry was completed, all results taken, and the future is done, it
///   should not be polled anymore.
#[derive(Debug)]
enum SubmissionState {
    Initial,
    Submitted(Key),
    Completed,
}

/// Future which drives an op when polled.
///
/// When driving an `O: SingleOp`, it implements [`Future`].
/// When driving an `O: MultiOp`, it implements [`Stream`].
///
/// It is created with [`Op::run_on`]. Constructing it does not submit the op to the
/// ring - as with other futures, it must be polled before it does any work.
///
/// Polling it drives the state machine, by submitting the op and checking for results
/// in the ring. Dropping it while it still has an entry submitted to the ring will
/// cancel it, by converting the op into a [Cancellation].
///
/// This is a fully safe interface, with the op implementation guaranteeing it.
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Submission<O: Op, U: UringProxy> {
    op: ManuallyDrop<O>,
    state: SubmissionState,
    uring: U,
}

/// [`Op`] guarantees that data passed to the kernel by pointer is not stored inline
/// so this is safe for any `O`.
impl<O: Op, U: UringProxy> Unpin for Submission<O, U> {}

/// Dropping a submission must cancel its inner `O`, but only in the `Submitted` state.
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
                self.uring.cancel(key, handle, entry);
            }
            SubmissionState::Completed => {
                // SAFETY: the op was already dropped
            }
        }
    }
}

impl<O: Op, U: UringProxy> Submission<O, U> {
    /// Wrap `op` and bind it to `uring`.
    pub(crate) const fn new(op: O, uring: U) -> Self {
        Self {
            op: ManuallyDrop::new(op),
            state: SubmissionState::Initial,
            uring,
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

/// For an `O: SingleOp` implement [`Future`] to drive it.
impl<O: SingleOp<U>, U: UringProxy> Future for Submission<O, U> {
    type Output = O::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        match this.state {
            SubmissionState::Initial => {
                let key = this.uring.submit(&mut *this.op, cx.waker().clone());
                this.state = SubmissionState::Submitted(key);
            }

            SubmissionState::Submitted(key) => {
                if let Some(result) = this.uring.result(key) {
                    this.state = SubmissionState::Completed;
                    // SAFETY: we got the completion of the op so there are no more
                    // references to it in the kernel, and the state is Completed
                    let op = unsafe { this.take_op() };
                    return Poll::Ready(op.result(result, &this.uring));
                }

                // the future may have moved to another task since the last poll
                this.uring.update(key, cx.waker());
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

/// For an `O: MultiOp` implement [`Stream`] to drive it.
impl<O: MultiOp<U>, U: UringProxy> Stream for Submission<O, U> {
    type Item = O::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        match this.state {
            SubmissionState::Initial => {
                let key = this.uring.submit_multi(&mut *this.op, cx.waker().clone());
                this.state = SubmissionState::Submitted(key);
                Poll::Pending
            }

            #[allow(clippy::option_if_let_else)]
            SubmissionState::Submitted(key) => match this.uring.result(key) {
                Some(result) => {
                    let has_more = result.has_more();
                    let res = this.op.result(result, &this.uring);
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
                    this.uring.update(key, cx.waker());
                    Poll::Pending
                }
            },

            SubmissionState::Completed => Poll::Ready(None),
        }
    }
}
