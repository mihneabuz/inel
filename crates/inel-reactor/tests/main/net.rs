use crate::helpers::{assert_ready, notifier, poll, runtime, ScopedReactor};
use futures::{future::FusedFuture, StreamExt};
use inel_macro::test_repeat;
use libc::{AF_INET, SOCK_STREAM};
use std::{
    io::Result,
    net::{Shutdown, SocketAddr},
    os::fd::RawFd,
    pin::pin,
    str::FromStr,
    task::Poll,
};

use inel_interface::Reactor;
use inel_reactor::{
    buffer::StableBuffer,
    op::{self, Op},
    util::{bind, getpeername, getsockname, listen},
    FileSlotKey,
};

fn make_addr(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(FromStr::from_str(ip).unwrap(), port)
}

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

    let mut op = op::Socket::new(AF_INET, SOCK_STREAM)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

fn create_fixed_socket_test(domain: i32, typ: i32) -> (ScopedReactor, FileSlotKey) {
    let (reactor, notifier) = runtime();
    let slot = reactor.get_file_slot();

    let mut op = op::Socket::new(domain, typ)
        .proto(0)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    (reactor, slot)
}

fn create_fixed_socket_error_test() {
    let (reactor, notifier) = runtime();
    let slot = reactor.get_file_slot();

    let mut op = op::Socket::new(i32::MAX - 10, i32::MAX / 3)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn create_fixed_socket_cancel_test() {
    let (reactor, notifier) = runtime();
    let slot = reactor.get_file_slot();

    let mut op = op::Socket::new(AF_INET, SOCK_STREAM)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

fn connect_test(addr: &str, port: u16) -> RawFd {
    let addr = make_addr(addr, port);
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

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    sock
}

fn connect_test_ipv4(port: u16) -> RawFd {
    connect_test("127.0.0.1", port)
}

fn connect_test_ipv6(port: u16) -> RawFd {
    connect_test("::1", port)
}

fn connect_error_test(addr: &str) {
    let addr = make_addr(addr, u16::MAX);
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
    let addr = make_addr(addr, u16::MAX);
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

fn accept_test(sock: RawFd) -> (RawFd, SocketAddr) {
    let (reactor, notifier) = runtime();

    let mut con = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_ok());
    assert!(addr.as_ref().unwrap().0 > 0);
    assert!(addr.as_ref().unwrap().1.ip().is_loopback());

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

fn accept_multi_test(sock: RawFd, count: usize) {
    let (reactor, notifier) = runtime();

    let mut con = op::AcceptMulti::new(sock).run_on(reactor.clone());
    let mut stream = pin!(&mut con);

    for _ in 0..count {
        let mut next = pin!(stream.next());

        let fd = match poll!(next, notifier) {
            Poll::Ready(fd) => fd,
            Poll::Pending => {
                reactor.wait();
                assert_eq!(notifier.try_recv(), Some(()));

                assert_ready!(poll!(next, notifier))
            }
        };

        assert!(fd.is_some_and(|fd| fd.is_ok()));
    }

    assert!(!stream.is_terminated());

    std::mem::drop(con);

    reactor.wait();
    reactor.wait();

    assert!(reactor.is_done());
}

fn accept_multi_error_test(sock: RawFd) {
    let (reactor, notifier) = runtime();

    let mut accept = op::AcceptMulti::new(sock).run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let fd = assert_ready!(poll!(fut, notifier));
    assert!(fd.is_err());
}

fn accept_fixed_test(reactor: ScopedReactor, listener: FileSlotKey) -> (FileSlotKey, SocketAddr) {
    let notifier = notifier();
    let slot = reactor.get_file_slot();

    let mut con = op::Accept::new(listener)
        .fixed(slot)
        .run_on(reactor.clone());
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

    (slot, addr.unwrap())
}

fn accept_fixed_error_test(reactor: ScopedReactor, listener: FileSlotKey) {
    let notifier = notifier();
    let slot = reactor.get_file_slot();

    let mut accept = op::Accept::new(listener)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_err());
}

fn accept_fixed_cancel_test(reactor: ScopedReactor, listener: FileSlotKey) {
    let notifier = notifier();
    let slot = reactor.get_file_slot();

    let mut accept = op::Accept::new(listener)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut accept);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(accept);

    reactor.wait();
    assert!(reactor.is_done());
}

fn shutdown_test(sock: RawFd, how: Shutdown) -> Result<()> {
    let (reactor, notifier) = runtime();

    let mut shut = op::Shutdown::new(sock, how).run_on(reactor.clone());
    let mut fut = pin!(shut);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));

    assert!(fut.is_terminated());
    assert!(reactor.is_done());

    res
}

fn create_listener(addr: &str) -> (RawFd, u16) {
    let addr = make_addr(addr, 0);
    let sock = create_socket_test(
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    bind(sock, addr).unwrap();
    listen(sock, 32).unwrap();

    let addr = getsockname(sock);
    let port = addr.unwrap().port();

    (sock, port)
}

fn create_listener_ipv4() -> (RawFd, u16) {
    create_listener("127.0.0.1")
}

fn create_listener_ipv6() -> (RawFd, u16) {
    create_listener("::1")
}

fn create_fixed_listener_ipv4() -> (ScopedReactor, FileSlotKey, u16) {
    let (reactor, _) = runtime();
    let (fd, port) = create_listener_ipv4();
    (reactor.clone(), reactor.register_file(fd), port)
}

fn create_fixed_listener_ipv6() -> (ScopedReactor, FileSlotKey, u16) {
    let (reactor, _) = runtime();
    let (fd, port) = create_listener_ipv6();
    (reactor.clone(), reactor.register_file(fd), port)
}

#[test]
fn socket() {
    for _ in 0..10 {
        create_socket_test(libc::AF_INET, libc::SOCK_STREAM);
        create_socket_test(libc::AF_INET, libc::SOCK_DGRAM);

        create_socket_test(libc::AF_INET6, libc::SOCK_STREAM);
        create_socket_test(libc::AF_INET6, libc::SOCK_DGRAM);

        create_fixed_socket_test(libc::AF_INET, libc::SOCK_STREAM);
        create_fixed_socket_test(libc::AF_INET, libc::SOCK_DGRAM);

        create_fixed_socket_test(libc::AF_INET6, libc::SOCK_STREAM);
        create_fixed_socket_test(libc::AF_INET6, libc::SOCK_DGRAM);
    }
}

#[test]
fn connect() {
    for _ in 0..4 {
        let (_, port) = create_listener_ipv4();
        for _ in 0..10 {
            let sock = connect_test_ipv4(port);
            assert!(getpeername(sock).is_ok_and(|addr| addr.port() == port));
        }
    }

    for _ in 0..4 {
        let (_, port) = create_listener_ipv6();
        for _ in 0..10 {
            let sock = connect_test_ipv6(port);
            assert!(getpeername(sock).is_ok_and(|addr| addr.port() == port));
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

    for _ in 0..4 {
        let (reactor, slot, port) = create_fixed_listener_ipv4();
        for _ in 0..10 {
            connect_test_ipv4(port);
        }
        for _ in 0..10 {
            accept_fixed_test(reactor.clone(), slot);
        }
    }

    for _ in 0..4 {
        let (reactor, slot, port) = create_fixed_listener_ipv6();
        for _ in 0..10 {
            connect_test_ipv6(port);
        }
        for _ in 0..10 {
            accept_fixed_test(reactor.clone(), slot);
        }
    }
}

#[test]
fn accept_multi() {
    for _ in 0..4 {
        let (sock, port) = create_listener_ipv4();
        for _ in 0..10 {
            connect_test_ipv4(port);
        }
        accept_multi_test(sock, 10);
    }

    for _ in 0..4 {
        let (sock, port) = create_listener_ipv6();
        for _ in 0..10 {
            connect_test_ipv6(port);
        }
        accept_multi_test(sock, 10);
    }
}

#[test]
fn shutdown() {
    for how in [Shutdown::Read, Shutdown::Write, Shutdown::Both] {
        let (listener, port) = create_listener_ipv4();
        let client = connect_test_ipv4(port);
        let server = accept_test(listener).0;

        assert!(shutdown_test(client, how).is_ok());
        assert!(shutdown_test(server, how).is_ok());
    }
}

#[test]
fn error() {
    create_socket_error_test();
    create_fixed_socket_error_test();

    connect_error_test_ipv4();
    connect_error_test_ipv6();

    accept_error_test(create_socket_test(libc::AF_INET, libc::SOCK_STREAM));
    accept_error_test(create_socket_test(libc::AF_INET6, libc::SOCK_STREAM));

    let (reactor, slot) = create_fixed_socket_test(libc::AF_INET, libc::SOCK_STREAM);
    accept_fixed_error_test(reactor, slot);
    let (reactor, slot) = create_fixed_socket_test(libc::AF_INET6, libc::SOCK_STREAM);
    accept_fixed_error_test(reactor, slot);

    accept_multi_error_test(create_socket_test(libc::AF_INET, libc::SOCK_STREAM));
    accept_multi_error_test(create_socket_test(libc::AF_INET6, libc::SOCK_STREAM));

    assert!(bind(i32::MAX, make_addr("127.0.0.1", u16::MAX)).is_err());
    assert!(listen(i32::MAX, i32::MAX as u32).is_err());
    assert!(getsockname(i32::MAX).is_err());
    assert!(getpeername(i32::MAX).is_err());

    assert!(shutdown_test(
        create_socket_test(libc::AF_INET, libc::SOCK_STREAM),
        Shutdown::Both
    )
    .is_err());
}

#[test]
#[test_repeat(10)]
fn cancel() {
    create_socket_cancel_test();
    create_fixed_socket_cancel_test();

    connect_cancel_test_ipv4();
    connect_cancel_test_ipv6();

    accept_cancel_test(create_listener_ipv4().0);
    accept_cancel_test(create_listener_ipv6().0);

    let (reactor, slot, _) = create_fixed_listener_ipv4();
    accept_fixed_cancel_test(reactor, slot);

    let (reactor, slot, _) = create_fixed_listener_ipv6();
    accept_fixed_cancel_test(reactor, slot);
}

#[test]
fn read_write() {
    let (listener, port) = create_listener_ipv4();
    let conn1 = connect_test_ipv4(port);
    let (conn2, _) = accept_test(listener);

    let (reactor, notifier) = runtime();

    let write = op::Write::new(conn1, Box::new([b'A'; 4096])).run_on(reactor.clone());
    let read = op::Read::new(conn2, Box::new([0; 8192])).run_on(reactor.clone());

    let mut fut1 = pin!(write);
    let mut fut2 = pin!(read);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert_eq!(reactor.active(), 2);

    reactor.wait();
    reactor.wait();

    let (wrote, res) = assert_ready!(poll!(fut1, notifier));
    assert!(res.is_ok_and(|wrote| wrote == 4096));

    let (read, res) = assert_ready!(poll!(fut2, notifier));
    assert!(res.is_ok_and(|read| read == 4096));

    assert_eq!(wrote.as_slice(), &read.as_slice()[..4096]);

    assert!(reactor.is_done());
}
