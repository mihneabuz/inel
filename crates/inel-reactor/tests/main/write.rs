use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::op::{self, Op};

use crate::helpers::{poll, runtime, TempFile, MESSAGE};

#[test]
fn single() {
    let (reactor, notifier) = runtime();
    let mut file = TempFile::with_content("");

    let mut write = op::Write::new(file.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());
    let mut fut = pin!(&mut write);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready((buf, res)) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    let wrote = res.expect("write failed");

    assert_eq!(wrote, MESSAGE.as_bytes().len());
    assert_eq!(&buf, MESSAGE.as_bytes());
    assert_eq!(file.read(), MESSAGE.to_string());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(10)]
fn multi() {
    let (reactor, notifier) = runtime();

    let mut file1 = TempFile::with_content("");
    let mut file2 = TempFile::with_content("");
    let mut file3 = TempFile::with_content("");

    let content = MESSAGE.as_bytes().to_vec();

    let mut write1 = op::Write::new(file1.fd(), content.clone()).run_on(reactor.clone());
    let mut write2 = op::Write::new(file2.fd(), content.clone()).run_on(reactor.clone());
    let mut write3 = op::Write::new(file3.fd(), content.clone()).run_on(reactor.clone());

    let mut fut1 = pin!(&mut write1);
    let mut fut2 = pin!(&mut write2);
    let mut fut3 = pin!(&mut write3);

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
    let write1 = res1.expect("write 1 failed");

    let Poll::Ready((buf2, res2)) = poll!(fut2, notifier) else {
        panic!("poll 2 not ready");
    };
    let write2 = res2.expect("write 2 failed");

    let Poll::Ready((buf3, res3)) = poll!(fut3, notifier) else {
        panic!("poll 3 not ready");
    };
    let write3 = res3.expect("write 3 failed");

    assert_eq!(write1, MESSAGE.as_bytes().len());
    assert_eq!(&buf1[..write1], &MESSAGE.as_bytes()[..write1]);
    assert_eq!(file1.read(), MESSAGE.to_string());

    assert_eq!(write2, MESSAGE.as_bytes().len());
    assert_eq!(&buf2[..write2], &MESSAGE.as_bytes()[..write2]);
    assert_eq!(file2.read(), MESSAGE.to_string());

    assert_eq!(write3, MESSAGE.as_bytes().len());
    assert_eq!(&buf3[..write3], &MESSAGE.as_bytes()[..write3]);
    assert_eq!(file3.read(), MESSAGE.to_string());

    assert!(fut1.is_terminated());
    assert!(fut2.is_terminated());
    assert!(fut3.is_terminated());

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(10)]
fn cancel() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content("");
    let file2 = TempFile::with_content("");
    let file3 = TempFile::with_content("");

    let mut read1 = op::Write::new(file1.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());
    let mut read2 = op::Write::new(file2.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());
    let mut read3 = op::Write::new(file3.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());

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
        let mut file = TempFile::with_content("");

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut read = op::WriteVectored::new(file.fd(), bufs).run_on(reactor.clone());

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

        let message = file.read();
        for i in 0..6 {
            assert_eq!(
                bufs[i].as_ref(),
                &message.as_bytes()[i * 256..(i + 1) * 256]
            );
        }

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(10)]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content("");
        let file2 = TempFile::with_content("");

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];

        let mut write1 = op::WriteVectored::new(file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut write2 = op::WriteVectored::new(file2.fd(), bufs.clone()).run_on(reactor.clone());

        let mut fut1 = pin!(&mut write1);
        let mut fut2 = pin!(&mut write2);
        let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert!(poll!(nop, notifier).is_pending());

        assert_eq!(reactor.active(), 3);

        reactor.wait();

        assert!(poll!(nop, notifier).is_ready());

        drop(write1);
        drop(write2);

        reactor.wait();
        reactor.wait();

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(10)]
    fn exact() {
        let (reactor, notifier) = runtime();
        let mut file1 = TempFile::with_content("");
        let file2 = TempFile::with_content("");

        let bufs: [Box<[u8]>; 6] = std::array::from_fn(|_| MESSAGE.as_bytes()[0..256].into());
        let mut w1 = op::WriteVectoredExact::new(file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut w2 = op::WriteVectoredExact::new(file2.fd(), bufs.clone()).run_on(reactor.clone());

        let mut fut1 = pin!(&mut w1);
        let mut fut2 = pin!(&mut w2);
        let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert!(poll!(nop, notifier).is_pending());

        assert_eq!(reactor.active(), 3);

        reactor.wait();

        drop(w2);

        assert!(poll!(nop, notifier).is_ready());

        reactor.wait();
        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((bufs1, res1)) = poll!(fut1, notifier) else {
            panic!("poll not ready");
        };
        let wrote1 = res1.expect("read failed");

        assert_eq!(wrote1, 256 * 6);

        let message = file1.read();
        for i in 0..6 {
            assert_eq!(
                bufs1[i].as_ref(),
                &message.as_bytes()[i * 256..(i + 1) * 256]
            );
        }

        assert!(fut1.is_terminated());

        assert!(reactor.is_done());
    }
}
