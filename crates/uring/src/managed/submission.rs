use core::{
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, future::FusedFuture};
use rustix::io_uring::IoringAsyncCancelFlags;

use crate::{
    managed::{
        UringProxy,
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

    fn project(self: Pin<&mut Self>) -> (Pin<&mut O>, &mut SubmissionState, &U) {
        // SAFETY: only the inner op must be pinned
        unsafe {
            let this = self.get_unchecked_mut();
            (
                Pin::new_unchecked(&mut this.op),
                &mut this.state,
                &this.ring,
            )
        }
    }

    unsafe fn take_op(self: Pin<&mut Self>) -> O {
        unsafe { ManuallyDrop::take(&mut self.get_unchecked_mut().op) }
    }

    unsafe fn drop_op(self: Pin<&mut Self>) {
        unsafe { ManuallyDrop::drop(&mut self.get_unchecked_mut().op) }
    }
}

impl<O: SingleOp, U: UringProxy> Future for Submission<O, U> {
    type Output = O::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (op, state, ring) = self.as_mut().project();

        match state {
            SubmissionState::Initial => {
                let key = ring.submit(op, cx.waker().clone());
                *state = SubmissionState::Submitted(key);
            }

            SubmissionState::Submitted(key) => {
                if let Some(result) = ring.result(*key) {
                    *state = SubmissionState::Completed;
                    // SAFETY: we got the completion of the op so there are no more
                    // references to it in the kernel
                    let op = unsafe { self.take_op() };
                    return Poll::Ready(op.result(result));
                }
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

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (op, state, ring) = self.as_mut().project();

        match state {
            SubmissionState::Initial => {
                let key = ring.submit_multi(op, cx.waker().clone());
                *state = SubmissionState::Submitted(key);
                Poll::Pending
            }

            #[allow(clippy::option_if_let_else)]
            SubmissionState::Submitted(key) => match ring.result(*key) {
                Some(result) => {
                    let has_more = result.has_more();
                    let res = op.result(result);
                    if !has_more {
                        *state = SubmissionState::Completed;
                        // SAFETY: we got the last completion of the op so there are
                        //  no more references to it in the kernel
                        unsafe { self.drop_op() };
                    }
                    Poll::Ready(Some(res))
                }
                None => Poll::Pending,
            },

            SubmissionState::Completed => Poll::Ready(None),
        }
    }
}

pub trait Op {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe);

    fn cancel(self) -> (Cancellation, bool)
    where
        Self: Sized,
    {
        (Cancellation::empty(), true)
    }

    fn run_on<U: UringProxy>(self, ring: U) -> Submission<Self, U>
    where
        Self: Sized,
    {
        Submission::new(self, ring)
    }
}

pub trait SingleOp: Op {
    type Output;
    fn result(self, cqe: CompletionResult) -> Self::Output;
}

pub trait MultiOp: Op {
    type Output;
    fn result(self: Pin<&mut Self>, cqe: CompletionResult) -> Self::Output;
}

pub trait DetachOp: Op + Unpin {
    fn prep_detached(&mut self, sqe: &mut Sqe) {
        unsafe { Pin::new(self).prep(sqe) }
    }

    fn run_detached<U: UringProxy>(self, mut ring: U)
    where
        Self: Sized,
    {
        ring.submit_detached(self);
    }
}

pub struct AsyncCancel {
    key: Key,
}

impl AsyncCancel {
    pub const fn new(key: Key) -> Self {
        Self { key }
    }
}

impl Op for AsyncCancel {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_cancel(self.key.as_u64(), IoringAsyncCancelFlags::empty());
    }

    fn cancel(self) -> (Cancellation, bool) {
        unreachable!();
    }
}

impl DetachOp for AsyncCancel {}
