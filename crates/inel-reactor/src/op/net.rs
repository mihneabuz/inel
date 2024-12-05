use std::{
    io::{Error, Result},
    mem::{self, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
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
    g: libc::sockaddr,
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

impl SocketAddrCRepr {
    pub fn as_ptr(&self) -> *const libc::sockaddr {
        self as *const _ as *const libc::sockaddr
    }
}

fn into_raw_addr(addr: SocketAddr) -> (SocketAddrCRepr, u32) {
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

fn from_raw_addr(addr: SocketAddrCRepr, len: u32) -> SocketAddr {
    unsafe {
        match addr.g.sa_family as i32 {
            libc::AF_INET => {
                assert!(len as usize >= mem::size_of::<libc::sockaddr_in>());
                SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from(addr.v4.sin_addr.s_addr.to_ne_bytes()),
                    u16::from_be(addr.v4.sin_port),
                ))
            }

            libc::AF_INET6 => {
                assert!(len as usize >= mem::size_of::<libc::sockaddr_in6>());
                SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(addr.v6.sin6_addr.s6_addr),
                    u16::from_be(addr.v6.sin6_port),
                    addr.v6.sin6_flowinfo,
                    addr.v6.sin6_scope_id,
                ))
            }

            _ => unreachable!(),
        }
    }
}

pub struct Connect {
    fd: RawFd,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Connect {
    pub fn new(fd: RawFd, addr: SocketAddr) -> Self {
        let (addr, len) = into_raw_addr(addr);
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

pub struct Accept {
    fd: RawFd,
    addr: Option<Box<(MaybeUninit<SocketAddrCRepr>, MaybeUninit<u32>)>>,
}

impl Accept {
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            addr: Some(Box::new((MaybeUninit::uninit(), MaybeUninit::uninit()))),
        }
    }
}

unsafe impl Op for Accept {
    type Output = Result<(RawFd, SocketAddr)>;

    fn entry(&mut self) -> Entry {
        let addr = self.addr.as_mut().unwrap();
        opcode::Accept::new(
            Fd(self.fd),
            addr.0.as_mut_ptr() as *mut _,
            addr.1.as_mut_ptr(),
        )
        .build()
    }

    fn result(mut self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            let res = self.addr.take().unwrap();
            let (addr, len) = unsafe { (res.0.assume_init(), res.1.assume_init()) };
            Ok((ret, from_raw_addr(addr, len)))
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            self.addr.unwrap().into(),
        )
    }
}
