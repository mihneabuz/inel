use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

use crate::helpers::{poll, runtime, TempFile, MESSAGE};

#[test]
fn single() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut read = op::Read::new(file.fd(), Box::new([0; 1024])).run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready((buf, res)) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    let read = res.expect("read failed");

    assert_eq!(read, 1024);
    assert_eq!(&buf[..read], &MESSAGE.as_bytes()[..read]);

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn multi() {
    let (reactor, notifier) = runtime();

    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);
    let file3 = TempFile::with_content(&MESSAGE);

    const READ_LEN1: usize = 128;
    const READ_LEN2: usize = 256;
    const READ_LEN3: usize = 512;

    let mut read1 = op::Read::new(file1.fd(), Box::new([0; READ_LEN1])).run_on(reactor.clone());
    let mut read2 = op::Read::new(file2.fd(), Box::new([0; READ_LEN2])).run_on(reactor.clone());
    let mut read3 = op::Read::new(file3.fd(), Box::new([0; READ_LEN3])).run_on(reactor.clone());

    let mut fut1 = pin!(&mut read1);
    let mut fut2 = pin!(&mut read2);
    let mut fut3 = pin!(&mut read3);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());

    assert_eq!(reactor.active(), 3);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(reactor.active() < 3);

    reactor.wait();
    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(notifier.try_recv(), Some(()));
    assert_eq!(reactor.active(), 0);

    let Poll::Ready((buf1, res1)) = poll!(fut1, notifier) else {
        panic!("poll 1 not ready");
    };
    let read1 = res1.expect("read 1 failed");

    let Poll::Ready((buf2, res2)) = poll!(fut2, notifier) else {
        panic!("poll 2 not ready");
    };
    let read2 = res2.expect("read 2 failed");

    let Poll::Ready((buf3, res3)) = poll!(fut3, notifier) else {
        panic!("poll 3 not ready");
    };
    let read3 = res3.expect("read 3 failed");

    assert_eq!(read1, READ_LEN1);
    assert_eq!(&buf1[..read1], &MESSAGE.as_bytes()[..read1]);

    assert_eq!(read2, READ_LEN2);
    assert_eq!(&buf2[..read2], &MESSAGE.as_bytes()[..read2]);

    assert_eq!(read3, READ_LEN3);
    assert_eq!(&buf3[..read3], &MESSAGE.as_bytes()[..read3]);

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(fut3.is_terminated());

    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);
    let file3 = TempFile::with_content(&MESSAGE);

    const READ_LEN1: usize = 1024;
    const READ_LEN2: usize = 2048;
    const READ_LEN3: usize = 2048;

    let mut read1 = op::Read::new(file1.fd(), Box::new([0; READ_LEN1])).run_on(reactor.clone());
    let mut read2 = op::Read::new(file2.fd(), Box::new([0; READ_LEN2])).run_on(reactor.clone());
    let mut read3 = op::Read::new(file3.fd(), Box::new([0; READ_LEN3])).run_on(reactor.clone());

    let mut fut1 = pin!(&mut read1);
    let mut fut2 = pin!(&mut read2);
    let mut fut3 = pin!(&mut read3);
    let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());

    assert_eq!(reactor.active(), 4);

    reactor.wait();

    assert!(poll!(nop, notifier).is_ready());

    drop(read1);
    drop(read2);
    drop(read3);

    reactor.wait();
    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

mod vectored {
    use super::*;

    #[test]
    fn single() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut read = op::ReadVectored::new(file.fd(), bufs).run_on(reactor.clone());

        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((bufs, res)) = poll!(fut, notifier) else {
            panic!("poll not ready");
        };
        let read = res.expect("read failed");

        assert_eq!(read, 256 * 6);

        for i in 0..6 {
            assert_eq!(
                bufs[i].as_ref(),
                &MESSAGE.as_bytes()[i * 256..(i + 1) * 256]
            );
        }

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];

        let mut read1 = op::ReadVectored::new(file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut read2 = op::ReadVectored::new(file2.fd(), bufs.clone()).run_on(reactor.clone());

        let mut fut1 = pin!(&mut read1);
        let mut fut2 = pin!(&mut read2);
        let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert!(poll!(nop, notifier).is_pending());

        assert_eq!(reactor.active(), 3);

        reactor.wait();

        assert!(poll!(nop, notifier).is_ready());

        drop(read1);
        drop(read2);

        reactor.wait();
        reactor.wait();

        assert!(reactor.is_done());
    }

    #[test]
    fn exact() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);

        let bufs: [Box<[u8]>; 6] = std::array::from_fn(|_| vec![0u8; 256].into());
        let mut r1 = op::ReadVectoredExact::new(file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut r2 = op::ReadVectoredExact::new(file2.fd(), bufs.clone()).run_on(reactor.clone());

        let mut fut1 = pin!(&mut r1);
        let mut fut2 = pin!(&mut r2);
        let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert!(poll!(nop, notifier).is_pending());

        assert_eq!(reactor.active(), 3);

        reactor.wait();

        drop(r2);

        assert!(poll!(nop, notifier).is_ready());

        reactor.wait();
        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((bufs1, res1)) = poll!(fut1, notifier) else {
            panic!("poll not ready");
        };
        let read1 = res1.expect("read failed");

        assert_eq!(read1, 256 * 6);

        for i in 0..6 {
            assert_eq!(
                bufs1[i].as_ref(),
                &MESSAGE.as_bytes()[i * 256..(i + 1) * 256]
            );
        }

        assert!(fut1.is_terminated());

        assert!(reactor.is_done());
    }
}
