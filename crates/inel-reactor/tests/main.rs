mod helpers;

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::future::FusedFuture;
use helpers::runtime;
use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

macro_rules! poll {
    ($fut:expr, $notifier:expr) => {{
        let waker = $notifier.waker();
        let mut context = Context::from_waker(&waker);
        $fut.as_mut().poll(&mut context)
    }};
}

#[test]
fn sanity() {
    let (reactor, notifier) = runtime();

    let mut nop = op::Nop.run_on(reactor.clone());
    let mut fut = pin!(&mut nop);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut, notifier), Poll::Ready(()));
    assert!(fut.is_terminated());
    assert_eq!(reactor.active(), 0);
}

#[test]
fn single_timeout() {
    let (reactor, notifier) = runtime();

    let start = Instant::now();

    let mut timeout = op::Timeout::new(Duration::from_millis(20)).run_on(reactor.clone());
    let mut fut = pin!(&mut timeout);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut, notifier), Poll::Ready(()));
    assert!(fut.is_terminated());
    assert_eq!(reactor.active(), 0);

    assert!(start.elapsed().as_millis() >= 20);
}

#[test]
fn multi_timeout() {
    let (reactor, notifier) = runtime();

    let start = Instant::now();

    let mut timeout1 = op::Timeout::new(Duration::from_millis(20)).run_on(reactor.clone());
    let mut timeout2 = op::Timeout::new(Duration::from_millis(80)).run_on(reactor.clone());
    let mut fut1 = pin!(&mut timeout1);
    let mut fut2 = pin!(&mut timeout2);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut1, notifier), Poll::Ready(()));
    assert!(fut1.is_terminated());
    assert_eq!(reactor.active(), 1);

    assert!(start.elapsed().as_millis() >= 20 && start.elapsed().as_millis() <= 21);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(fut2, notifier), Poll::Ready(()));
    assert!(fut2.is_terminated());
    assert_eq!(reactor.active(), 0);

    assert!(start.elapsed().as_millis() >= 80 && start.elapsed().as_millis() <= 81);
}
