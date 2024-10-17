use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
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

    let Ok(wrote) = res else {
        panic!("write failed");
    };

    assert_eq!(wrote, MESSAGE.as_bytes().len());
    assert_eq!(&buf, MESSAGE.as_bytes());
    assert_eq!(file.read(), MESSAGE.to_string());

    assert!(fut.is_terminated());
    assert_eq!(reactor.active(), 0);
}
