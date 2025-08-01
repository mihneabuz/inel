use std::{os::fd::RawFd, pin::pin};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::{
    buffer::{StableBuffer, View},
    op::{self, OpExt},
};

use crate::helpers::{assert_ready, poll, runtime, TempFile, MESSAGE};

#[test]
fn single() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut read = op::Read::new(&file.fd(), Box::new([0; 1024])).run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let (buf, res) = assert_ready!(poll!(fut, notifier));
    let read = res.expect("read failed");

    assert_eq!(read, 1024);
    assert_eq!(&buf[..read], &MESSAGE.as_bytes()[..read]);

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn offset() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let mut read = op::Read::new(&file.fd(), Box::new([0; 1024]))
        .offset(512)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let (buf, res) = assert_ready!(poll!(fut, notifier));
    let read = res.expect("read failed");

    assert_eq!(read, 1024);
    assert_eq!(&buf[..read], &MESSAGE.as_bytes()[512..512 + read]);

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn view() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let buf = Box::new([0; 1024]);
    let mut read = op::Read::new(&file.fd(), View::new(buf, 64..512)).run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let (view, res) = assert_ready!(poll!(fut, notifier));
    let read = res.expect("read failed");

    assert_eq!(&view.stable_slice()[..read], &MESSAGE.as_bytes()[..read]);

    let buf = view.unview();
    assert_eq!(&buf[0..64], &[0; 64]);
    assert_eq!(&buf[512..], &[0; 512]);

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
fn multi() {
    let (reactor, notifier) = runtime();

    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);
    let file3 = TempFile::with_content(&MESSAGE);

    const READ_LEN1: usize = 128;
    const READ_LEN2: usize = 256;
    const READ_LEN3: usize = 512;

    let mut read1 = op::Read::new(&file1.fd(), Box::new([0; READ_LEN1])).run_on(reactor.clone());
    let mut read2 = op::Read::new(&file2.fd(), Box::new([0; READ_LEN2])).run_on(reactor.clone());
    let mut read3 = op::Read::new(&file3.fd(), Vec::from([0; READ_LEN3])).run_on(reactor.clone());

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

    let (buf1, res1) = assert_ready!(poll!(fut1, notifier));
    let read1 = res1.expect("read 1 failed");

    let (buf2, res2) = assert_ready!(poll!(fut2, notifier));
    let read2 = res2.expect("read 2 failed");

    let (buf3, res3) = assert_ready!(poll!(fut3, notifier));
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
fn error() {
    let (reactor, notifier) = runtime();

    let mut read = op::Read::new(&RawFd::from(9999), Box::new([0; 128])).run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let (buf, res) = assert_ready!(poll!(fut, notifier));
    assert_eq!(buf, Box::new([0; 128]));
    res.expect_err("read didn't fail");
}

#[test]
#[test_repeat(100)]
fn cancel() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content(&MESSAGE);
    let file2 = TempFile::with_content(&MESSAGE);
    let file3 = TempFile::with_content(&MESSAGE);

    const READ_LEN1: usize = 1024;
    const READ_LEN2: usize = 2048;
    const READ_LEN3: usize = 2048;

    let mut read1 = op::Read::new(&file1.fd(), Box::new([0; READ_LEN1])).run_on(reactor.clone());
    let mut read2 = op::Read::new(&file2.fd(), Box::new([0; READ_LEN2]))
        .offset(128)
        .run_on(reactor.clone());
    let mut read3 = op::Read::new(&file3.fd(), View::new(Box::new([0; READ_LEN3]), 128..))
        .run_on(reactor.clone());

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

    let mut i = 0;
    while !reactor.is_done() {
        reactor.wait();
        i += 1;
        assert!(i < 5);
    }
}

mod vectored {
    use super::*;

    #[test]
    fn single() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut read = op::ReadVectored::new(&file.fd(), bufs).run_on(reactor.clone());

        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let (bufs, res) = assert_ready!(poll!(fut, notifier));
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
    fn offset() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut read = op::ReadVectored::new(&file.fd(), bufs)
            .offset(512)
            .run_on(reactor.clone());

        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let (bufs, res) = assert_ready!(poll!(fut, notifier));
        let read = res.expect("read failed");

        assert_eq!(read, 256 * 6);

        for i in 0..6 {
            assert_eq!(
                bufs[i].as_ref(),
                &MESSAGE.as_bytes()[512 + i * 256..512 + (i + 1) * 256]
            );
        }

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    fn error() {
        let (reactor, notifier) = runtime();

        let orig_bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let orig_bufs_exact: [Box<[u8]>; 6] = std::array::from_fn(|_| vec![0u8; 256].into());

        let mut read1 =
            op::ReadVectored::new(&RawFd::from(9999), orig_bufs.clone()).run_on(reactor.clone());
        let mut read2 = op::ReadVectoredExact::new(&RawFd::from(9999), orig_bufs_exact.clone())
            .run_on(reactor.clone());

        let mut fut1 = pin!(&mut read1);
        let mut fut2 = pin!(&mut read2);

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert_eq!(reactor.active(), 2);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));
        assert_eq!(notifier.try_recv(), Some(()));

        let (bufs1, res1) = assert_ready!(poll!(fut1, notifier));
        let (bufs2, res2) = assert_ready!(poll!(fut2, notifier));

        assert_eq!(bufs1, orig_bufs);
        assert_eq!(bufs2, orig_bufs_exact);

        res1.expect_err("read didn't fail");
        res2.expect_err("read didn't fail");

        assert!(fut1.is_terminated());
        assert!(fut2.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(100)]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];

        let mut read1 = op::ReadVectored::from_iter(&file1.fd(), bufs.clone().into_iter())
            .run_on(reactor.clone());
        let mut read2 = op::ReadVectored::from_iter(&file2.fd(), bufs.clone().into_iter())
            .run_on(reactor.clone());

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

        let mut i = 0;
        while !reactor.is_done() {
            reactor.wait();
            i += 1;
            assert!(i < 5);
        }
    }

    #[test]
    #[test_repeat(100)]
    fn exact() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);

        let bufs: [Box<[u8]>; 6] = std::array::from_fn(|_| vec![0u8; 256].into());
        let mut r1 = op::ReadVectoredExact::new(&file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut r2 = op::ReadVectoredExact::new(&file2.fd(), bufs.clone())
            .offset(0)
            .run_on(reactor.clone());

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

        let (bufs1, res1) = assert_ready!(poll!(fut1, notifier));
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

mod fixed {
    use inel_reactor::buffer::Fixed;

    use super::*;

    #[test]
    fn single() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);
        let slot = reactor.register_file(file.fd());

        let buf = Fixed::register(Box::new([0; 1024]), reactor.clone()).unwrap();
        let mut read = op::ReadFixed::new(&slot, buf).run_on(reactor.clone());
        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let (buf, res) = assert_ready!(poll!(fut, notifier));
        let read = res.expect("read failed");

        assert_eq!(read, 1024);
        assert_eq!(&buf.stable_slice()[..read], &MESSAGE.as_bytes()[..read]);

        std::mem::drop(buf);

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    fn view() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);
        let slot = reactor.register_file(file.fd());

        let buf = Fixed::register(Box::new([0; 1024]), reactor.clone()).unwrap();
        let mut read = op::ReadFixed::new(&slot, View::new(buf, 64..512)).run_on(reactor.clone());
        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let (view, res) = assert_ready!(poll!(fut, notifier));
        let read = res.expect("read failed");

        assert_eq!(&view.stable_slice()[..read], &MESSAGE.as_bytes()[..read]);

        let buf = view.unview();
        assert_eq!(&buf.stable_slice()[0..64], &[0; 64]);
        assert_eq!(&buf.stable_slice()[512..], &[0; 512]);

        std::mem::drop(buf);

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(100)]
    fn multi() {
        let (reactor, notifier) = runtime();

        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);
        let file3 = TempFile::with_content(&MESSAGE);

        const READ_LEN1: usize = 128;
        const READ_LEN2: usize = 256;
        const READ_LEN3: usize = 512;

        let buf1 = Fixed::new(READ_LEN1, reactor.clone()).unwrap();
        let buf2 = Fixed::new(READ_LEN2, reactor.clone()).unwrap();
        let buf3 = Fixed::new(READ_LEN3, reactor.clone()).unwrap();

        let mut read1 = op::ReadFixed::new(&file1.fd(), buf1).run_on(reactor.clone());
        let mut read2 = op::ReadFixed::new(&file2.fd(), buf2).run_on(reactor.clone());
        let mut read3 = op::ReadFixed::new(&file3.fd(), buf3).run_on(reactor.clone());

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

        let (buf1, res1) = assert_ready!(poll!(fut1, notifier));
        let read1 = res1.expect("read 1 failed");

        let (buf2, res2) = assert_ready!(poll!(fut2, notifier));
        let read2 = res2.expect("read 2 failed");

        let (buf3, res3) = assert_ready!(poll!(fut3, notifier));
        let read3 = res3.expect("read 3 failed");

        assert_eq!(read1, READ_LEN1);
        assert_eq!(&buf1.stable_slice()[..read1], &MESSAGE.as_bytes()[..read1]);

        assert_eq!(read2, READ_LEN2);
        assert_eq!(&buf2.stable_slice()[..read2], &MESSAGE.as_bytes()[..read2]);

        assert_eq!(read3, READ_LEN3);
        assert_eq!(&buf3.stable_slice()[..read3], &MESSAGE.as_bytes()[..read3]);

        assert!(fut1.is_terminated());
        assert!(fut2.is_terminated());
        assert!(fut3.is_terminated());

        std::mem::drop(buf1);
        std::mem::drop(buf2);
        std::mem::drop(buf3);

        assert!(reactor.is_done());
    }

    #[test]
    fn error() {
        let (reactor, notifier) = runtime();

        let buf = Fixed::register(Box::new([0; 128]), reactor.clone()).unwrap();
        let mut read = op::ReadFixed::new(&RawFd::from(9999), buf).run_on(reactor.clone());
        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let (buf, res) = assert_ready!(poll!(fut, notifier));
        assert_eq!(&buf.stable_slice()[..], &[0; 128]);
        res.expect_err("read didn't fail");

        std::mem::drop(buf);

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(100)]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content(&MESSAGE);
        let file2 = TempFile::with_content(&MESSAGE);
        let file3 = TempFile::with_content(&MESSAGE);

        const READ_LEN1: usize = 1024;
        const READ_LEN2: usize = 2048;
        const READ_LEN3: usize = 2048;

        let buf1 = Fixed::new(READ_LEN1, reactor.clone()).unwrap();
        let buf2 = Fixed::new(READ_LEN2, reactor.clone()).unwrap();
        let buf3 = Fixed::new(READ_LEN3, reactor.clone()).unwrap();

        let mut read1 = op::ReadFixed::new(&file1.fd(), buf1).run_on(reactor.clone());
        let mut read2 = op::ReadFixed::new(&file2.fd(), buf2)
            .offset(128)
            .run_on(reactor.clone());
        let mut read3 =
            op::ReadFixed::new(&file3.fd(), View::new(buf3, 128..)).run_on(reactor.clone());

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

        let mut i = 0;
        while !reactor.is_done() {
            reactor.wait();
            i += 1;
            assert!(i < 5);
        }
    }
}
