mod dir;
mod file;
pub mod helpers;
mod read;
mod timeout;
mod write;

use std::{
    pin::{pin, Pin},
    task::Poll,
};

use futures::future::FusedFuture;
use helpers::{poll, runtime};
use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

#[test]
fn sanity() {
    let (reactor, notifier) = runtime();

    let mut nop = pin!(op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    assert_eq!(poll!(nop, notifier), Poll::Ready(()));
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
        assert_eq!(poll!(nop, notifier), Poll::Ready(()));
        assert!(nop.is_terminated());
    }

    assert!(reactor.is_done());
}
