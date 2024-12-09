use std::{
    pin::pin,
    task::Poll,
    time::{Duration, Instant},
};

use crate::helpers::{poll, runtime};
use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

#[test]
fn single() {
    let (reactor, notifier) = runtime();

    let mut timeout = op::Timeout::new(Duration::from_millis(10)).run_on(reactor.clone());
    let mut fut = pin!(&mut timeout);

    let start = Instant::now();
    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut, notifier), Poll::Ready(()));
    assert!(fut.is_terminated());

    assert!(reactor.is_done());

    assert!(10 <= start.elapsed().as_millis());
}

#[test]
fn multi() {
    let (reactor, notifier) = runtime();

    let mut timeout1 = op::Timeout::new(Duration::from_millis(10)).run_on(reactor.clone());
    let mut timeout2 = op::Timeout::new(Duration::from_millis(50)).run_on(reactor.clone());
    let mut fut1 = pin!(&mut timeout1);
    let mut fut2 = pin!(&mut timeout2);

    let start = Instant::now();

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut1, notifier), Poll::Ready(()));
    assert!(fut1.is_terminated());
    assert_eq!(reactor.active(), 1);

    assert!(10 <= start.elapsed().as_millis());

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut2, notifier), Poll::Ready(()));
    assert!(fut2.is_terminated());
    assert_eq!(reactor.active(), 0);

    assert!(50 <= start.elapsed().as_millis());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();

    let mut timeout1 = op::Timeout::new(Duration::from_millis(2000)).run_on(reactor.clone());
    let mut timeout2 = op::Timeout::new(Duration::from_millis(50)).run_on(reactor.clone());
    let mut fut1 = pin!(&mut timeout1);
    let mut fut2 = pin!(&mut timeout2);

    let start = Instant::now();

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    drop(timeout1);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut2, notifier), Poll::Ready(()));
    assert!(fut2.is_terminated());

    assert!(reactor.is_done());

    assert!(50 <= start.elapsed().as_millis());
}

#[test]
fn forget() {
    let (reactor, notifier) = runtime();

    let mut timeout = op::Timeout::new(Duration::from_millis(50)).run_on(reactor.clone());
    let mut fut = pin!(&mut timeout);

    let start = Instant::now();
    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    std::mem::forget(fut);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert!(50 <= start.elapsed().as_millis());
}
