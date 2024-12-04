use crate::helpers::{assert_ready, poll, runtime};
use futures::future::FusedFuture;
use std::{
    net::SocketAddr,
    os::fd::{IntoRawFd, RawFd},
    pin::pin,
    str::FromStr,
    task::Poll,
};

use inel_interface::Reactor;
use inel_reactor::op::{self, Op};

fn create_socket_test(domain: i32, typ: i32) -> RawFd {
    let (reactor, notifier) = runtime();

    let mut op = op::Socket::new(domain, typ)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    sock.unwrap()
}

fn create_socket_error_test() {
    let (reactor, notifier) = runtime();

    let mut op = op::Socket::new(i32::MAX - 10, i32::MAX / 3)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn create_socket_cancel_test() {
    let (reactor, notifier) = runtime();

    let mut op = op::Socket::new(i32::MAX - 10, i32::MAX / 3)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

fn connect_test(addr: &str, port: u16) -> RawFd {
    let addr = SocketAddr::new(FromStr::from_str(addr).unwrap(), port);
    let sock = create_socket_test(
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let (reactor, notifier) = runtime();

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    sock.unwrap()
}

fn connect_test_ipv4(port: u16) -> RawFd {
    connect_test("127.0.0.1", port)
}

fn connect_test_ipv6(port: u16) -> RawFd {
    connect_test("::1", port)
}

fn connect_error_test(addr: &str) {
    let addr = SocketAddr::new(FromStr::from_str(addr).unwrap(), u16::MAX);
    let sock = create_socket_test(
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let (reactor, notifier) = runtime();

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn connect_error_test_ipv4() {
    connect_error_test("127.0.0.1");
}

fn connect_error_test_ipv6() {
    connect_error_test("::1");
}

fn connect_cancel_test(addr: &str) {
    let addr = SocketAddr::new(FromStr::from_str(addr).unwrap(), u16::MAX);
    let sock = create_socket_test(
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let (reactor, notifier) = runtime();

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(&mut con);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(con);

    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

fn connect_cancel_test_ipv4() {
    connect_cancel_test("127.0.0.1");
}

fn connect_cancel_test_ipv6() {
    connect_cancel_test("::1");
}

fn accept_test(sock: RawFd) -> SocketAddr {
    let (reactor, notifier) = runtime();

    let mut con = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_ok());
    assert!(addr.as_ref().unwrap().ip().is_loopback());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    addr.unwrap()
}

fn accept_error_test(sock: RawFd) {
    let (reactor, notifier) = runtime();

    let mut accept = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_err());
}

fn accept_cancel_test(sock: RawFd) {
    let (reactor, notifier) = runtime();

    let mut accept = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(&mut accept);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(accept);

    reactor.wait();
    assert!(reactor.is_done());
}

fn create_listener(addr: &str) -> (RawFd, u16) {
    let listener = std::net::TcpListener::bind((addr, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let fd = listener.into_raw_fd();

    (fd, port)
}

fn create_listener_ipv4() -> (RawFd, u16) {
    create_listener("127.0.0.1")
}

fn create_listener_ipv6() -> (RawFd, u16) {
    create_listener("::1")
}

#[test]
fn socket() {
    for _ in 0..10 {
        create_socket_test(libc::AF_INET, libc::SOCK_STREAM);
        create_socket_test(libc::AF_INET, libc::SOCK_DGRAM);

        create_socket_test(libc::AF_INET6, libc::SOCK_STREAM);
        create_socket_test(libc::AF_INET6, libc::SOCK_DGRAM);
    }
}

#[test]
fn connect() {
    for _ in 0..4 {
        let (_, port) = create_listener_ipv4();
        for _ in 0..10 {
            connect_test_ipv4(port);
        }
    }

    for _ in 0..4 {
        let (_, port) = create_listener_ipv6();
        for _ in 0..10 {
            connect_test_ipv6(port);
        }
    }
}

#[test]
fn accept() {
    for _ in 0..4 {
        let (sock, port) = create_listener_ipv4();
        for _ in 0..10 {
            connect_test_ipv4(port);
        }
        for _ in 0..10 {
            accept_test(sock);
        }
    }

    for _ in 0..4 {
        let (sock, port) = create_listener_ipv6();
        for _ in 0..10 {
            connect_test_ipv6(port);
        }
        for _ in 0..10 {
            accept_test(sock);
        }
    }
}

#[test]
fn error() {
    create_socket_error_test();

    connect_error_test_ipv4();
    connect_error_test_ipv6();

    accept_error_test(create_socket_test(libc::AF_INET, libc::SOCK_STREAM));
    accept_error_test(create_socket_test(libc::AF_INET6, libc::SOCK_STREAM));
}

#[test]
fn cancel() {
    create_socket_cancel_test();

    connect_cancel_test_ipv4();
    connect_cancel_test_ipv6();

    accept_cancel_test(create_listener_ipv4().0);
    accept_cancel_test(create_listener_ipv6().0);
}
