use crate::helpers::{assert_ready, notifier, poll, reactor, runtime, ScopedReactor};
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

fn create_socket_test(reactor: ScopedReactor, domain: i32, typ: i32) -> RawFd {
    let notifier = notifier();

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

    sock.unwrap()
}

fn create_socket_error_test(reactor: ScopedReactor) {
    let notifier = notifier();

    let mut op = op::Socket::new(i32::MAX - 10, i32::MAX / 3)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn create_socket_cancel_test(reactor: ScopedReactor) {
    let notifier = notifier();

    let mut op = op::Socket::new(AF_INET, SOCK_STREAM)
        .proto(0)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();
}

fn create_fixed_socket_test(reactor: ScopedReactor, domain: i32, typ: i32) -> FileSlotKey {
    let notifier = notifier();
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

    slot
}

fn create_fixed_socket_error_test(reactor: ScopedReactor) {
    let notifier = notifier();
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

fn create_fixed_socket_cancel_test(reactor: ScopedReactor) {
    let notifier = notifier();
    let slot = reactor.get_file_slot();

    let mut op = op::Socket::new(AF_INET, SOCK_STREAM)
        .fixed(slot)
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();

    reactor.release_file_slot(slot);
}

fn create_auto_socket_test(reactor: ScopedReactor, domain: i32, typ: i32) -> FileSlotKey {
    let notifier = notifier();

    let mut op = op::Socket::new(domain, typ)
        .proto(0)
        .direct()
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());
    let slot = res.unwrap();

    assert!(fut.is_terminated());

    slot
}

fn create_auto_socket_error_test(reactor: ScopedReactor) {
    let notifier = notifier();

    let mut op = op::Socket::new(i32::MAX - 10, i32::MAX / 3)
        .direct()
        .run_on(reactor.clone());
    let mut fut = pin!(op);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn create_auto_socket_cancel_test(reactor: ScopedReactor) {
    let notifier = notifier();

    let mut op = op::Socket::new(AF_INET, SOCK_STREAM)
        .direct()
        .run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    reactor.wait();
}

fn connect_test(reactor: ScopedReactor, addr: &str, port: u16) -> RawFd {
    let notifier = notifier();
    let addr = make_addr(addr, port);
    let sock = create_socket_test(
        reactor.clone(),
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());
    assert!(fut.is_terminated());

    sock
}

fn connect_fixed_test(reactor: ScopedReactor, addr: &str, port: u16) -> FileSlotKey {
    let notifier = notifier();
    let addr = make_addr(addr, port);
    let slot = create_fixed_socket_test(
        reactor.clone(),
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let mut con = op::Connect::new(slot, addr).run_on(reactor.clone());
    let mut fut = pin!(&mut con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());

    assert!(fut.is_terminated());

    slot
}

fn connect_test_ipv4(reactor: ScopedReactor, port: u16) -> RawFd {
    connect_test(reactor, "127.0.0.1", port)
}

fn connect_test_ipv6(reactor: ScopedReactor, port: u16) -> RawFd {
    connect_test(reactor, "::1", port)
}

fn connect_fixed_test_ipv4(reactor: ScopedReactor, port: u16) -> FileSlotKey {
    connect_fixed_test(reactor, "127.0.0.1", port)
}

fn connect_fixed_test_ipv6(reactor: ScopedReactor, port: u16) -> FileSlotKey {
    connect_fixed_test(reactor, "::1", port)
}

fn connect_error_test(reactor: ScopedReactor, addr: &str) {
    let notifier = notifier();
    let addr = make_addr(addr, u16::MAX);
    let sock = create_socket_test(
        reactor.clone(),
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let sock = assert_ready!(poll!(fut, notifier));
    assert!(sock.is_err());
}

fn connect_error_test_ipv4(reactor: ScopedReactor) {
    connect_error_test(reactor, "127.0.0.1");
}

fn connect_error_test_ipv6(reactor: ScopedReactor) {
    connect_error_test(reactor, "::1");
}

fn connect_cancel_test(reactor: ScopedReactor, addr: &str) {
    let notifier = notifier();
    let addr = make_addr(addr, u16::MAX);
    let sock = create_socket_test(
        reactor.clone(),
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let mut con = op::Connect::new(sock, addr).run_on(reactor.clone());
    let mut fut = pin!(&mut con);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(con);

    reactor.wait();
    reactor.wait();
}

fn connect_cancel_test_ipv4(reactor: ScopedReactor) {
    connect_cancel_test(reactor, "127.0.0.1");
}

fn connect_cancel_test_ipv6(reactor: ScopedReactor) {
    connect_cancel_test(reactor, "::1");
}

fn accept_test(reactor: ScopedReactor, sock: RawFd) -> (RawFd, SocketAddr) {
    let notifier = notifier();

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

    addr.unwrap()
}

fn accept_error_test(reactor: ScopedReactor, sock: RawFd) {
    let notifier = notifier();

    let mut accept = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_err());
}

fn accept_cancel_test(reactor: ScopedReactor, sock: RawFd) {
    let notifier = notifier();

    let mut accept = op::Accept::new(sock).run_on(reactor.clone());
    let mut fut = pin!(&mut accept);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(accept);

    reactor.wait();
}

fn accept_multi_test(reactor: ScopedReactor, sock: RawFd, count: usize) {
    let notifier = notifier();

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
}

fn accept_multi_direct_test(reactor: ScopedReactor, slot: FileSlotKey, count: usize) {
    let notifier = notifier();

    let mut con = op::AcceptMulti::new(slot).direct().run_on(reactor.clone());
    let mut stream = pin!(&mut con);

    for _ in 0..count {
        let mut next = pin!(stream.next());

        let slot = match poll!(next, notifier) {
            Poll::Ready(fd) => fd,
            Poll::Pending => {
                reactor.wait();
                assert_eq!(notifier.try_recv(), Some(()));
                assert_ready!(poll!(next, notifier))
            }
        };

        assert!(slot.is_some_and(|slot| slot.is_ok_and(|slot| slot.index() > 0)))
    }

    assert!(!stream.is_terminated());
    std::mem::drop(con);

    reactor.wait();
}

fn accept_multi_once_test(reactor: ScopedReactor, sock: RawFd) {
    let notifier = notifier();

    let mut con = op::AcceptMulti::new(sock).run_on(reactor.clone());
    let mut fut = pin!(&mut con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert!(poll!(fut, notifier).is_ready());
    assert!(!fut.is_terminated());

    std::mem::drop(con);
    reactor.wait();
}

fn accept_multi_direct_once_test(reactor: ScopedReactor, slot: FileSlotKey) {
    let notifier = notifier();

    let mut con = op::AcceptMulti::new(slot).direct().run_on(reactor.clone());
    let mut fut = pin!(&mut con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert!(poll!(fut, notifier).is_ready());
    assert!(!fut.is_terminated());

    std::mem::drop(con);
    reactor.wait();
}

fn accept_multi_error_test(reactor: ScopedReactor, sock: RawFd) {
    let notifier = notifier();

    let mut accept = op::AcceptMulti::new(sock).run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());
    reactor.wait();

    let fd = assert_ready!(poll!(fut, notifier));
    assert!(fd.is_err());
}

fn accept_multi_direct_error_test(reactor: ScopedReactor, slot: FileSlotKey) {
    let notifier = notifier();

    let mut accept = op::AcceptMulti::new(slot).run_on(reactor.clone());
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

    (slot, addr.unwrap())
}

fn accept_auto_test(reactor: ScopedReactor, listener: FileSlotKey) -> (FileSlotKey, SocketAddr) {
    let notifier = notifier();

    let mut con = op::Accept::new(listener).direct().run_on(reactor.clone());
    let mut fut = pin!(con);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(res.is_ok());
    assert!(res.as_ref().unwrap().1.ip().is_loopback());

    assert!(fut.is_terminated());

    res.unwrap()
}

fn accept_auto_error_test(reactor: ScopedReactor, listener: FileSlotKey) {
    let notifier = notifier();
    let mut accept = op::Accept::new(listener).direct().run_on(reactor.clone());
    let mut fut = pin!(accept);

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    let addr = assert_ready!(poll!(fut, notifier));
    assert!(addr.is_err());
}

fn accept_auto_cancel_test(reactor: ScopedReactor, listener: FileSlotKey) {
    let notifier = notifier();
    let mut accept = op::Accept::new(listener).direct().run_on(reactor.clone());
    let mut fut = pin!(&mut accept);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(accept);

    reactor.wait();
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
}

fn shutdown_test(reactor: ScopedReactor, sock: RawFd, how: Shutdown) -> Result<()> {
    let notifier = notifier();

    let mut shut = op::Shutdown::new(sock, how).run_on(reactor.clone());
    let mut fut = pin!(shut);

    assert!(poll!(fut, notifier).is_pending());
    assert_eq!(reactor.active(), 1);

    reactor.wait();
    assert_eq!(notifier.try_recv(), Some(()));

    let res = assert_ready!(poll!(fut, notifier));
    assert!(fut.is_terminated());
    res
}

fn create_listener(reactor: ScopedReactor, addr: &str) -> (RawFd, u16) {
    let addr = make_addr(addr, 0);
    let sock = create_socket_test(
        reactor.clone(),
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

fn create_listener_ipv4(reactor: ScopedReactor) -> (RawFd, u16) {
    create_listener(reactor.clone(), "127.0.0.1")
}

fn create_listener_ipv6(reactor: ScopedReactor) -> (RawFd, u16) {
    create_listener(reactor.clone(), "::1")
}

fn create_fixed_listener_ipv4(reactor: ScopedReactor) -> (FileSlotKey, u16) {
    let (fd, port) = create_listener_ipv4(reactor.clone());
    (reactor.register_file(fd), port)
}

fn create_fixed_listener_ipv6(reactor: ScopedReactor) -> (FileSlotKey, u16) {
    let (fd, port) = create_listener_ipv6(reactor.clone());
    (reactor.register_file(fd), port)
}

#[test]
#[test_repeat(5)]
fn socket() {
    let reactor = reactor();

    create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
    create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

    create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
    create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(5)]
fn connect() {
    let reactor = reactor();

    let (_, port) = create_listener_ipv4(reactor.clone());
    for _ in 0..10 {
        let sock = connect_test_ipv4(reactor.clone(), port);
        assert!(getpeername(sock).is_ok_and(|addr| addr.port() == port));
    }

    let (_, port) = create_listener_ipv6(reactor.clone());
    for _ in 0..10 {
        let sock = connect_test_ipv6(reactor.clone(), port);
        assert!(getpeername(sock).is_ok_and(|addr| addr.port() == port));
    }

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(5)]
fn accept() {
    let reactor = reactor();

    let (sock, port) = create_listener_ipv4(reactor.clone());
    for _ in 0..10 {
        connect_test_ipv4(reactor.clone(), port);
    }
    for _ in 0..10 {
        accept_test(reactor.clone(), sock);
    }

    let (sock, port) = create_listener_ipv6(reactor.clone());
    for _ in 0..10 {
        connect_test_ipv6(reactor.clone(), port);
    }
    for _ in 0..10 {
        accept_test(reactor.clone(), sock);
    }

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(5)]
fn accept_multi() {
    let reactor = reactor();

    let (sock, port) = create_listener_ipv4(reactor.clone());
    for _ in 0..10 {
        connect_test_ipv4(reactor.clone(), port);
    }
    accept_multi_test(reactor.clone(), sock, 10);

    let (sock, port) = create_listener_ipv6(reactor.clone());
    for _ in 0..10 {
        connect_test_ipv6(reactor.clone(), port);
    }
    accept_multi_test(reactor.clone(), sock, 10);

    let (sock, port) = create_listener_ipv4(reactor.clone());
    connect_test_ipv4(reactor.clone(), port);
    accept_multi_once_test(reactor.clone(), sock);

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(5)]
fn shutdown() {
    let reactor = reactor();

    for how in [Shutdown::Read, Shutdown::Write, Shutdown::Both] {
        let (listener, port) = create_listener_ipv4(reactor.clone());
        let client = connect_test_ipv4(reactor.clone(), port);
        let server = accept_test(reactor.clone(), listener).0;

        assert!(shutdown_test(reactor.clone(), client, how).is_ok());
        assert!(shutdown_test(reactor.clone(), server, how).is_ok());
    }

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(5)]
fn errors() {
    let reactor = reactor();

    create_socket_error_test(reactor.clone());

    connect_error_test_ipv4(reactor.clone());
    connect_error_test_ipv6(reactor.clone());

    accept_error_test(
        reactor.clone(),
        create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM),
    );
    accept_error_test(
        reactor.clone(),
        create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
    );

    accept_multi_error_test(
        reactor.clone(),
        create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM),
    );
    accept_multi_error_test(
        reactor.clone(),
        create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
    );

    assert!(bind(i32::MAX, make_addr("127.0.0.1", u16::MAX)).is_err());
    assert!(listen(i32::MAX, i32::MAX as u32).is_err());
    assert!(getsockname(i32::MAX).is_err());
    assert!(getpeername(i32::MAX).is_err());

    assert!(shutdown_test(
        reactor.clone(),
        create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM),
        Shutdown::Both
    )
    .is_err());

    assert!(reactor.is_done());
}

#[test]
#[test_repeat(10)]
fn cancel() {
    let reactor = reactor();

    create_socket_cancel_test(reactor.clone());

    connect_cancel_test_ipv4(reactor.clone());
    connect_cancel_test_ipv6(reactor.clone());

    accept_cancel_test(reactor.clone(), create_listener_ipv4(reactor.clone()).0);
    accept_cancel_test(reactor.clone(), create_listener_ipv6(reactor.clone()).0);

    assert!(reactor.is_done());
}

#[test]
fn read_write() {
    let (reactor, notifier) = runtime();

    let (listener, port) = create_listener_ipv4(reactor.clone());
    let conn1 = connect_test_ipv4(reactor.clone(), port);
    let (conn2, _) = accept_test(reactor.clone(), listener);

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

mod direct {
    use super::*;

    #[test]
    #[test_repeat(5)]
    fn socket() {
        let reactor = reactor();

        let s1 = create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
        let s2 = create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

        let s3 = create_fixed_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
        let s4 = create_fixed_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

        create_auto_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
        create_auto_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

        create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
        create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

        reactor.release_file_slot(s1);
        reactor.release_file_slot(s2);
        reactor.release_file_slot(s3);
        reactor.release_file_slot(s4);

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(5)]
    fn connect() {
        let reactor = reactor();
        let mut slots = vec![];

        let (_, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let slot = connect_fixed_test_ipv4(reactor.clone(), port);
            slots.push(slot);
        }

        let (_, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let slot = connect_fixed_test_ipv6(reactor.clone(), port);
            slots.push(slot);
        }

        for slot in slots {
            reactor.release_file_slot(slot);
        }

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(5)]
    fn accept() {
        let reactor = reactor();
        let mut slots = vec![];

        let (slot4, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv4(reactor.clone(), port);
            slots.push(s);
        }
        for _ in 0..10 {
            let (s, _) = accept_fixed_test(reactor.clone(), slot4);
            slots.push(s);
        }

        let (slot6, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv6(reactor.clone(), port);
            slots.push(s);
        }
        for _ in 0..10 {
            accept_auto_test(reactor.clone(), slot6);
        }

        for slot in slots {
            reactor.release_file_slot(slot);
        }

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(5)]
    fn accept_multi() {
        let reactor = reactor();
        let mut slots = vec![];

        let (slot4, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv4(reactor.clone(), port);
            slots.push(s);
        }
        accept_multi_direct_test(reactor.clone(), slot4, 10);

        let (slot6, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv6(reactor.clone(), port);
            slots.push(s);
        }
        accept_multi_direct_test(reactor.clone(), slot6, 10);

        let (slot4, port) = create_fixed_listener_ipv4(reactor.clone());
        let s = connect_fixed_test_ipv4(reactor.clone(), port);
        slots.push(s);
        accept_multi_direct_once_test(reactor.clone(), slot4);

        for slot in slots {
            reactor.release_file_slot(slot);
        }

        assert!(reactor.is_done());
    }

    #[test]
    #[test_repeat(5)]
    fn errors() {
        let reactor = reactor();

        create_fixed_socket_error_test(reactor.clone());
        create_auto_socket_error_test(reactor.clone());

        accept_fixed_error_test(
            reactor.clone(),
            create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM),
        );
        accept_auto_error_test(
            reactor.clone(),
            create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
        );

        accept_multi_direct_error_test(
            reactor.clone(),
            create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
        );
    }

    #[test]
    #[test_repeat(10)]
    fn cancel() {
        let reactor = reactor();

        for _ in 0..10 {
            create_fixed_socket_cancel_test(reactor.clone());
            create_auto_socket_cancel_test(reactor.clone());
        }

        accept_fixed_cancel_test(
            reactor.clone(),
            create_fixed_listener_ipv4(reactor.clone()).0,
        );
        accept_auto_cancel_test(
            reactor.clone(),
            create_fixed_listener_ipv6(reactor.clone()).0,
        );
    }

    #[test]
    fn read_write() {
        let (reactor, notifier) = runtime();

        let (listener, port) = create_fixed_listener_ipv4(reactor.clone());
        let conn1 = connect_fixed_test_ipv4(reactor.clone(), port);
        let (conn2, _) = accept_auto_test(reactor.clone(), listener);

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

        reactor.release_file_slot(conn1);
        assert!(reactor.is_done());
    }
}
