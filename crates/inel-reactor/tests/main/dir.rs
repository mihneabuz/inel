use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::op::{self, Op};

use crate::helpers::{assert_ready, poll, runtime, TempFile};

#[test]
fn create() {
    let (reactor, notifier) = runtime();
    let filename = TempFile::new_name();

    let mut mkdir = op::MkDirAt::new(&filename).run_on(reactor.clone());
    let mut fut = pin!(&mut mkdir);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename).is_ok_and(|exists| exists));
    assert!(std::fs::remove_dir(&filename).is_ok());
}

#[test]
fn relative() {
    let (reactor, notifier) = runtime();
    let filename = TempFile::new_relative_name();

    let mut mkdir = op::MkDirAt::new(&filename).run_on(reactor.clone());
    let mut fut = pin!(&mut mkdir);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename).is_ok_and(|exists| exists));
    assert!(std::fs::remove_dir(&filename).is_ok());
}

#[test]
fn error() {
    let (reactor, notifier) = runtime();
    let dir = TempFile::dir();

    let mut mkdir = op::MkDirAt::new(dir.path()).run_on(reactor.clone());
    let mut fut = pin!(&mut mkdir);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_err());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
fn cancel() {
    let (reactor, notifier) = runtime();
    let dir = TempFile::dir();

    let mut mkdir = op::MkDirAt::new(dir.path()).run_on(reactor.clone());
    let mut fut = pin!(&mut mkdir);
    let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(poll!(nop, notifier).is_ready());

    drop(mkdir);

    let mut i = 0;
    while !reactor.is_done() {
        reactor.wait();
        i += 1;
        assert!(i < 3);
    }
}
