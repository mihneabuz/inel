use std::{
    future::Future,
    io::Result,
    pin::Pin,
    task::{ready, Context, Poll},
};

use futures::{AsyncRead, AsyncWrite};
use hyper::{
    body::{Body, Incoming},
    service::HttpService,
    Request, Response,
};

#[derive(Copy, Clone)]
pub struct Executor;

impl<F> hyper::rt::Executor<F> for Executor
where
    F: Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        crate::spawn(fut).detach();
    }
}

pub struct HyperStream<Stream>(Stream);

impl<S> HyperStream<S> {
    pub fn new(stream: S) -> Self {
        Self(stream)
    }
}

impl<S: Unpin> HyperStream<S> {
    fn project(self: Pin<&mut Self>) -> Pin<&mut S> {
        Pin::new(&mut self.get_mut().0)
    }
}

impl<S> hyper::rt::Read for HyperStream<S>
where
    S: AsyncRead + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut cursor: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<()>> {
        let buf: &mut [u8] = unsafe { std::mem::transmute(cursor.as_mut()) };
        let read = ready!(self.project().poll_read(cx, buf))?;
        unsafe {
            cursor.advance(read);
        }
        Poll::Ready(Ok(()))
    }
}

impl<S> hyper::rt::Write for HyperStream<S>
where
    S: AsyncWrite + Unpin,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        self.project().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.project().poll_close(cx)
    }
}

pub enum HyperClient<B> {
    Http1(hyper::client::conn::http1::SendRequest<B>),
    Http2(hyper::client::conn::http2::SendRequest<B>),
}

impl<B> HyperClient<B>
where
    B: Body + 'static,
    <B as Body>::Data: Send,
    <B as Body>::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    pub async fn handshake_http1<S>(stream: S) -> hyper::Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + 'static,
    {
        let hyper = HyperStream::new(stream);

        let (sender, connection) = hyper::client::conn::http1::Builder::new()
            .handshake::<_, B>(hyper)
            .await?;

        crate::spawn(connection);

        Ok(Self::Http1(sender))
    }

    pub async fn handshake_http2<S>(stream: S) -> hyper::Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + 'static,
        B: Unpin,
    {
        let hyper = HyperStream::new(stream);

        let (sender, connection) = hyper::client::conn::http2::Builder::new(Executor)
            .handshake::<_, B>(hyper)
            .await?;

        crate::spawn(connection);

        Ok(Self::Http2(sender))
    }

    pub async fn send_request(&mut self, req: Request<B>) -> hyper::Result<Response<Incoming>> {
        match self {
            HyperClient::Http1(sender) => sender.send_request(req).await,
            HyperClient::Http2(sender) => sender.send_request(req).await,
        }
    }

    pub fn is_ready(&mut self) -> bool {
        match self {
            HyperClient::Http1(sender) => sender.is_ready(),
            HyperClient::Http2(sender) => sender.is_ready(),
        }
    }
}

pub async fn serve_http1<S, A, B, E>(stream: S, app: A) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    A: HttpService<Incoming, ResBody = B, Error = E>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
    B: Body + 'static,
    <B as Body>::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let hyper = HyperStream::new(stream);
    hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper, app)
        .await
}

pub async fn serve_http2<S, A, B, E, F>(stream: S, app: A) -> hyper::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    A: HttpService<Incoming, ResBody = B, Error = E, Future = F>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
    F: Future<Output = std::result::Result<Response<B>, E>> + Send + 'static,
    B: Body + Send + 'static,
    <B as Body>::Data: Send,
    <B as Body>::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let hyper = HyperStream::new(stream);
    hyper::server::conn::http2::Builder::new(Executor)
        .serve_connection(hyper, app)
        .await
}
