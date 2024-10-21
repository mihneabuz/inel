use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::op::{self, Op};

use crate::helpers::{poll, runtime, TempFile, MESSAGE};

#[test]
fn create() {
    let (reactor, notifier) = runtime();
    let filename = TempFile::new_name();

    let mut creat = op::OpenAt::new(&filename, libc::O_WRONLY | libc::O_CREAT)
        .unwrap()
        .run_on(reactor.clone());
    let mut fut = pin!(&mut creat);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(fd) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    assert!(fd.is_ok_and(|fd| fd > 0));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename).is_ok());
}

#[test]
fn stats() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut open = op::OpenAt::new(&file.name(), libc::O_RDONLY)
        .unwrap()
        .run_on(reactor.clone());
    let mut fut = pin!(&mut open);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(fd) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    assert!(fd.as_ref().is_ok_and(|&fd| fd > 0));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat = op::Statx::new(fd.unwrap()).run_on(reactor.clone());
    let mut fut = pin!(&mut stat);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(stats) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };

    assert!(stats.is_ok());
    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
fn stats_cancel() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut open = op::OpenAt::new(&file.name(), libc::O_RDONLY)
        .unwrap()
        .run_on(reactor.clone());
    let mut fut = pin!(&mut open);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(fd) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    assert!(fd.as_ref().is_ok_and(|&fd| fd > 0));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat = op::Statx::new(fd.unwrap()).run_on(reactor.clone());
    let mut fut = pin!(&mut stat);
    let mut nop = pin!(op::Nop.run_on(reactor.clone()));

    assert!(poll!(nop, notifier).is_pending());
    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();

    assert!(poll!(nop, notifier).is_ready());
    drop(stat);

    reactor.wait();

    let mut i = 0;
    while !reactor.is_done() {
        reactor.wait();
        i += 1;
        assert!(i < 3);
    }
}

#[test]
fn create_dir() {
    let (reactor, notifier) = runtime();
    let filename = TempFile::new_name();

    let mut mkdir = op::MkDirAt::new(&filename).unwrap().run_on(reactor.clone());
    let mut fut = pin!(&mut mkdir);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(res) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    assert!(res.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename).is_ok_and(|exists| exists));
    assert!(std::fs::remove_dir(&filename).is_ok());
}
