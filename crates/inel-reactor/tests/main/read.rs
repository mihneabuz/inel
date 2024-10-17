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

    let Ok(read) = res else {
        panic!("read failed");
    };

    assert_eq!(read, MESSAGE.as_bytes().len());
    assert_eq!(&buf[..read], MESSAGE.as_bytes());

    assert!(fut.is_terminated());
    assert_eq!(reactor.active(), 0);
}
