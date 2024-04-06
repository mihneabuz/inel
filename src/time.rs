use std::{
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};

use futures::{Future, Stream};
use io_uring::types::Timespec;

use crate::reactor::handle::RingHandle;

pub fn immediate<T>(value: T) -> Immediate<T> {
    Immediate { value: Some(value) }
}

pub struct Immediate<T> {
    value: Option<T>,
}

impl<T: Unpin> Future for Immediate<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(self.get_mut().value.take().unwrap())
    }
}

pub fn sleep(time: Duration) -> Timeout {
    Timeout::new(time)
}

pub struct Timeout {
    time: Timespec,
    ring: RingHandle,
}

impl Timeout {
    pub fn new(time: Duration) -> Self {
        Self {
            time: Timespec::new()
                .sec(time.as_secs())
                .nsec(time.subsec_nanos()),
            ring: RingHandle::default(),
        }
    }
}

impl Future for Timeout {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        unsafe { this.ring.timeout(cx, &this.time) }
    }
}

pub fn interval(time: Duration) -> Interval {
    Interval::new(time)
}

pub struct Interval {
    time: Timespec,
    ring: RingHandle,
}

impl Interval {
    pub fn new(time: Duration) -> Interval {
        Self {
            time: Timespec::new()
                .sec(time.as_secs())
                .nsec(time.subsec_nanos()),
            ring: RingHandle::default(),
        }
    }
}

impl Stream for Interval {
    type Item = ();

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        ready!(unsafe { this.ring.timeout(cx, &this.time) });
        Poll::Ready(Some(()))
    }
}
