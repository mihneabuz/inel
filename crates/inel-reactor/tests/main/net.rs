use crate::helpers::{poll, runtime};
use futures::future::FusedFuture;
use std::{os::fd::RawFd, pin::pin, task::Poll};

use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

fn socket_test(domain: i32, typ: i32) -> RawFd {
    let (reactor, notifier) = runtime();

    let mut op = op::Socket::new(domain, typ).run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let Poll::Ready(sock) = poll!(fut, notifier) else {
        panic!("poll not ready");
    };
    assert!(sock.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    sock.unwrap()
}

#[test]
fn socket() {
    for _ in 0..100 {
        socket_test(libc::AF_INET, libc::SOCK_STREAM);
        socket_test(libc::AF_INET, libc::SOCK_DGRAM);

        socket_test(libc::AF_INET6, libc::SOCK_STREAM);
        socket_test(libc::AF_INET6, libc::SOCK_DGRAM);
    }
}
