use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::FusedFuture, Future};
use inel_interface::Reactor;
use pin_project_lite::pin_project;

use crate::{completion::Key, op::Op, reactor::RingReactor, Ring};

pub enum SubmissionState {
    Initial,
    Submitted(Key),
    Completed,
}

impl SubmissionState {
    pub fn take(&mut self) -> Self {
        std::mem::replace(self, Self::Initial)
    }
}

pin_project! {
    pub struct Submission<T: Op, R: Reactor<Handle = Ring>> {
        op: Option<T>,
        state: SubmissionState,
        react: R,
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
                unsafe { this.react.cancel(key, entry, cancel) };
            }
        }
    }
}

impl<T, R> Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    pub fn new(react: R, op: T) -> Self {
        Self {
            op: Some(op),
            state: SubmissionState::Initial,
            react,
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

        let (ret, next_state) = match this.state.take() {
            SubmissionState::Initial => {
                let entry = this.op.as_mut().unwrap().entry();
                let waker = cx.waker().clone();
                let key = unsafe { this.react.submit(entry, waker) };

                (Poll::Pending, SubmissionState::Submitted(key))
            }

            SubmissionState::Submitted(key) => match this.react.check_result(key) {
                None => (Poll::Pending, SubmissionState::Submitted(key)),
                Some(ret) => {
                    let op = this.op.take().unwrap();
                    (Poll::Ready(op.result(ret)), SubmissionState::Completed)
                }
            },

            SubmissionState::Completed => {
                panic!("Polled already completed Submission");
            }
        };

        *this.state = next_state;

        ret
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
