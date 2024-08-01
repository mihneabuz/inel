use std::{pin::pin, task::Poll};

use futures::future::FusedFuture;
use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

use crate::helpers::{poll, runtime, TempFile};

const MESSAGE: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
                       labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
                       nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
                       esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
                       in culpa qui officia deserunt mollit anim id est laborum.";

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

    assert!(matches!(res, Ok(_)));
    let Ok(read) = res else {
        panic!("read failed");
    };

    assert_eq!(read, MESSAGE.as_bytes().len());
    assert_eq!(&buf[..read], MESSAGE.as_bytes());

    assert!(fut.is_terminated());
    assert_eq!(reactor.active(), 0);
}
