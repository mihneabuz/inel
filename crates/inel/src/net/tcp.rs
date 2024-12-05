use std::{
    io::Result,
    net::SocketAddr,
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

pub struct TcpListener {
    sock: RawFd,
}

impl TcpListener {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
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

pub struct TcpStream {
    sock: RawFd,
}

impl ReadSource for TcpStream {}
impl WriteSource for TcpStream {}

impl TcpStream {
    pub fn local_addr(&self) -> Result<SocketAddr> {
        util::getsockname(self.sock)
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.sock
    }
}
