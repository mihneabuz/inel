use std::{
    io::{Error, Result},
    mem,
    net::SocketAddr,
    os::fd::RawFd,
};

use io_uring::{
    opcode::{self, AsyncCancel},
    squeue::Entry,
    types::Fd,
};

use crate::Cancellation;

use super::Op;

pub struct Socket {
    domain: i32,
    typ: i32,
    proto: i32,
}

impl Socket {
    pub fn new(domain: i32, typ: i32) -> Self {
        Self {
            domain,
            typ,
            proto: 0,
        }
    }

    pub fn proto(mut self, proto: i32) -> Self {
        self.proto = proto;
        self
    }
}

unsafe impl Op for Socket {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::Socket::new(self.domain, self.typ, self.proto).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret)
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}

union SocketAddrCRepr {
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

impl SocketAddrCRepr {
    pub fn as_ptr(&self) -> *const libc::sockaddr {
        self as *const _ as *const libc::sockaddr
    }
}

fn raw_addr(addr: SocketAddr) -> (SocketAddrCRepr, u32) {
    match addr {
        SocketAddr::V4(addr_v4) => (
            SocketAddrCRepr {
                v4: libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: addr_v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(addr_v4.ip().octets()),
                    },
                    ..unsafe { mem::zeroed() }
                },
            },
            mem::size_of::<libc::sockaddr_in>() as u32,
        ),

        SocketAddr::V6(addr_v6) => (
            SocketAddrCRepr {
                v6: libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: addr_v6.port().to_be(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: addr_v6.ip().octets(),
                    },
                    sin6_flowinfo: addr_v6.flowinfo(),
                    sin6_scope_id: addr_v6.scope_id(),
                },
            },
            mem::size_of::<libc::sockaddr_in6>() as u32,
        ),
    }
}

pub struct Connect {
    fd: RawFd,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Connect {
    pub fn new(fd: RawFd, addr: SocketAddr) -> Self {
        let (addr, len) = raw_addr(addr);
        Self { fd, addr, len }
    }
}

unsafe impl Op for Connect {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.len).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(self.fd)
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}
