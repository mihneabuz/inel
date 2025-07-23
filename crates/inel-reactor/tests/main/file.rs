use std::{os::fd::IntoRawFd, pin::pin};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::op::{self, DetachOp, OpExt};

use crate::helpers::{assert_ready, poll, runtime, TempFile, MESSAGE};

#[test]
fn create() {
    let (reactor, notifier) = runtime();
    let filename1 = TempFile::new_name();
    let filename2 = TempFile::new_name();
    let filename3 = TempFile::new_name();

    let mut creat1 =
        op::OpenAt::new(&filename1, libc::O_WRONLY | libc::O_CREAT).run_on(reactor.clone());
    let mut creat2 =
        op::OpenAt::new(&filename2, libc::O_RDWR | libc::O_CREAT).run_on(reactor.clone());
    let mut creat3 = op::OpenAt::new(&filename3, libc::O_WRONLY | libc::O_CREAT)
        .mode(0)
        .run_on(reactor.clone());

    let mut fut1 = pin!(&mut creat1);
    let mut fut2 = pin!(&mut creat2);
    let mut fut3 = pin!(&mut creat3);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();
    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.is_ok_and(|fd| fd > 0));

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.is_ok_and(|fd| fd > 0));

    let res = assert_ready!(poll!(fut3, notifier));
    assert!(res.is_ok());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(fut3.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&filename1).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename1).is_ok());

    assert!(std::fs::exists(&filename2).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename2).is_ok());

    assert!(std::fs::exists(&filename3).is_ok_and(|exists| exists));
    assert!(std::fs::remove_file(&filename3).is_ok());
}

#[test]
fn relative() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(MESSAGE);
    let file2 = TempFile::with_content(MESSAGE);
    let file3 = TempFile::with_content(MESSAGE);

    let mut open1 = op::OpenAt::new(file1.relative_path(), libc::O_RDONLY).run_on(reactor.clone());
    let mut open2 = op::OpenAt::new(file2.relative_path(), libc::O_RDONLY).run_on(reactor.clone());
    let mut open3 = op::OpenAt::new(file3.relative_path(), libc::O_RDONLY).run_on(reactor.clone());

    let mut fut1 = pin!(&mut open1);
    let mut fut2 = pin!(&mut open2);
    let mut fut3 = pin!(&mut open3);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();
    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.as_ref().is_ok_and(|&fd| fd > 0));

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.as_ref().is_ok_and(|&fd| fd > 0));

    let res = assert_ready!(poll!(fut3, notifier));
    assert!(res.is_ok());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(fut3.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn error() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(MESSAGE);
    let path1 = file1.name() + "nopnop";
    let file2 = TempFile::with_content(MESSAGE);
    let path2 = file2.name() + "nopnop";
    let file3 = TempFile::with_content(MESSAGE);
    let path3 = file3.name() + "nopnop";

    let mut open1 = op::OpenAt::new(path1, libc::O_RDONLY).run_on(reactor.clone());
    let mut open2 = op::OpenAt::new(path2, libc::O_RDONLY).run_on(reactor.clone());
    let mut open3 = op::OpenAt::new(path3, libc::O_RDONLY).run_on(reactor.clone());
    let mut fut1 = pin!(&mut open1);
    let mut fut2 = pin!(&mut open2);
    let mut fut3 = pin!(&mut open3);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();
    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));

    let fd1 = assert_ready!(poll!(fut1, notifier));
    assert!(fd1.is_err());

    let fd2 = assert_ready!(poll!(fut2, notifier));
    assert!(fd2.is_err());

    let res = assert_ready!(poll!(fut3, notifier));
    assert!(res.is_err());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(fut3.is_terminated());
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

    let dir3 = TempFile::dir();
    let mut path3 = dir3.relative_path();
    path3.push("cancel");

    let dir4 = TempFile::dir();
    let mut path4 = dir4.relative_path();
    path4.push("cancel");

    let mut creat1 = op::OpenAt::new(&path1, libc::O_CREAT).run_on(reactor.clone());
    let mut creat2 = op::OpenAt::new(&path2, libc::O_CREAT).run_on(reactor.clone());
    let mut creat3 = op::OpenAt::new(&path3, libc::O_CREAT).run_on(reactor.clone());
    let mut creat4 = op::OpenAt::new(&path3, libc::O_CREAT).run_on(reactor.clone());
    let mut fut1 = pin!(&mut creat1);
    let mut fut2 = pin!(&mut creat2);
    let mut fut3 = pin!(&mut creat3);
    let mut fut4 = pin!(&mut creat4);
    let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert!(poll!(fut4, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());
    assert_eq!(reactor.active(), 5);

    drop(creat1);
    drop(creat2);
    drop(creat3);
    drop(creat4);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(poll!(nop, notifier).is_ready());

    let mut i = 0;
    while !reactor.is_done() {
        reactor.wait();
        i += 1;
        assert!(i < 10);
    }
}

#[test]
fn close() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);

    let fd1 = std::fs::File::open(file1.name()).unwrap().into_raw_fd();
    let fd2 = std::fs::File::open(file2.name()).unwrap().into_raw_fd();

    let mut close1 = op::Close::new(&fd1).run_on(reactor.clone());
    let mut close2 = op::Close::new(&fd2).run_on(reactor.clone());
    let mut error = op::Close::new(&1337).run_on(reactor.clone());

    let mut fut1 = pin!(&mut close1);
    let mut fut2 = pin!(&mut close2);
    let mut bad = pin!(&mut error);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(bad, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();
    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert!(assert_ready!(poll!(fut1, notifier)).is_ok());
    assert!(assert_ready!(poll!(fut2, notifier)).is_ok());
    assert!(assert_ready!(poll!(bad, notifier)).is_err());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(bad.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn close_detach() {
    let (mut reactor, notifier) = runtime();
    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);

    let fd1 = std::fs::File::open(file1.name()).unwrap().into_raw_fd();
    let fd2 = std::fs::File::open(file2.name()).unwrap().into_raw_fd();

    op::Close::new(&fd1).run_detached(&mut reactor);
    op::Close::new(&fd2).run_detached(&mut reactor);
    op::Close::new(&1338).run_detached(&mut reactor);

    assert_eq!(reactor.active(), 0);
    assert!(!reactor.is_done());

    reactor.wait();
    assert_eq!(notifier.try_recv(), None);

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

    let fd = assert_ready!(poll!(fut, notifier)).unwrap();

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat1 = op::Statx::new(fd, libc::STATX_BASIC_STATS).run_on(reactor.clone());
    let mut stat2 = op::Statx::new(fd, libc::STATX_BASIC_STATS).run_on(reactor.clone());
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

    let fd = assert_ready!(poll!(fut, notifier)).unwrap();
    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    assert!(std::fs::exists(&file.name()).is_ok_and(|exists| exists));

    let mut stat = op::Statx::new(fd, libc::STATX_BASIC_STATS).run_on(reactor.clone());
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

mod direct {
    use inel_reactor::source::DirectFd;

    use super::*;

    #[test]
    fn auto() {
        let (reactor, notifier) = runtime();
        let filename1 = TempFile::new_name();
        let filename2 = TempFile::new_name();

        let mut creat1 = op::OpenAt::new(&filename1, libc::O_WRONLY | libc::O_CREAT)
            .auto()
            .run_on(reactor.clone());
        let mut creat2 = op::OpenAt::new(&filename2, libc::O_WRONLY | libc::O_CREAT)
            .auto()
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
        assert!(fd1.is_ok());

        let fd2 = assert_ready!(poll!(fut2, notifier));
        assert!(fd2.is_ok());

        assert!(fut1.is_terminated());
        assert!(fut2.is_terminated());
        assert!(reactor.is_done());

        assert!(std::fs::exists(&filename1).is_ok_and(|exists| exists));
        assert!(std::fs::remove_file(&filename1).is_ok());

        assert!(std::fs::exists(&filename2).is_ok_and(|exists| exists));
        assert!(std::fs::remove_file(&filename2).is_ok());
    }

    #[test]
    fn manual() {
        let (mut reactor, notifier) = runtime();
        let direct1 = DirectFd::get(&mut reactor).unwrap();
        let direct2 = DirectFd::get(&mut reactor).unwrap();

        let filename1 = TempFile::new_name();
        let filename2 = TempFile::new_name();

        {
            let mut creat1 = op::OpenAt::new(&filename1, libc::O_WRONLY | libc::O_CREAT)
                .direct(&direct1)
                .run_on(reactor.clone());
            let mut creat2 = op::OpenAt::new(&filename2, libc::O_WRONLY | libc::O_CREAT)
                .direct(&direct2)
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
            assert!(fd1.is_ok());

            let fd2 = assert_ready!(poll!(fut2, notifier));
            assert!(fd2.is_ok());

            assert!(fut1.is_terminated());
            assert!(fut2.is_terminated());
        }

        direct1.release(&mut reactor);
        direct2.release(&mut reactor);
        assert!(reactor.is_done());

        assert!(std::fs::exists(&filename1).is_ok_and(|exists| exists));
        assert!(std::fs::remove_file(&filename1).is_ok());

        assert!(std::fs::exists(&filename2).is_ok_and(|exists| exists));
        assert!(std::fs::remove_file(&filename2).is_ok());
    }

    #[test]
    fn error() {
        let (mut reactor, _) = runtime();

        let directs = (0..reactor.resources())
            .filter_map(|_| DirectFd::get(&mut reactor).ok())
            .collect::<Vec<_>>();

        assert_eq!(directs.len(), reactor.resources() as usize);
        assert!(DirectFd::get(&mut reactor).is_err());

        directs
            .into_iter()
            .for_each(|direct| direct.release(&mut reactor));

        assert!(DirectFd::get(&mut reactor).is_ok());
    }
}
