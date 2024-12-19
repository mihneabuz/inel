use std::convert::Infallible;

use futures::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use hyper::{body::Bytes, server::conn::http1, service::service_fn, Request, Response, StatusCode};
use inel::io::AsyncWriteOwned;

fn main() {
    inel::block_on(async { run().await.unwrap() })
}

async fn run() -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = inel::net::TcpListener::bind(addr).await?;

    print("Started server\n".to_string()).await;

    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        print("Received connection\n".to_string()).await;

        inel::spawn(async move {
            let hyper = stream::HyperStream::new(stream).unwrap();

            let res = http1::Builder::new()
                .serve_connection(hyper, service_fn(service))
                .await;

            if let Err(e) = res {
                print(format!("Error: {:?}", e)).await;
            }
        });
    }

    Ok(())
}

async fn print(message: String) {
    let _ = inel::io::stdout().write_owned(message).await;
}

async fn service(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible> {
    match req.uri().path() {
        "/" => Ok(Response::new(req.into_body().boxed())),
        "/hello" => Ok(Response::new(
            Full::new(Bytes::from("Hello from hyper + inel!\n"))
                .map_err(|never| match never {})
                .boxed(),
        )),
        _ => {
            let mut not_found = Response::new(
                Empty::<Bytes>::new()
                    .map_err(|never| match never {})
                    .boxed(),
            );
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

mod stream {
    use std::{
        pin::Pin,
        task::{ready, Context, Poll},
    };

    use futures::{AsyncBufRead, AsyncWrite};

    use hyper::rt;

    pin_project_lite::pin_project! {
        pub struct HyperStream {
            #[pin]
            reader: inel::io::FixedBufReader<inel::io::ReadHandle<inel::net::TcpStream>>,

            #[pin]
            writer: inel::io::FixedBufWriter<inel::io::WriteHandle<inel::net::TcpStream>>,
        }
    }

    impl HyperStream {
        pub fn new(stream: inel::net::TcpStream) -> std::io::Result<Self> {
            let (reader, writer) = stream.split_buffered();
            let (reader, writer) = (reader.fix()?, writer.fix()?);
            Ok(Self { reader, writer })
        }
    }

    impl rt::Read for HyperStream {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            mut cursor: rt::ReadBufCursor<'_>,
        ) -> Poll<std::io::Result<()>> {
            let mut this = self.project();
            let buf = ready!(this.reader.as_mut().poll_fill_buf(cx))?;
            let size = cursor.remaining().min(buf.len());
            cursor.put_slice(&buf[..size]);
            this.reader.consume(size);
            Poll::Ready(Ok(()))
        }
    }

    impl rt::Write for HyperStream {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.project().writer.poll_write(cx, buf)
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.project().writer.poll_flush(cx)
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.project().writer.poll_close(cx)
        }
    }
}
