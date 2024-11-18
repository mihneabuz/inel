use std::{os::fd::RawFd, pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_macro::test_repeat;
use inel_reactor::{
    buffer::{StableBuffer, View},
    op::{self, Op},
};

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
fn offset() {
    let (reactor, notifier) = runtime();
    let mut file = TempFile::with_content("");

    let mut write = op::Write::new(file.fd(), MESSAGE.as_bytes().to_vec())
        .offset(512)
        .run_on(reactor.clone());
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
    assert_eq!(&file.read().as_bytes()[..512], &[0u8; 512]);
    assert_eq!(
        &file.read().as_bytes()[512..],
        MESSAGE.to_string().as_bytes()
    );

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
fn view() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content(&MESSAGE);

    let buf = MESSAGE.as_bytes().to_vec();
    let mut read = op::Write::new(file.fd(), View::new(buf, 64..512)).run_on(reactor.clone());
    let mut fut = pin!(&mut read);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready((view, res)) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    let read = res.expect("write failed");

    assert_eq!(&view.as_slice()[..read], &MESSAGE.as_bytes()[64..64 + read]);

    let buf = view.unview();
    assert_eq!(buf.as_slice(), MESSAGE.as_bytes());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

#[test]
#[test_repeat(100)]
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
fn error() {
    let (reactor, notifier) = runtime();

    let mut write = op::Write::new(RawFd::from(9999), Box::new([0; 128])).run_on(reactor.clone());
    let mut fut = pin!(&mut write);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready((buf, res)) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };

    assert_eq!(buf, Box::new([0; 128]));
    res.expect_err("write didn't fail");
}

#[test]
#[test_repeat(100)]
fn cancel() {
    let (reactor, notifier) = runtime();
    let file1 = TempFile::with_content("");
    let file2 = TempFile::with_content("");
    let file3 = TempFile::with_content("");

    let mut write1 =
        op::Write::new(file1.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());
    let mut write2 = op::Write::new(file2.fd(), MESSAGE.as_bytes().to_vec())
        .offset(256)
        .run_on(reactor.clone());
    let mut write3 = op::Write::new(file3.fd(), View::new(MESSAGE.as_bytes().to_vec(), ..512))
        .run_on(reactor.clone());

    let mut fut1 = pin!(&mut write1);
    let mut fut2 = pin!(&mut write2);
    let mut fut3 = pin!(&mut write3);
    let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert!(poll!(nop, notifier).is_pending());

    assert_eq!(reactor.active(), 4);

    reactor.wait();

    assert!(poll!(nop, notifier).is_ready());

    drop(write1);
    drop(write2);
    drop(write3);

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
        let mut file = TempFile::with_content("");

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut write = op::WriteVectored::new(file.fd(), bufs).run_on(reactor.clone());

        let mut fut = pin!(&mut write);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((bufs, res)) = poll!(fut, notifier) else {
            panic!("poll not ready");
        };
        let write = res.expect("write failed");

        assert_eq!(write, 256 * 6);

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
    fn offset() {
        let (reactor, notifier) = runtime();
        let mut file = TempFile::with_content("");

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];
        let mut read = op::WriteVectored::new(file.fd(), bufs)
            .offset(512)
            .run_on(reactor.clone());

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
                &message.as_bytes()[512 + i * 256..512 + (i + 1) * 256]
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

        let mut write1 =
            op::WriteVectored::new(RawFd::from(9999), orig_bufs.clone()).run_on(reactor.clone());
        let mut write2 = op::WriteVectoredExact::new(RawFd::from(9999), orig_bufs_exact.clone())
            .run_on(reactor.clone());

        let mut fut1 = pin!(&mut write1);
        let mut fut2 = pin!(&mut write2);

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert_eq!(reactor.active(), 2);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));
        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((bufs1, res1)) = poll!(fut1, notifier) else {
            panic!("poll not ready");
        };

        let Poll::Ready((bufs2, res2)) = poll!(fut2, notifier) else {
            panic!("poll not ready");
        };

        assert_eq!(bufs1, orig_bufs);
        assert_eq!(bufs2, orig_bufs_exact);

        res1.expect_err("write didn't fail");
        res2.expect_err("write didn't fail");

        assert!(fut1.is_terminated());
        assert!(fut2.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(100)]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content("");
        let file2 = TempFile::with_content("");

        let bufs: Vec<Box<[u8]>> = vec![vec![0u8; 256].into(); 6];

        let mut write1 = op::WriteVectored::from_iter(file1.fd(), bufs.clone().into_iter())
            .run_on(reactor.clone());
        let mut write2 = op::WriteVectored::from_iter(file2.fd(), bufs.clone().into_iter())
            .run_on(reactor.clone());

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
        let mut file1 = TempFile::with_content("");
        let file2 = TempFile::with_content("");

        let bufs: [Box<[u8]>; 6] = std::array::from_fn(|_| MESSAGE.as_bytes()[0..256].into());
        let mut w1 = op::WriteVectoredExact::new(file1.fd(), bufs.clone()).run_on(reactor.clone());
        let mut w2 = op::WriteVectoredExact::new(file2.fd(), bufs.clone())
            .offset(0)
            .run_on(reactor.clone());

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
        let wrote1 = res1.expect("write failed");

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

#[test]
fn sync() {
    let (reactor, notifier) = runtime();
    let file = TempFile::with_content("");

    let mut write = op::Write::new(file.fd(), MESSAGE.as_bytes().to_vec()).run_on(reactor.clone());
    let mut fut = pin!(&mut write);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(matches!(poll!(fut, notifier), Poll::Ready((_, Ok(_)))));

    let mut sync = op::Fsync::new(file.fd()).run_on(reactor.clone());
    let mut fut = pin!(&mut sync);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert!(matches!(poll!(fut, notifier), Poll::Ready(Ok(()))));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());
}

mod fixed {
    use inel_reactor::buffer::Fixed;

    use super::*;

    #[test]
    fn single() {
        let (reactor, notifier) = runtime();
        let mut file = TempFile::with_content("");

        let buf = Fixed::register(MESSAGE.as_bytes().to_vec(), reactor.clone()).unwrap();
        let mut write = op::WriteFixed::new(file.fd(), buf).run_on(reactor.clone());
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
        assert_eq!(buf.inner().as_slice(), MESSAGE.as_bytes());
        assert_eq!(file.read(), MESSAGE.to_string());

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    fn view() {
        let (reactor, notifier) = runtime();
        let file = TempFile::with_content(&MESSAGE);

        let buf = Fixed::register(MESSAGE.as_bytes().to_vec(), reactor.clone()).unwrap();
        let mut read =
            op::WriteFixed::new(file.fd(), View::new(buf, 64..512)).run_on(reactor.clone());
        let mut fut = pin!(&mut read);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((view, res)) = poll!(fut, notifier) else {
            panic!("poll not ready");
        };
        let read = res.expect("write failed");

        assert_eq!(&view.as_slice()[..read], &MESSAGE.as_bytes()[64..64 + read]);

        let buf = view.unview();
        assert_eq!(buf.into_inner().as_slice(), MESSAGE.as_bytes());

        assert!(fut.is_terminated());
        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(100)]
    fn multi() {
        let (reactor, notifier) = runtime();

        let mut file1 = TempFile::with_content("");
        let mut file2 = TempFile::with_content("");
        let mut file3 = TempFile::with_content("");

        let content = MESSAGE.as_bytes().to_vec();
        let buf1 = Fixed::register(content.clone(), reactor.clone()).unwrap();
        let buf2 = Fixed::register(content.clone(), reactor.clone()).unwrap();
        let buf3 = Fixed::register(content.clone(), reactor.clone()).unwrap();

        let mut write1 = op::WriteFixed::new(file1.fd(), buf1).run_on(reactor.clone());
        let mut write2 = op::WriteFixed::new(file2.fd(), buf2).run_on(reactor.clone());
        let mut write3 = op::WriteFixed::new(file3.fd(), buf3).run_on(reactor.clone());

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
        assert_eq!(&buf1.inner()[..write1], &MESSAGE.as_bytes()[..write1]);
        assert_eq!(file1.read(), MESSAGE.to_string());

        assert_eq!(write2, MESSAGE.as_bytes().len());
        assert_eq!(&buf2.inner()[..write2], &MESSAGE.as_bytes()[..write2]);
        assert_eq!(file2.read(), MESSAGE.to_string());

        assert_eq!(write3, MESSAGE.as_bytes().len());
        assert_eq!(&buf3.inner()[..write3], &MESSAGE.as_bytes()[..write3]);
        assert_eq!(file3.read(), MESSAGE.to_string());

        assert!(fut1.is_terminated());
        assert!(fut2.is_terminated());
        assert!(fut3.is_terminated());

        assert!(reactor.is_done());
    }

    #[test]
    fn error() {
        let (reactor, notifier) = runtime();

        let buf = Fixed::register(MESSAGE.as_bytes().to_vec(), reactor.clone()).unwrap();
        let mut write = op::WriteFixed::new(RawFd::from(9999), buf).run_on(reactor.clone());
        let mut fut = pin!(&mut write);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(reactor.active(), 1);

        reactor.wait();

        assert_eq!(notifier.try_recv(), Some(()));

        let Poll::Ready((buf, res)) = poll!(fut, notifier) else {
            panic!("poll not ready");
        };

        assert_eq!(buf.into_inner().as_slice(), MESSAGE.as_bytes());
        res.expect_err("write didn't fail");
    }

    #[test]
    // #[test_repeat(100)]
    fn cancel() {
        let (reactor, notifier) = runtime();
        let file1 = TempFile::with_content("");
        let file2 = TempFile::with_content("");
        let file3 = TempFile::with_content("");

        let content = MESSAGE.as_bytes().to_vec();
        let buf1 = Fixed::register(content.clone(), reactor.clone()).unwrap();
        let buf2 = Fixed::register(content.clone(), reactor.clone()).unwrap();
        let buf3 = Fixed::register(content.clone(), reactor.clone()).unwrap();

        let mut write1 = op::WriteFixed::new(file1.fd(), buf1).run_on(reactor.clone());
        let mut write2 = op::WriteFixed::new(file2.fd(), buf2)
            .offset(256)
            .run_on(reactor.clone());
        let mut write3 =
            op::WriteFixed::new(file3.fd(), View::new(buf3, ..512)).run_on(reactor.clone());

        let mut fut1 = pin!(&mut write1);
        let mut fut2 = pin!(&mut write2);
        let mut fut3 = pin!(&mut write3);
        let mut nop = pin!(&mut op::Nop.run_on(reactor.clone()));

        assert!(poll!(fut1, notifier).is_pending());
        assert!(poll!(fut2, notifier).is_pending());
        assert!(poll!(fut3, notifier).is_pending());
        assert!(poll!(nop, notifier).is_pending());

        assert_eq!(reactor.active(), 4);

        reactor.wait();

        assert!(poll!(nop, notifier).is_ready());

        drop(write1);
        drop(write2);
        drop(write3);

        let mut i = 0;
        while !reactor.is_done() {
            reactor.wait();
            i += 1;
            assert!(i < 5);
        }
    }
}
