use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::FusedFuture, Future, Stream};
use inel_interface::Reactor;
use pin_project_lite::pin_project;

use crate::{
    op::{MultiOp, Op},
    Key, Ring, RingReactor,
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
        op: Option<T>,
        state: SubmissionState,
        reactor: R,
    }

    impl<T, R> PinnedDrop for Submission<T, R>
    where
        T: Op,
        R: Reactor<Handle = Ring>,
    {
        fn drop(this: Pin<&mut Self>) {
            if let SubmissionState::Submitted(key) = this.state {
                let this = this.project();
                let (entry, cancel) = this.op.take().unwrap().cancel(key.as_u64());
                unsafe { this.reactor.cancel(key, entry, cancel) };
            }
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
            op: Some(op),
            state: SubmissionState::Initial,
            reactor,
        }
    }
}

impl<T, R> Future for Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let (res, next_state) = match this.state.take() {
            SubmissionState::Initial => {
                let entry = this.op.as_mut().unwrap().entry();
                let waker = cx.waker().clone();
                let key = unsafe { this.reactor.submit(entry, waker) };

                (Poll::Pending, SubmissionState::Submitted(key))
            }

            SubmissionState::Submitted(key) => match this.reactor.check_result(key) {
                None => (Poll::Pending, SubmissionState::Submitted(key)),
                Some((ret, _)) => {
                    let op = this.op.take().unwrap();
                    (Poll::Ready(op.result(ret)), SubmissionState::Completed)
                }
            },

            SubmissionState::Completed => {
                panic!("Polled already completed Submission");
            }
        };

        this.state.update(next_state);
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

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let (res, next_state) = match this.state.take() {
            SubmissionState::Initial => {
                let entry = this.op.as_mut().unwrap().entry();
                let waker = cx.waker().clone();
                let key = unsafe { this.reactor.submit(entry, waker) };

                (Poll::Pending, SubmissionState::Submitted(key))
            }

            SubmissionState::Submitted(key) => match this.reactor.check_result(key) {
                None => (Poll::Pending, SubmissionState::Submitted(key)),
                Some((ret, has_more)) => {
                    let res = Poll::Ready(this.op.as_ref().map(|op| op.next(ret)));

                    (
                        res,
                        if has_more {
                            SubmissionState::Submitted(key)
                        } else {
                            SubmissionState::Completed
                        },
                    )
                }
            },

            SubmissionState::Completed => (Poll::Ready(None), SubmissionState::Completed),
        };

        this.state.update(next_state);
        res
    }
}
