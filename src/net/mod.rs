use std::pin::Pin;
use std::task::{Context, Poll};
use std::{future::Future, io, net::ToSocketAddrs};

use std::net::{self as sync, SocketAddr};

use crate::reactor::REACTOR;

#[derive(Debug)]
pub struct TcpListener {
    inner: sync::TcpListener,
}

impl TcpListener {
    pub fn bind<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let inner = sync::TcpListener::bind(addr)?;
        inner.set_nonblocking(true)?;
        Ok(Self { inner })
    }

    pub fn accept(&self) -> Accept {
        Accept {
            listener: &self.inner,
        }
    }
}

pub struct Accept<'a> {
    listener: &'a sync::TcpListener,
}

impl Future for Accept<'_> {
    type Output = io::Result<(TcpStream, SocketAddr)>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.as_ref().listener;
        match inner.accept() {
            Ok((stream, peer)) => Poll::Ready(Ok((TcpStream { inner: stream }, peer))),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                REACTOR.with_borrow_mut(|reactor| reactor.wake_readable(inner, cx));
                Poll::Pending
            }
            Err(other) => Poll::Ready(Err(other)),
        }
    }
}

#[derive(Debug)]
pub struct TcpStream {
    inner: sync::TcpStream,
}

impl TcpStream {
    pub fn connect<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        // this blocks right?
        let inner = sync::TcpStream::connect(addr)?;
        inner.set_nonblocking(true)?;
        Ok(Self { inner })
    }
}
