use std::{
    fmt::{self, Debug},
    future::Future,
    io::{self, Result},
    net::{Shutdown, SocketAddr, ToSocketAddrs},
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt};
use inel_reactor::{
    op::{self, AcceptMulti, AcceptMultiAuto, OpExt},
    util, AsSource, Source, Submission,
};

use crate::{
    io::{ReadSource, WriteSource},
    source::{OwnedDirect, OwnedFd},
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
    sock: OwnedFd,
}

impl Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpListener").finish()
    }
}

impl TcpListener {
    pub async fn bind<A>(addr: A) -> Result<Self>
    where
        A: ToSocketAddrs,
    {
        for_each_addr(addr, |addr| async move {
            let sock = op::Socket::stream_from_addr(&addr)
                .run_on(GlobalReactor)
                .await?;

            let sock = OwnedFd::from_raw(sock);

            op::Bind::new(&sock, addr).run_on(GlobalReactor).await?;

            op::Listen::new(&sock, DEFAULT_LISTEN_BACKLOG)
                .run_on(GlobalReactor)
                .await?;

            Ok(Self { sock })
        })
        .await
    }

    pub async fn bind_direct<A>(addr: A) -> Result<DirectTcpListener>
    where
        A: ToSocketAddrs,
    {
        let inner = Self::bind(addr).await?;

        let slot = op::RegisterFile::new(inner.sock.as_raw())
            .run_on(GlobalReactor)
            .await?;

        let direct = OwnedDirect::auto(slot);

        Ok(DirectTcpListener { inner, direct })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        util::getsockname(self.sock.as_raw())
    }

    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (sock, peer) = op::Accept::new(&self.sock).run_on(GlobalReactor).await?;

        Ok((unsafe { TcpStream::from_raw_fd(sock) }, peer))
    }

    pub fn incoming(self) -> Incoming {
        Incoming::new(self)
    }
}

impl AsRawFd for TcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.sock.as_raw()
    }
}

impl IntoRawFd for TcpListener {
    fn into_raw_fd(self) -> RawFd {
        self.sock.into_raw()
    }
}

impl FromRawFd for TcpListener {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            sock: OwnedFd::from_raw(fd),
        }
    }
}

pub struct Incoming {
    #[allow(dead_code)]
    listener: TcpListener,
    stream: Submission<AcceptMulti, GlobalReactor>,
}

impl Incoming {
    pub fn new(listener: TcpListener) -> Self {
        let stream = AcceptMulti::new(&listener.sock).run_on(GlobalReactor);
        Self { listener, stream }
    }
}

impl Stream for Incoming {
    type Item = Result<TcpStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::into_inner(self)
            .stream
            .poll_next_unpin(cx)
            .map(|next| next.map(|res| res.map(|sock| unsafe { TcpStream::from_raw_fd(sock) })))
    }
}

pub struct DirectIncoming {
    #[allow(dead_code)]
    listener: DirectTcpListener,
    stream: Submission<AcceptMultiAuto, GlobalReactor>,
}

impl DirectIncoming {
    pub fn new(listener: DirectTcpListener) -> Self {
        let stream = AcceptMulti::new(&listener.direct)
            .direct()
            .run_on(GlobalReactor);

        Self { listener, stream }
    }
}

impl Stream for DirectIncoming {
    type Item = Result<DirectTcpStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::into_inner(self)
            .stream
            .poll_next_unpin(cx)
            .map(|next| {
                next.map(|res| {
                    res.map(|slot| DirectTcpStream::from_direct(OwnedDirect::auto(slot)))
                })
            })
    }
}

pub struct TcpStream {
    sock: OwnedFd,
}

impl Debug for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpStream").finish()
    }
}

impl ReadSource for TcpStream {
    fn read_source(&self) -> Source {
        self.sock.as_source()
    }
}

impl WriteSource for TcpStream {
    fn write_source(&self) -> Source {
        self.sock.as_source()
    }
}

impl TcpStream {
    pub async fn connect<A>(addr: A) -> Result<Self>
    where
        A: ToSocketAddrs,
    {
        for_each_addr(addr, |addr| async move {
            let sock = op::Socket::stream_from_addr(&addr)
                .run_on(GlobalReactor)
                .await?;

            let sock = OwnedFd::from_raw(sock);

            op::Connect::new(&sock, addr).run_on(GlobalReactor).await?;

            Ok(Self { sock })
        })
        .await
    }

    pub async fn connect_direct<A>(addr: A) -> Result<DirectTcpStream>
    where
        A: ToSocketAddrs,
    {
        for_each_addr(addr, |addr| async move {
            let slot = op::Socket::stream_from_addr(&addr)
                .direct()
                .run_on(GlobalReactor)
                .await?;

            let slot = OwnedDirect::auto(slot);

            op::Connect::new(&slot, addr).run_on(GlobalReactor).await?;

            Ok(DirectTcpStream::from_direct(slot))
        })
        .await
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        util::getsockname(self.sock.as_raw())
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        util::getpeername(self.sock.as_raw())
    }

    pub async fn shutdown(&self, how: Shutdown) -> Result<()> {
        op::Shutdown::new(&self.sock, how)
            .run_on(GlobalReactor)
            .await
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.sock.as_raw()
    }
}

impl IntoRawFd for TcpStream {
    fn into_raw_fd(self) -> RawFd {
        self.sock.into_raw()
    }
}

impl FromRawFd for TcpStream {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            sock: OwnedFd::from_raw(fd),
        }
    }
}

pub struct DirectTcpListener {
    inner: TcpListener,
    direct: OwnedDirect,
}

impl Debug for DirectTcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectTcpListener").finish()
    }
}

impl DirectTcpListener {
    pub async fn accept(&self) -> Result<(DirectTcpStream, SocketAddr)> {
        let (slot, peer) = op::Accept::new(&self.direct)
            .direct()
            .run_on(GlobalReactor)
            .await?;

        let direct = OwnedDirect::auto(slot);

        Ok((DirectTcpStream::from_direct(direct), peer))
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.local_addr()
    }

    pub fn incoming(self) -> DirectIncoming {
        DirectIncoming::new(self)
    }
}

pub struct DirectTcpStream {
    direct: OwnedDirect,
}

impl Debug for DirectTcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectTcpStream").finish()
    }
}

impl ReadSource for DirectTcpStream {
    fn read_source(&self) -> Source {
        self.direct.as_source()
    }
}

impl WriteSource for DirectTcpStream {
    fn write_source(&self) -> Source {
        self.direct.as_source()
    }
}

impl DirectTcpStream {
    fn from_direct(direct: OwnedDirect) -> Self {
        Self { direct }
    }

    pub async fn shutdown(&self, how: Shutdown) -> Result<()> {
        op::Shutdown::new(&self.direct, how)
            .run_on(GlobalReactor)
            .await
    }
}
