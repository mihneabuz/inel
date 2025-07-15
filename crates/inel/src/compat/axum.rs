use std::{io::Result, net::ToSocketAddrs};

use axum::Router;
use futures::{AsyncRead, AsyncWrite, Stream, StreamExt};
use rustls::ServerConfig;
use tower::Service;

use crate::{
    compat::{
        self,
        hyper::HyperStream,
        rustls::TlsAcceptor,
        stream::{BufStream, FixedBufStream},
    },
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
    Rustls(TlsAcceptor),
}

#[derive(Clone)]
enum HttpProto {
    HTTP1,
    HTTP2,
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
            http: HttpProto::HTTP1,
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

    pub fn with_tls(mut self, config: ServerConfig) -> Self {
        self.security = Tls::Rustls(TlsAcceptor::from(config));
        self
    }

    pub fn with_http2(mut self) -> Self {
        self.http = HttpProto::HTTP2;
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
                        let stream = BufStream::new(stream);
                        self.with_raw_stream(stream, app).await;
                    }

                    Buffering::Fixed => {
                        if let Ok(stream) = FixedBufStream::new(stream) {
                            self.with_raw_stream(stream, app).await;
                        }
                    }

                    Buffering::Group(share) => {
                        let stream = share.supply_to(stream);
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
            HttpProto::HTTP1 => crate::spawn(handle_http1(stream, app)).detach(),
            HttpProto::HTTP2 => crate::spawn(handle_http2(stream, app)).detach(),
        }
    }
}

async fn handle_http1<S>(stream: S, app: Router) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hyper = HyperStream::new(stream);
    let service = hyper::service::service_fn(|request| app.clone().call(request));

    hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper, service)
        .await
        .map_err(std::io::Error::other)
}

async fn handle_http2<S>(stream: S, app: Router) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hyper = HyperStream::new(stream);
    let service = hyper::service::service_fn(|request| app.clone().call(request));

    hyper::server::conn::http2::Builder::new(compat::hyper::Executor)
        .serve_connection(hyper, service)
        .await
        .map_err(std::io::Error::other)
}
