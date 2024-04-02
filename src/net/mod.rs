use std::{
    io::Result,
    net::{self as sync, SocketAddr, ToSocketAddrs},
    ops::{Deref, DerefMut},
    os::{
        fd::{FromRawFd, IntoRawFd},
        unix::io::RawFd,
    },
};

use crate::reactor::socket::{accept, bind, connect, listen, socket, Domain, SockType};

pub struct TcpListener {
    inner: sync::TcpListener,
}

impl Deref for TcpListener {
    type Target = sync::TcpListener;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TcpListener {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl FromRawFd for TcpListener {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::from_std(sync::TcpListener::from_raw_fd(fd))
    }
}

impl IntoRawFd for TcpListener {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}

impl TcpListener {
    pub fn from_std(listener: sync::TcpListener) -> Self {
        Self { inner: listener }
    }

    pub fn into_std(self) -> sync::TcpListener {
        self.inner
    }

    pub async fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = addr.to_socket_addrs()?.next().unwrap();

        let domain = match &addr {
            sync::SocketAddr::V4(_) => Domain::Inet,
            sync::SocketAddr::V6(_) => Domain::Inet6,
        };

        // open a socket async
        let sock = socket(domain, SockType::Stream).await?;

        // bind the address
        bind(sock, addr)?;

        // listen on the socket
        listen(sock, 128)?;

        Ok(unsafe { Self::from_raw_fd(sock) })
    }

    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (sock, addr) = accept(&self.inner).await?;
        Ok((unsafe { TcpStream::from_raw_fd(sock) }, addr))
    }
}

pub struct TcpStream {
    inner: sync::TcpStream,
}

impl Deref for TcpStream {
    type Target = sync::TcpStream;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TcpStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl FromRawFd for TcpStream {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::from_std(sync::TcpStream::from_raw_fd(fd))
    }
}

impl IntoRawFd for TcpStream {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}

impl TcpStream {
    pub fn from_std(stream: sync::TcpStream) -> Self {
        Self { inner: stream }
    }

    pub fn into_std(self) -> sync::TcpStream {
        self.inner
    }

    pub async fn connect(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = addr.to_socket_addrs()?.next().unwrap();

        let domain = match &addr {
            sync::SocketAddr::V4(_) => Domain::Inet,
            sync::SocketAddr::V6(_) => Domain::Inet6,
        };

        // open a socket async
        let sock = socket(domain, SockType::Stream).await?;

        // connect to addr
        connect(sock, addr).await?;

        Ok(unsafe { Self::from_raw_fd(sock) })
    }
}
