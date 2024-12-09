use std::{
    future::Future,
    io::{self, Result},
    net::{Shutdown, SocketAddr, ToSocketAddrs},
    os::fd::{AsRawFd, RawFd},
};

use inel_reactor::{
    op::{self, Op},
    util,
};

use crate::{
    io::{ReadSource, WriteSource},
    GlobalReactor,
};

const DEFAULT_LISTEN_BACKLOG: u32 = 4096;

async fn for_each_addr<A, F, H, T>(addr: A, f: F) -> Result<T>
where
    A: ToSocketAddrs,
    F: Fn(SocketAddr) -> H,
    H: Future<Output = Result<T>>,
{
    let mut last_error = None;
    for addr in addr.to_socket_addrs()? {
        match f(addr).await {
            Ok(res) => return Ok(res),
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or(io::Error::new(
        io::ErrorKind::InvalidInput,
        "could not resolve any addresses",
    )))
}

pub struct TcpListener {
    sock: RawFd,
}

impl TcpListener {
    pub async fn bind<A>(addr: A) -> Result<Self>
    where
        A: ToSocketAddrs,
    {
        for_each_addr(addr, |addr| async move {
            let domain = if addr.is_ipv4() {
                libc::AF_INET
            } else {
                libc::AF_INET6
            };

            let sock = op::Socket::new(domain, libc::SOCK_STREAM)
                .run_on(GlobalReactor)
                .await?;

            util::bind(sock, addr)?;
            util::listen(sock, DEFAULT_LISTEN_BACKLOG)?;

            Ok(Self { sock })
        })
        .await
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        util::getsockname(self.sock)
    }

    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (sock, peer) = op::Accept::new(self.sock).run_on(GlobalReactor).await?;
        Ok((TcpStream { sock }, peer))
    }
}

impl AsRawFd for TcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.sock
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        crate::util::spawn_drop(self.as_raw_fd());
    }
}

pub struct TcpStream {
    sock: RawFd,
}

impl ReadSource for TcpStream {}
impl WriteSource for TcpStream {}

impl TcpStream {
    pub async fn connect<A>(addr: A) -> Result<Self>
    where
        A: ToSocketAddrs,
    {
        for_each_addr(addr, |addr| async move {
            let domain = if addr.is_ipv4() {
                libc::AF_INET
            } else {
                libc::AF_INET6
            };

            let sock = op::Socket::new(domain, libc::SOCK_STREAM)
                .run_on(GlobalReactor)
                .await?;

            let sock = op::Connect::new(sock, addr).run_on(GlobalReactor).await?;

            Ok(Self { sock })
        })
        .await
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        util::getsockname(self.sock)
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        util::getpeername(self.sock)
    }

    pub async fn shutdown(&self, how: Shutdown) -> Result<()> {
        op::Shutdown::new(self.sock, how)
            .run_on(GlobalReactor)
            .await
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.sock
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        crate::util::spawn_drop(self.as_raw_fd());
    }
}
