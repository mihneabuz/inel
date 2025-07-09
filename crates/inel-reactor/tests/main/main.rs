mod buffer;
mod chain;
mod dir;
mod file;
mod group;
pub mod helpers;
mod net;
mod read;
mod timeout;
mod write;

use std::pin::{pin, Pin};

use futures::{future::FusedFuture, StreamExt};
use helpers::{assert_ready, poll, runtime};
use inel_interface::Reactor;
use inel_reactor::op::{self, DetachOp, OpExt};

#[test]
fn sanity() {
    let (reactor, notifier) = runtime();

    let mut nop = pin!(op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    assert_ready!(poll!(nop, notifier));
    assert!(nop.is_terminated());

    assert!(reactor.is_done());
}

#[test]
fn nops() {
    let (reactor, notifier) = runtime();

    let mut nops: [_; 10] = std::array::from_fn(|_| op::Nop.run_on(reactor.clone()));
    let mut futs = nops.iter_mut().map(|nop| Pin::new(nop)).collect::<Vec<_>>();

    for nop in futs.iter_mut() {
        assert!(poll!(nop, notifier).is_pending());
    }
    assert_eq!(reactor.active(), 10);

    for nop in futs.iter_mut() {
        assert!(poll!(nop, notifier).is_pending());
    }
    assert_eq!(reactor.active(), 10);

    reactor.wait();

    for nop in futs.iter_mut() {
        assert_ready!(poll!(nop, notifier));
        assert!(nop.is_terminated());
    }

    assert!(reactor.is_done());
}

#[test]
fn polling() {
    let (reactor, notifier) = runtime();

    let mut nop = pin!(op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_ready!(poll!(nop, notifier));
    assert!(nop.is_terminated());

    assert!(reactor.is_done());
}

#[test]
fn stream() {
    let (reactor, notifier) = runtime();

    let mut nop = op::Nop.run_on(reactor.clone());
    let fut = pin!(&mut nop);

    let mut stream = pin!(fut.collect::<Vec<_>>());

    assert!(poll!(stream, notifier).is_pending());

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(assert_ready!(poll!(stream, notifier)), vec![()]);
    assert!(nop.is_terminated());

    assert!(reactor.is_done());
}

#[test]
fn detach() {
    let (mut reactor, notifier) = runtime();

    op::Nop.run_detached(&mut reactor);

    reactor.wait();
    assert_eq!(notifier.try_recv(), None);

    assert!(reactor.is_done());
}

#[test]
#[should_panic]
fn panic() {
    let (reactor, notifier) = runtime();

    let mut nop = pin!(op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());

    reactor.wait();

    assert_ready!(poll!(nop, notifier));
    assert!(nop.is_terminated());

    let _ = poll!(nop, notifier);
}
