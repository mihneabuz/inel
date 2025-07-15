use std::{
    io::Result,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite};
use rustls::{pki_types::ServerName, ClientConfig, ServerConfig};

#[derive(Clone)]
pub struct TlsAcceptor(futures_rustls::TlsAcceptor);

impl From<Arc<ServerConfig>> for TlsAcceptor {
    fn from(config: Arc<ServerConfig>) -> Self {
        Self(futures_rustls::TlsAcceptor::from(config))
    }
}

impl From<ServerConfig> for TlsAcceptor {
    fn from(config: ServerConfig) -> Self {
        Self::from(Arc::new(config))
    }
}

impl TlsAcceptor {
    pub async fn accept<S>(&self, stream: S) -> Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.0
            .accept(stream)
            .await
            .map(futures_rustls::TlsStream::Server)
            .map(TlsStream)
    }
}

#[derive(Clone)]
pub struct TlsConnector(futures_rustls::TlsConnector);

impl From<Arc<ClientConfig>> for TlsConnector {
    fn from(config: Arc<ClientConfig>) -> Self {
        Self(futures_rustls::TlsConnector::from(config))
    }
}

impl From<ClientConfig> for TlsConnector {
    fn from(config: ClientConfig) -> Self {
        Self::from(Arc::new(config))
    }
}

impl TlsConnector {
    pub async fn connect<S>(&self, domain: ServerName<'static>, stream: S) -> Result<TlsStream<S>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.0
            .connect(domain, stream)
            .await
            .map(futures_rustls::TlsStream::Client)
            .map(TlsStream)
    }
}

pub struct TlsStream<S>(futures_rustls::TlsStream<S>);

impl<S> TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn pinned_inner(self: Pin<&mut Self>) -> Pin<&mut futures_rustls::TlsStream<S>> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl<S> AsyncRead for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize>> {
        self.pinned_inner().poll_read(cx, buf)
    }
}

impl<S> AsyncWrite for TlsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.pinned_inner().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_inner().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.pinned_inner().poll_close(cx)
    }
}
