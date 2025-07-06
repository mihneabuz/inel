use crate::helpers::{assert_ready, notifier, poll, runtime, ScopedReactor};
use futures::{future::FusedFuture, StreamExt};
use inel_macro::test_repeat;
use libc::{AF_INET, SOCK_STREAM};
use std::{
    i32,
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
    op::{self, Op, OpExt},
    util::{getpeername, getsockname},
    DirectFd,
};

fn make_addr(ip: &str, port: u16) -> SocketAddr {
    SocketAddr::new(FromStr::from_str(ip).unwrap(), port)
}

fn complete_op<T: Op>(reactor: ScopedReactor, op: T) -> T::Output {
    let notifier = notifier();
    let mut fut = pin!(op.run_on(reactor.clone()));

    assert!(poll!(fut, notifier).is_pending());

    reactor.wait();

    assert_eq!(notifier.try_recv(), Some(()));
    assert_ready!(poll!(fut, notifier))
}

fn cancel_op<T: Op>(reactor: ScopedReactor, op: T) {
    let notifier = notifier();
    let mut op = op.run_on(reactor.clone());
    let mut fut = pin!(&mut op);

    assert!(poll!(fut, notifier).is_pending());
    std::mem::drop(op);

    reactor.wait();
    assert_eq!(notifier.try_recv(), None);

    reactor.wait();
    assert_eq!(notifier.try_recv(), None);

    assert_eq!(reactor.active(), 0);
}

fn create_socket_test(reactor: ScopedReactor, domain: i32, typ: i32) -> RawFd {
    let sock = complete_op(reactor, op::Socket::new(domain, typ).proto(0));
    assert!(sock.is_ok());
    sock.unwrap()
}

fn create_socket_error_test(reactor: ScopedReactor) {
    let sock = complete_op(reactor, op::Socket::new(i32::MAX, i32::MAX).proto(0));
    assert!(sock.is_err());
}

fn create_socket_cancel_test(reactor: ScopedReactor) {
    cancel_op(reactor, op::Socket::new(AF_INET, SOCK_STREAM).proto(0));
}

fn create_fixed_socket_test(
    reactor: ScopedReactor,
    domain: i32,
    typ: i32,
) -> DirectFd<ScopedReactor> {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    let res = complete_op(
        reactor,
        op::Socket::new(domain, typ).proto(0).fixed(direct.slot()),
    );
    assert!(res.is_ok());
    direct
}

fn create_fixed_socket_error_test(reactor: ScopedReactor) {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    let res = complete_op(
        reactor.clone(),
        op::Socket::new(i32::MAX, i32::MAX)
            .proto(0)
            .fixed(&direct.slot()),
    );
    assert!(res.is_err());
    direct.release();
}

fn create_fixed_socket_cancel_test(reactor: ScopedReactor) {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    cancel_op(
        reactor.clone(),
        op::Socket::new(AF_INET, SOCK_STREAM).fixed(direct.slot()),
    );
    direct.release();
}

fn create_auto_socket_test(
    reactor: ScopedReactor,
    domain: i32,
    typ: i32,
) -> DirectFd<ScopedReactor> {
    let res = complete_op(
        reactor.clone(),
        op::Socket::new(domain, typ).proto(0).direct(),
    );
    assert!(res.is_ok());
    DirectFd::from_slot(res.unwrap(), reactor)
}

fn create_auto_socket_error_test(reactor: ScopedReactor) {
    let res = complete_op(
        reactor,
        op::Socket::new(i32::MAX, i32::MAX).proto(0).direct(),
    );
    assert!(res.is_err());
}

fn create_auto_socket_cancel_test(reactor: ScopedReactor) {
    cancel_op(reactor, op::Socket::new(AF_INET, SOCK_STREAM).direct());
}

fn connect_test(reactor: ScopedReactor, addr: &str, port: u16) -> RawFd {
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

    let res = complete_op(reactor, op::Connect::new(&sock, addr));
    assert!(res.is_ok());

    sock
}

fn connect_fixed_test(reactor: ScopedReactor, addr: &str, port: u16) -> DirectFd<ScopedReactor> {
    let addr = make_addr(addr, port);
    let direct = create_fixed_socket_test(
        reactor.clone(),
        if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        },
        libc::SOCK_STREAM,
    );

    let res = complete_op(reactor, op::Connect::new(&direct, addr));
    assert!(res.is_ok());

    direct
}

fn connect_test_ipv4(reactor: ScopedReactor, port: u16) -> RawFd {
    connect_test(reactor, "127.0.0.1", port)
}

fn connect_test_ipv6(reactor: ScopedReactor, port: u16) -> RawFd {
    connect_test(reactor, "::1", port)
}

fn connect_fixed_test_ipv4(reactor: ScopedReactor, port: u16) -> DirectFd<ScopedReactor> {
    connect_fixed_test(reactor, "127.0.0.1", port)
}

fn connect_fixed_test_ipv6(reactor: ScopedReactor, port: u16) -> DirectFd<ScopedReactor> {
    connect_fixed_test(reactor, "::1", port)
}

fn connect_error_test(reactor: ScopedReactor, addr: &str) {
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

    let res = complete_op(reactor, op::Connect::new(&sock, addr));
    assert!(res.is_err());
}

fn connect_error_test_ipv4(reactor: ScopedReactor) {
    connect_error_test(reactor, "127.0.0.1");
}

fn connect_error_test_ipv6(reactor: ScopedReactor) {
    connect_error_test(reactor, "::1");
}

fn connect_cancel_test(reactor: ScopedReactor, addr: &str) {
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

    cancel_op(reactor, op::Connect::new(&sock, addr));
}

fn connect_cancel_test_ipv4(reactor: ScopedReactor) {
    connect_cancel_test(reactor, "127.0.0.1");
}

fn connect_cancel_test_ipv6(reactor: ScopedReactor) {
    connect_cancel_test(reactor, "::1");
}

fn accept_test(reactor: ScopedReactor, sock: RawFd) -> (RawFd, SocketAddr) {
    let addr = complete_op(reactor, op::Accept::new(&sock));
    assert!(addr.is_ok());
    assert!(addr.as_ref().unwrap().0 > 0);
    assert!(addr.as_ref().unwrap().1.ip().is_loopback());
    addr.unwrap()
}

fn accept_error_test(reactor: ScopedReactor, sock: RawFd) {
    let addr = complete_op(reactor, op::Accept::new(&sock));
    assert!(addr.is_err());
}

fn accept_cancel_test(reactor: ScopedReactor, sock: RawFd) {
    cancel_op(reactor, op::Accept::new(&sock));
}

fn accept_fixed_test(
    reactor: ScopedReactor,
    listener: &DirectFd<ScopedReactor>,
) -> (DirectFd<ScopedReactor>, SocketAddr) {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    let addr = complete_op(reactor, op::Accept::new(listener).fixed(direct.slot()));
    assert!(addr.is_ok());
    assert!(addr.as_ref().unwrap().ip().is_loopback());
    (direct, addr.unwrap())
}

fn accept_fixed_error_test(reactor: ScopedReactor, listener: &DirectFd<ScopedReactor>) {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    let addr = complete_op(
        reactor.clone(),
        op::Accept::new(listener).fixed(direct.slot()),
    );
    assert!(addr.is_err());
    direct.release();
}

fn accept_fixed_cancel_test(reactor: ScopedReactor, listener: &DirectFd<ScopedReactor>) {
    let direct = DirectFd::get(reactor.clone()).unwrap();
    cancel_op(
        reactor.clone(),
        op::Accept::new(listener).fixed(direct.slot()),
    );
    direct.release();
}

fn accept_auto_test(
    reactor: ScopedReactor,
    listener: &DirectFd<ScopedReactor>,
) -> (DirectFd<ScopedReactor>, SocketAddr) {
    let addr = complete_op(reactor.clone(), op::Accept::new(listener).direct());
    assert!(addr.is_ok());
    assert!(addr.as_ref().unwrap().1.ip().is_loopback());
    let (slot, addr) = addr.unwrap();
    (DirectFd::from_slot(slot, reactor), addr)
}

fn accept_auto_error_test(reactor: ScopedReactor, listener: &DirectFd<ScopedReactor>) {
    let addr = complete_op(reactor, op::Accept::new(listener).direct());
    assert!(addr.is_err());
}

fn accept_auto_cancel_test(reactor: ScopedReactor, listener: &DirectFd<ScopedReactor>) {
    cancel_op(reactor, op::Accept::new(listener).direct());
}

fn accept_multi_test(reactor: ScopedReactor, sock: RawFd, count: usize) {
    let notifier = notifier();
    let mut con = op::AcceptMulti::new(&sock).run_on(reactor.clone());
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

fn accept_multi_direct_test(reactor: ScopedReactor, direct: DirectFd<ScopedReactor>, count: usize) {
    let notifier = notifier();

    let mut con = op::AcceptMulti::new(&direct)
        .direct()
        .run_on(reactor.clone());
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
    let res = complete_op(reactor.clone(), op::AcceptMulti::new(&sock));
    assert!(res.is_ok());
    reactor.wait();
}

fn accept_multi_direct_once_test(reactor: ScopedReactor, direct: DirectFd<ScopedReactor>) {
    let res = complete_op(reactor.clone(), op::AcceptMulti::new(&direct));
    assert!(res.is_ok());
    reactor.wait();
}

fn accept_multi_error_test(reactor: ScopedReactor, sock: RawFd) {
    let res = complete_op(reactor.clone(), op::AcceptMulti::new(&sock));
    assert!(res.is_err());
    reactor.wait();
}

fn accept_multi_direct_error_test(reactor: ScopedReactor, direct: DirectFd<ScopedReactor>) {
    let res = complete_op(reactor.clone(), op::AcceptMulti::new(&direct));
    assert!(res.is_err());
    reactor.wait();
}

fn shutdown_test(reactor: ScopedReactor, sock: RawFd, how: Shutdown) -> Result<()> {
    complete_op(reactor, op::Shutdown::new(&sock, how))
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

    assert!(complete_op(reactor.clone(), op::Bind::new(&sock, addr)).is_ok());
    assert!(complete_op(reactor.clone(), op::Listen::new(&sock, 32)).is_ok());

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

fn create_fixed_listener_ipv4(reactor: ScopedReactor) -> (DirectFd<ScopedReactor>, u16) {
    let (fd, port) = create_listener_ipv4(reactor.clone());
    (reactor.register_file(fd), port)
}

fn create_fixed_listener_ipv6(reactor: ScopedReactor) -> (DirectFd<ScopedReactor>, u16) {
    let (fd, port) = create_listener_ipv6(reactor.clone());
    (reactor.register_file(fd), port)
}

#[test]
fn socket() {
    let (reactor, _) = runtime();

    create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
    create_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

    create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
    create_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

    assert!(reactor.is_done());
}

#[test]
fn connect() {
    let (reactor, _) = runtime();

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
fn accept() {
    let (reactor, _) = runtime();

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
fn accept_multi() {
    let (reactor, _) = runtime();

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
fn shutdown() {
    let (reactor, _) = runtime();

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
fn errors() {
    let (reactor, _) = runtime();

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
    let (reactor, _) = runtime();

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

    let write = op::Write::new(&conn1, Box::new([b'A'; 4096])).run_on(reactor.clone());
    let read = op::Read::new(&conn2, Box::new([0; 8192])).run_on(reactor.clone());

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
    fn socket() {
        let (reactor, _) = runtime();

        let s1 = create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
        let s2 = create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

        let s3 = create_fixed_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
        let s4 = create_fixed_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

        create_auto_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM);
        create_auto_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_DGRAM);

        create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM);
        create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_DGRAM);

        s1.release();
        s2.release();
        s3.release();
        s4.release();

        assert!(reactor.is_done());
    }

    #[test]
    fn connect() {
        let (reactor, _) = runtime();
        let mut directs = vec![];

        let (_, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let direct = connect_fixed_test_ipv4(reactor.clone(), port);
            directs.push(direct);
        }

        let (_, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let direct = connect_fixed_test_ipv6(reactor.clone(), port);
            directs.push(direct);
        }

        for direct in directs {
            direct.release();
        }

        assert!(reactor.is_done());
    }

    #[test]
    fn accept() {
        let (reactor, _) = runtime();
        let mut directs = vec![];

        let (direct4, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv4(reactor.clone(), port);
            directs.push(s);
        }
        for _ in 0..10 {
            let (s, _) = accept_fixed_test(reactor.clone(), &direct4);
            directs.push(s);
        }

        let (direct6, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv6(reactor.clone(), port);
            directs.push(s);
        }
        for _ in 0..10 {
            accept_auto_test(reactor.clone(), &direct6);
        }

        for direct in directs {
            direct.release();
        }

        assert!(reactor.is_done());
    }

    #[test]
    fn accept_multi() {
        let (reactor, _) = runtime();
        let mut directs = vec![];

        let (direct4, port) = create_fixed_listener_ipv4(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv4(reactor.clone(), port);
            directs.push(s);
        }
        accept_multi_direct_test(reactor.clone(), direct4, 10);

        let (direct6, port) = create_fixed_listener_ipv6(reactor.clone());
        for _ in 0..10 {
            let s = connect_fixed_test_ipv6(reactor.clone(), port);
            directs.push(s);
        }
        accept_multi_direct_test(reactor.clone(), direct6, 10);

        let (direct4, port) = create_fixed_listener_ipv4(reactor.clone());
        let s = connect_fixed_test_ipv4(reactor.clone(), port);
        directs.push(s);
        accept_multi_direct_once_test(reactor.clone(), direct4);

        for direct in directs {
            direct.release();
        }

        assert!(reactor.is_done());
    }

    #[test]
    fn errors() {
        let (reactor, _) = runtime();

        create_fixed_socket_error_test(reactor.clone());
        create_auto_socket_error_test(reactor.clone());

        accept_fixed_error_test(
            reactor.clone(),
            &create_fixed_socket_test(reactor.clone(), libc::AF_INET, libc::SOCK_STREAM),
        );
        accept_auto_error_test(
            reactor.clone(),
            &create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
        );

        accept_multi_direct_error_test(
            reactor.clone(),
            create_auto_socket_test(reactor.clone(), libc::AF_INET6, libc::SOCK_STREAM),
        );
    }

    #[test]
    #[test_repeat(10)]
    fn cancel() {
        let (reactor, _) = runtime();

        for _ in 0..10 {
            create_fixed_socket_cancel_test(reactor.clone());
            create_auto_socket_cancel_test(reactor.clone());
        }

        accept_fixed_cancel_test(
            reactor.clone(),
            &create_fixed_listener_ipv4(reactor.clone()).0,
        );
        accept_auto_cancel_test(
            reactor.clone(),
            &create_fixed_listener_ipv6(reactor.clone()).0,
        );
    }

    #[test]
    fn read_write() {
        let (reactor, notifier) = runtime();

        let (listener, port) = create_fixed_listener_ipv4(reactor.clone());
        let conn1 = connect_fixed_test_ipv4(reactor.clone(), port);
        let (conn2, _) = accept_auto_test(reactor.clone(), &listener);

        let write = op::Write::new(&conn1, Box::new([b'A'; 4096])).run_on(reactor.clone());
        let read = op::Read::new(&conn2, Box::new([0; 8192])).run_on(reactor.clone());

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

        conn1.release();
        assert!(reactor.is_done());
    }
}
