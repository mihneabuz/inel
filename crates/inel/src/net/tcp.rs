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
    op::{self, AcceptMulti, Op},
    util, AsSource, Source, Submission,
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

    pub fn incoming(self) -> Incoming {
        Incoming::new(self)
    }
}

impl AsRawFd for TcpListener {
    fn as_raw_fd(&self) -> RawFd {
        self.sock
    }
}

impl IntoRawFd for TcpListener {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.as_raw_fd();
        std::mem::forget(self);
        fd
    }
}

impl FromRawFd for TcpListener {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self { sock: fd }
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        crate::util::spawn_drop(self.as_raw_fd());
    }
}

pub struct Incoming {
    #[allow(dead_code)]
    listener: TcpListener,
    stream: Submission<AcceptMulti, GlobalReactor>,
}

impl Incoming {
    pub fn new(listener: TcpListener) -> Self {
        let stream = AcceptMulti::new(listener.as_raw_fd()).run_on(GlobalReactor);
        Self { listener, stream }
    }
}

impl Stream for Incoming {
    type Item = Result<TcpStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::into_inner(self)
            .stream
            .poll_next_unpin(cx)
            .map(|next| next.map(|res| res.map(|sock| TcpStream { sock })))
    }
}

pub struct TcpStream {
    sock: RawFd,
}

impl Debug for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpStream").finish()
    }
}

impl ReadSource for TcpStream {
    fn read_source(&self) -> Source {
        self.as_raw_fd().as_source()
    }
}

impl WriteSource for TcpStream {
    fn write_source(&self) -> Source {
        self.as_raw_fd().as_source()
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

            op::Connect::new(sock, addr).run_on(GlobalReactor).await?;

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

impl IntoRawFd for TcpStream {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.as_raw_fd();
        std::mem::forget(self);
        fd
    }
}

impl FromRawFd for TcpStream {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self { sock: fd }
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        crate::util::spawn_drop(self.as_raw_fd());
    }
}
