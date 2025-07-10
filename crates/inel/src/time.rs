use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{future::FusedFuture, FutureExt};
use inel_reactor::{
    op::{self, OpExt},
    submission::Submission,
};

use crate::GlobalReactor;

pub fn sleep(time: Duration) -> Timeout {
    Timeout::new(time)
}

pub struct Timeout {
    sub: Submission<op::Timeout, GlobalReactor>,
}

impl Timeout {
    pub fn new(time: Duration) -> Self {
        Self {
            sub: op::Timeout::new(time).run_on(GlobalReactor),
        }
    }
}

impl Future for Timeout {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl FusedFuture for Timeout {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}

pub fn instant() -> Instant {
    Instant::default()
}

pub struct Instant {
    sub: Submission<op::Nop, GlobalReactor>,
}

impl Instant {
    pub fn new() -> Self {
        Self {
            sub: op::Nop.run_on(GlobalReactor),
        }
    }
}

impl Default for Instant {
    fn default() -> Self {
        Self::new()
    }
}

impl Future for Instant {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::into_inner(self).sub.poll_unpin(cx)
    }
}

impl FusedFuture for Instant {
    fn is_terminated(&self) -> bool {
        self.sub.is_terminated()
    }
}
