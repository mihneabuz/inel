use std::{
    io::{Error, Result},
    mem::{self, MaybeUninit},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::fd::RawFd,
};

pub(crate) union SocketAddrCRepr {
    g: libc::sockaddr,
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

impl SocketAddrCRepr {
    pub fn as_ptr(&self) -> *const libc::sockaddr {
        self as *const _ as *const libc::sockaddr
    }

    pub fn as_mut_ptr(&mut self) -> *mut libc::sockaddr {
        self as *mut _ as *mut libc::sockaddr
    }
}

pub(crate) fn into_raw_addr(addr: SocketAddr) -> (SocketAddrCRepr, u32) {
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

pub(crate) fn from_raw_addr(addr: SocketAddrCRepr, len: u32) -> SocketAddr {
    unsafe {
        match addr.g.sa_family as i32 {
            libc::AF_INET => {
                assert!(len as usize == mem::size_of::<libc::sockaddr_in>());
                SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from(addr.v4.sin_addr.s_addr.to_ne_bytes()),
                    u16::from_be(addr.v4.sin_port),
                ))
            }

            libc::AF_INET6 => {
                assert!(len as usize == mem::size_of::<libc::sockaddr_in6>());
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

pub fn bind(sock: RawFd, addr: SocketAddr) -> Result<()> {
    let (addr, len) = into_raw_addr(addr);
    let ret = unsafe { libc::bind(sock, addr.as_ptr() as *const _, len) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn listen(sock: RawFd, capacity: u32) -> Result<()> {
    let ret = unsafe { libc::listen(sock, capacity as i32) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn getsockname(sock: RawFd) -> Result<SocketAddr> {
    let mut addr: MaybeUninit<SocketAddrCRepr> = MaybeUninit::uninit();
    let mut len = std::mem::size_of::<SocketAddrCRepr>() as u32;

    let ret = unsafe { libc::getsockname(sock, addr.as_mut_ptr() as *mut _, &mut len) };
    if ret < 0 {
        Err(Error::last_os_error())
    } else {
        Ok(from_raw_addr(unsafe { addr.assume_init() }, len))
    }
}
