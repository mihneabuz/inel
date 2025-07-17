use std::{
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::FusedFuture, Future, Stream};
use inel_interface::Reactor;
use pin_project_lite::pin_project;

use crate::{
    op::{MultiOp, Op},
    ring::{Key, Ring},
    RingReactor,
};

pub enum SubmissionState {
    Initial,
    Submitted(Key),
    Completed,
}

impl SubmissionState {
    pub fn take(&mut self) -> Self {
        std::mem::replace(self, Self::Initial)
    }

    pub fn update(&mut self, state: Self) {
        *self = state;
    }
}

pin_project! {
    /// State machine that drives an [Op] to completion.
    pub struct Submission<T: Op, R: Reactor<Handle = Ring>> {
        op: ManuallyDrop<T>,
        state: SubmissionState,
        reactor: R,
    }

    impl<T, R> PinnedDrop for Submission<T, R>
    where
        T: Op,
        R: Reactor<Handle = Ring>,
    {
        fn drop(mut this: Pin<&mut Self>) {
            this.cancel();
        }
    }
}

impl<T, R> Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    pub fn new(reactor: R, op: T) -> Self {
        Self {
            op: ManuallyDrop::new(op),
            state: SubmissionState::Initial,
            reactor,
        }
    }

    fn cancel(&mut self) {
        if let SubmissionState::Submitted(key) = self.state {
            let op = unsafe { ManuallyDrop::take(&mut self.op) };
            let cancel = op.cancel();
            let entry = T::entry_cancel(key.as_u64());
            unsafe { self.reactor.cancel(key, entry, cancel) };
        }
    }
}

impl<T, R> Future for Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    type Output = T::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (res, next_state) = match self.state.take() {
            SubmissionState::Initial => {
                let entry = self.op.entry();
                let waker = cx.waker().clone();
                let key = unsafe { self.reactor.submit(entry, waker) };

                (Poll::Pending, SubmissionState::Submitted(key))
            }

            SubmissionState::Submitted(key) => match self.reactor.check_result(key) {
                None => (Poll::Pending, SubmissionState::Submitted(key)),
                Some(result) => {
                    let op = unsafe { ManuallyDrop::take(&mut self.op) };
                    let next = if result.has_more() {
                        SubmissionState::Submitted(key)
                    } else {
                        SubmissionState::Completed
                    };

                    (Poll::Ready(op.result(result)), next)
                }
            },

            SubmissionState::Completed => {
                panic!("Polled already completed Submission");
            }
        };

        self.state.update(next_state);
        res
    }
}

impl<T, R> FusedFuture for Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    fn is_terminated(&self) -> bool {
        matches!(self.state, SubmissionState::Completed)
    }
}

impl<T, R> Stream for Submission<T, R>
where
    T: MultiOp,
    R: Reactor<Handle = Ring>,
{
    type Item = T::Output;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let (res, next_state) = match self.state.take() {
            SubmissionState::Initial => {
                let entry = self.op.entry();
                let waker = cx.waker().clone();
                let key = unsafe { self.reactor.submit(entry, waker) };

                (Poll::Pending, SubmissionState::Submitted(key))
            }

            SubmissionState::Submitted(key) => match self.reactor.check_result(key) {
                None => (Poll::Pending, SubmissionState::Submitted(key)),
                Some(result) => {
                    let next = if result.has_more() {
                        SubmissionState::Submitted(key)
                    } else {
                        SubmissionState::Completed
                    };

                    (Poll::Ready(Some(self.op.next(result))), next)
                }
            },

            SubmissionState::Completed => (Poll::Ready(None), SubmissionState::Completed),
        };

        self.state.update(next_state);
        res
    }
}
