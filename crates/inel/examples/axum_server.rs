use inel::io::AsyncWriteOwned;

use axum::{
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use hyper::{body::Incoming, server::conn::http1, Request, StatusCode};
use tower::Service;

fn main() {
    let app = Router::new()
        .route("/hello", get(|| async { "Hello from axum + inel!" }))
        .route("/echo", post(|body: String| async { body }))
        .route(
            "/reverse",
            post(|body: String| async move { body.chars().rev().collect::<String>() }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Leave me alone!") });

    inel::block_on(async { run(app).await.unwrap() })
}

async fn run(app: Router) -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = inel::net::TcpListener::bind(addr).await?;

    print("Started server\n".to_string()).await;

    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        print("Received connection\n".to_string()).await;

        let tower_service = app.clone();

        inel::spawn(async move {
            let hyper = stream::HyperStream::new(stream).unwrap();

            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                tower_service.clone().call(request)
            });

            let res = http1::Builder::new()
                .serve_connection(hyper, hyper_service)
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
