use std::{io::Result, net::ToSocketAddrs};

use axum::Router;
use futures::{AsyncRead, AsyncWrite, Stream, StreamExt};
use tower::Service;

use crate::{
    compat,
    group::BufferShareGroup,
    io::{ReadSource, WriteSource},
    net::TcpListener,
};

pub async fn serve<A>(addr: A, app: Router) -> Result<()>
where
    A: ToSocketAddrs,
{
    Serve::builder().serve(addr, app).await
}

#[cfg(feature = "rustls")]
pub async fn serve_tls<A>(addr: A, config: rustls::ServerConfig, app: Router) -> Result<()>
where
    A: ToSocketAddrs,
{
    Serve::builder().with_tls(config).serve(addr, app).await
}

#[derive(Clone)]
enum Descriptors {
    Raw,
    Direct,
}

#[derive(Clone)]
enum Buffering {
    Simple,
    Fixed,
    Group(BufferShareGroup),
}

#[derive(Clone)]
enum Tls {
    None,
    #[cfg(feature = "rustls")]
    Rustls(compat::rustls::TlsAcceptor),
}

#[derive(Clone)]
enum HttpProto {
    Http1,
    Http2,
}

pub struct Serve {
    descriptors: Descriptors,
    buffering: Buffering,
    security: Tls,
    http: HttpProto,
}

impl Default for Serve {
    fn default() -> Self {
        Self {
            descriptors: Descriptors::Raw,
            buffering: Buffering::Simple,
            security: Tls::None,
            http: HttpProto::Http1,
        }
    }
}

impl Serve {
    pub fn builder() -> Self {
        Self::default()
    }

    pub fn with_direct_descriptors(mut self) -> Self {
        self.descriptors = Descriptors::Direct;
        self
    }

    pub fn with_fixed_buffers(mut self) -> Self {
        self.buffering = Buffering::Fixed;
        self
    }

    pub fn with_shared_buffers(mut self, group: BufferShareGroup) -> Self {
        self.buffering = Buffering::Group(group);
        self
    }

    #[cfg(feature = "rustls")]
    pub fn with_tls(mut self, config: rustls::ServerConfig) -> Self {
        self.security = Tls::Rustls(compat::rustls::TlsAcceptor::from(config));
        self
    }

    pub fn with_http2(mut self) -> Self {
        self.http = HttpProto::Http2;
        self
    }

    pub async fn serve<A>(self, addr: A, app: Router) -> Result<()>
    where
        A: ToSocketAddrs,
    {
        match self.descriptors {
            Descriptors::Raw => {
                let listener = TcpListener::bind(addr).await?;
                self.with_incoming(listener.incoming(), app).await;
            }
            Descriptors::Direct => {
                let listener = TcpListener::bind_direct(addr).await?;
                self.with_incoming(listener.incoming(), app).await;
            }
        }

        Ok(())
    }

    async fn with_incoming<S, I>(self, incoming: I, app: Router)
    where
        I: Stream<Item = Result<S>>,
        S: ReadSource + WriteSource + Unpin + 'static,
    {
        incoming
            .filter_map(async |stream| stream.ok())
            .for_each(async |stream| {
                let app = app.clone();

                match &self.buffering {
                    Buffering::Simple => {
                        let stream = compat::stream::BufStream::new(stream);
                        self.with_raw_stream(stream, app).await;
                    }

                    Buffering::Fixed => {
                        if let Ok(stream) = compat::stream::FixedBufStream::new(stream) {
                            self.with_raw_stream(stream, app).await;
                        }
                    }

                    Buffering::Group(share) => {
                        let stream = compat::stream::ShareBufStream::new(stream, share);
                        self.with_raw_stream(stream, app).await;
                    }
                }
            })
            .await;
    }

    async fn with_raw_stream<S>(&self, stream: S, app: Router)
    where
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        match &self.security {
            Tls::None => self.with_stream(stream, app).await,
            #[cfg(feature = "rustls")]
            Tls::Rustls(acceptor) => {
                if let Ok(tls_stream) = acceptor.accept(stream).await {
                    self.with_stream(tls_stream, app).await;
                }
            }
        }
    }

    async fn with_stream<S>(&self, stream: S, app: Router)
    where
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        match self.http {
            HttpProto::Http1 => crate::spawn(handle_http1(stream, app)).detach(),
            HttpProto::Http2 => crate::spawn(handle_http2(stream, app)).detach(),
        }
    }
}

async fn handle_http1<S>(stream: S, app: Router)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let service = hyper::service::service_fn(|request| app.clone().call(request));
    if let Err(err) = compat::hyper::serve_http1(stream, service).await {
        tracing::error!(?err);
    }
}

async fn handle_http2<S>(stream: S, app: Router)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let service = hyper::service::service_fn(|request| app.clone().call(request));
    if let Err(err) = compat::hyper::serve_http2(stream, service).await {
        tracing::error!(?err);
    }
}
