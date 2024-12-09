use std::{os::fd::IntoRawFd, pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::op::{self, Op};

use crate::helpers::{assert_ready, poll, runtime, TempFile, MESSAGE};

#[test]
fn create() {
    let (reactor, notifier) = runtime();
    let filename1 = TempFile::new_name();
    let filename2 = TempFile::new_name();

    let mut creat1 = op::OpenAt::new(&filename1, libc::O_WRONLY | libc::O_CREAT)
        .mode(0)
        .run_on(reactor.clone());
    let mut creat2 = op::OpenAt2::new(&filename2, (libc::O_WRONLY | libc::O_CREAT) as u64)
        .mode(0)
        .resolve(0)
        .run_on(reactor.clone());
    let mut fut1 = pin!(&mut creat1);
    let mut fut2 = pin!(&mut creat2);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.is_ok_and(|fd| fd > 0));

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.is_ok_and(|fd| fd > 0));

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename1).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename1).is_ok());

    assert!(std::fs::exists(&filename2).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename2).is_ok());
}

#[test]
fn relative() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(MESSAGE);
    let file2 = TempFile::with_content(MESSAGE);

    let mut open1 = op::OpenAt::new(file1.relative_path(), libc::O_RDONLY).run_on(reactor.clone());
    let mut open2 =
        op::OpenAt2::new(file2.relative_path(), libc::O_RDONLY as u64).run_on(reactor.clone());

    let mut fut1 = pin!(&mut open1);
    let mut fut2 = pin!(&mut open2);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.as_ref().is_ok_and(|&fd| fd > 0));

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.as_ref().is_ok_and(|&fd| fd > 0));

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn error() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(MESSAGE);
    let path1 = file1.name() + "nopnop";
    let file2 = TempFile::with_content(MESSAGE);
    let path2 = file2.name() + "nopnop";

    let mut open1 = op::OpenAt::new(path1, libc::O_RDONLY).run_on(reactor.clone());
    let mut open2 = op::OpenAt2::new(path2, libc::O_RDONLY as u64).run_on(reactor.clone());
    let mut fut1 = pin!(&mut open1);
    let mut fut2 = pin!(&mut open2);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.is_err());

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.is_err());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
fn cancel() {
    let (reactor, notifier) = runtime();
    let dir1 = TempFile::dir();
    let mut path1 = dir1.relative_path();
    path1.push("cancel");

    let dir2 = TempFile::dir();
    let mut path2 = dir2.relative_path();
    path2.push("cancel");

    let mut creat1 = op::OpenAt::new(&path1, libc::O_CREAT).run_on(reactor.clone());
    let mut creat2 = op::OpenAt2::new(&path2, libc::O_CREAT as u64).run_on(reactor.clone());
    let mut fut1 = pin!(&mut creat1);
    let mut fut2 = pin!(&mut creat2);
    let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(poll!(nop, notifier).is_ready());

    drop(creat1);
    drop(creat2);

    let mut i = 0;
    while !reactor.is_done() {
        reactor.wait();
        i += 1;
        assert!(i < 5);
    }
}

#[test]
fn close() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);
    let fd = std::fs::File::open(file.name()).unwrap().into_raw_fd();

    let mut close = op::Close::new(fd).run_on(reactor.clone());
    let mut error = op::Close::new(1337).run_on(reactor.clone());

    let mut fut = pin!(&mut close);
    let mut bad = pin!(&mut error);

    assert!(poll!(fut, notifier).is_pending());
    assert!(poll!(bad, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(assert_ready!(poll!(fut, notifier)).is_ok());
    assert!(assert_ready!(poll!(bad, notifier)).is_err());

    assert!(fut.is_terminated());
    assert!(bad.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn stats() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut open = op::OpenAt::new(&file.name(), libc::O_RDONLY).run_on(reactor.clone());
    let mut fut = pin!(&mut open);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let fd = assert_ready!(poll!(fut, notifier));
    assert!(fd.as_ref().is_ok_and(|&fd| fd > 0));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat1 = op::Statx::from_fd(fd.unwrap()).run_on(reactor.clone());
    let mut stat2 = op::Statx::new(file.relative_path()).run_on(reactor.clone());
    let mut fut1 = pin!(&mut stat1);
    let mut fut2 = pin!(&mut stat2);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let stats1 = assert_ready!(poll!(fut1, notifier));
    let stats2 = assert_ready!(poll!(fut2, notifier));

    assert!(stats1.is_ok());
    assert!(stats2.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
fn stats_cancel() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut open = op::OpenAt::new(&file.name(), libc::O_RDONLY).run_on(reactor.clone());
    let mut fut = pin!(&mut open);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let fd = assert_ready!(poll!(fut, notifier));
    assert!(fd.as_ref().is_ok_and(|&fd| fd > 0));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat = op::Statx::new(file.path()).run_on(reactor.clone());
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
