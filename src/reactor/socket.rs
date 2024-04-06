use std::{
    io::{Error, Result},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::fd::{AsRawFd, RawFd},
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::Future;

use super::handle::RingHandle;

pub fn socket(domain: Domain, stype: SockType) -> Socket {
    Socket {
        domain,
        stype,
        ring: RingHandle::default(),
    }
}

pub enum Domain {
    Unix,
    Inet,
    Inet6,
    Packet,
}

impl Domain {
    pub fn as_raw(&self) -> i32 {
        match self {
            Domain::Unix => libc::AF_UNIX,
            Domain::Inet => libc::AF_INET,
            Domain::Inet6 => libc::AF_INET6,
            Domain::Packet => libc::AF_PACKET,
        }
    }
}

pub enum SockType {
    Stream,
    Dgram,
    Raw,
}

impl SockType {
    pub fn as_raw(&self) -> i32 {
        match self {
            SockType::Stream => libc::SOCK_STREAM,
            SockType::Dgram => libc::SOCK_DGRAM,
            SockType::Raw => libc::SOCK_RAW,
        }
    }
}

pub struct Socket {
    domain: Domain,
    stype: SockType,
    ring: RingHandle,
}

impl Future for Socket {
    type Output = Result<RawFd>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let domain = self.domain.as_raw();
        let stype = self.stype.as_raw();
        self.get_mut().ring.socket(cx, domain, stype)
    }
}

pub fn bind(sock: RawFd, addr: SocketAddr) -> Result<()> {
    let ret = match addr {
        SocketAddr::V4(inet) => {
            let addr = libc::sockaddr_in {
                sin_family: libc::AF_INET as u16,
                sin_port: inet.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(inet.ip().octets()),
                },
                sin_zero: [0u8; 8],
            };

            let len = std::mem::size_of::<libc::sockaddr_in>() as u32;
            let ptr = (&addr) as *const libc::sockaddr_in;

            unsafe { libc::bind(sock, ptr.cast::<libc::sockaddr>(), len) }
        }

        SocketAddr::V6(inet6) => {
            let addr = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as u16,
                sin6_port: inet6.port(),
                sin6_flowinfo: inet6.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: inet6.ip().octets(),
                },
                sin6_scope_id: inet6.scope_id(),
            };

            let len = std::mem::size_of::<libc::sockaddr_in6>() as u32;
            let ptr = (&addr) as *const libc::sockaddr_in6;

            unsafe { libc::bind(sock, ptr.cast::<libc::sockaddr>(), len) }
        }
    };

    match ret {
        0 => Ok(()),
        -1 => Err(Error::last_os_error()),
        _ => unreachable!(),
    }
}

pub fn listen(sock: RawFd, backlog: i32) -> Result<()> {
    let ret = unsafe { libc::listen(sock, backlog) };

    match ret {
        0 => Ok(()),
        -1 => Err(Error::last_os_error()),
        _ => unreachable!(),
    }
}

pub fn accept<T>(listener: &T) -> Accept<'_, T> {
    let addr = libc::sockaddr_in6 {
        sin6_family: 0,
        sin6_port: 0,
        sin6_flowinfo: 0,
        sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
        sin6_scope_id: 0,
    };

    Accept {
        listener,
        addr,
        ring: RingHandle::default(),
    }
}

pub struct Accept<'a, T> {
    listener: &'a T,
    addr: libc::sockaddr_in6,
    ring: RingHandle,
}

impl<T> Future for Accept<'_, T>
where
    T: AsRawFd,
{
    type Output = Result<(RawFd, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let fd = this.listener.as_raw_fd();

        let mut len = std::mem::size_of::<libc::sockaddr_in6>() as u32;
        let ptr = (&mut this.addr) as *mut libc::sockaddr_in6;

        let sock = ready!(unsafe {
            this.ring
                .accept(cx, fd, ptr.cast::<libc::sockaddr>(), &mut len)
        })?;

        let addr = match this.addr.sin6_family as i32 {
            libc::AF_INET => {
                let addr = ptr.cast::<libc::sockaddr_in>();

                SocketAddr::V4(SocketAddrV4::new(
                    Ipv4Addr::from(unsafe { (*addr).sin_addr.s_addr.to_be() }),
                    unsafe { (*addr).sin_port.to_be() },
                ))
            }

            libc::AF_INET6 => {
                let mut sin6_addr = this.addr.sin6_addr.s6_addr;
                sin6_addr.reverse();

                SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(sin6_addr),
                    this.addr.sin6_port.to_be(),
                    this.addr.sin6_flowinfo,
                    this.addr.sin6_scope_id,
                ))
            }

            _ => unreachable!(),
        };

        Poll::Ready(Ok((sock, addr)))
    }
}

pub fn connect(sock: RawFd, addr: SocketAddr) -> Connect {
    let mut sockaddr = libc::sockaddr_in6 {
        sin6_family: 0,
        sin6_port: 0,
        sin6_flowinfo: 0,
        sin6_addr: libc::in6_addr { s6_addr: [0; 16] },
        sin6_scope_id: 0,
    };

    let ptr_in6 = (&mut sockaddr) as *mut libc::sockaddr_in6;
    let ptr_in = ptr_in6.cast::<libc::sockaddr_in>();

    let socklen = match addr {
        SocketAddr::V4(inet) => unsafe {
            (*ptr_in).sin_family = libc::AF_INET as u16;
            (*ptr_in).sin_port = inet.port().to_be();
            (*ptr_in).sin_addr.s_addr = u32::from_ne_bytes(inet.ip().octets());
            (*ptr_in).sin_zero = [0u8; 8];

            std::mem::size_of::<libc::sockaddr_in>() as u32
        },

        SocketAddr::V6(inet6) => unsafe {
            (*ptr_in6).sin6_family = libc::AF_INET6 as u16;
            (*ptr_in6).sin6_port = inet6.port().to_be();
            (*ptr_in6).sin6_addr.s6_addr = inet6.ip().octets();
            (*ptr_in6).sin6_flowinfo = inet6.flowinfo().to_be();
            (*ptr_in6).sin6_scope_id = inet6.scope_id().to_be();

            std::mem::size_of::<libc::sockaddr_in6>() as u32
        },
    };

    Connect {
        sock,
        addr: sockaddr,
        len: socklen,
        ring: RingHandle::default(),
    }
}

pub struct Connect {
    sock: RawFd,
    addr: libc::sockaddr_in6,
    len: u32,
    ring: RingHandle,
}

impl Future for Connect {
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        let sock = this.sock;
        let ptr_in6 = (&mut this.addr) as *mut libc::sockaddr_in6;
        let ptr = ptr_in6.cast::<libc::sockaddr>();
        let len = this.len;

        let ret = ready!(unsafe { this.ring.connect(cx, sock, ptr, len) })?;

        assert_eq!(ret, 0);

        Poll::Ready(Ok(()))
    }
}
