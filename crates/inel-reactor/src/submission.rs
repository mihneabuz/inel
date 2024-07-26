use std::{
    mem::ManuallyDrop,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{future::FusedFuture, Future};
use inel_interface::Reactor;

use crate::{completion::Key, op::Op, Ring};

pub enum SubmissionState {
    Init,
    Submitted(Key),
    Completed,
}

pub struct Submission<T: Op, R: Reactor<Handle = Ring>> {
    op: Option<ManuallyDrop<T>>,
    state: SubmissionState,
    reactor: R,
}

impl<T, R> Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    pub fn new(reactor: R, op: T) -> Self {
        Self {
            op: Some(ManuallyDrop::new(op)),
            state: SubmissionState::Init,
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
        let this = unsafe { Pin::into_inner_unchecked(self) };

        match &mut this.state {
            SubmissionState::Init => {
                let entry = this.op.as_mut().unwrap().entry();
                let key = this
                    .reactor
                    .with(|reactor| unsafe { reactor.submit(entry, cx.waker().clone()) });

                this.state = SubmissionState::Submitted(key);

                cx.waker().wake_by_ref();

                Poll::Pending
            }

            SubmissionState::Submitted(key) => {
                let Some(ret) = this.reactor.with(|reactor| reactor.check_result(*key)) else {
                    return Poll::Pending;
                };

                let op = ManuallyDrop::into_inner(this.op.take().unwrap());

                this.state = SubmissionState::Completed;

                Poll::Ready(op.result(ret))
            }

            SubmissionState::Completed => {
                panic!("Polled completed Submission");
            }
        }
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

impl<T, R> Drop for Submission<T, R>
where
    T: Op,
    R: Reactor<Handle = Ring>,
{
    fn drop(&mut self) {
        match self.state {
            SubmissionState::Init => {}
            SubmissionState::Completed => {}
            SubmissionState::Submitted(key) => {
                if let Some((entry, cancel)) = self.op.take().unwrap().cancel(key.as_u64()) {
                    self.reactor
                        .with(|reactor| unsafe { reactor.cancel(entry, key, cancel) });
                }
            }
        }
    }
}
