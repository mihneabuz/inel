use crate::helpers::setup_tracing;

#[cfg(feature = "hyper")]
mod hyper {
    use super::*;

    use std::{
        pin::Pin,
        rc::Rc,
        task::{Context, Poll},
    };

    use ::hyper::{
        body::{Body, Bytes, Frame, Incoming},
        client, rt, server,
        service::service_fn,
        Error, Request, Response, StatusCode,
    };
    use futures::{Stream, StreamExt};
    use inel::group::BufferShareGroup;

    #[test]
    fn simple() {
        run_server(|stream| inel::compat::hyper::HyperStream::new(stream));
    }

    #[test]
    fn fixed() {
        run_server(|stream| inel::compat::hyper::HyperStream::new_fixed(stream).unwrap());
    }

    #[test]
    fn shared() {
        let group = inel::block_on(BufferShareGroup::new()).unwrap();
        run_server(move |stream| {
            inel::compat::hyper::HyperStream::with_shared_buffers(stream, &group)
        });
    }

    fn run_server<F, S>(wrap: F)
    where
        F: Fn(inel::net::TcpStream) -> S + 'static,
        S: rt::Read + rt::Write + Unpin + 'static,
    {
        setup_tracing();

        let wrap = Rc::new(wrap);
        let connections = 10;

        inel::block_on(async move {
            let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();

            let port = listener.local_addr().unwrap().port();

            let wrap1 = Rc::clone(&wrap);
            inel::spawn(async move {
                for _ in 0..connections {
                    let (stream, _) = listener.accept().await.unwrap();
                    let hyper = wrap1(stream);

                    let res = server::conn::http1::Builder::new()
                        .serve_connection(hyper, service_fn(echo))
                        .await;

                    assert!(res.is_ok());
                }
            });

            for _ in 0..connections {
                let wrap2 = Rc::clone(&wrap);
                inel::spawn(async move {
                    let stream = inel::net::TcpStream::connect(("127.0.0.1", port))
                        .await
                        .unwrap();
                    let hyper = wrap2(stream);
                    let (mut sender, conn) = client::conn::http1::handshake(hyper).await.unwrap();

                    inel::spawn(async move {
                        assert!(conn.await.is_ok());
                    });

                    let req = Request::builder()
                        .uri("127.0.0.1")
                        .body("hello world!".repeat(10000))
                        .unwrap();

                    let res = sender.send_request(req).await;
                    assert!(res.is_ok());

                    let body = res.unwrap().into_body();
                    let mut stream = FrameStream::new(body);
                    while let Some(frame) = stream.next().await {
                        assert!(frame.is_ok());
                    }
                });
            }
        });

        assert!(inel::is_done());
    }

    async fn echo(req: Request<Incoming>) -> Result<Response<Incoming>, Error> {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(req.into_body())
            .unwrap())
    }

    pin_project_lite::pin_project! {
        struct FrameStream {
            #[pin]
            body: Incoming
        }
    }

    impl FrameStream {
        pub fn new(body: Incoming) -> Self {
            Self { body }
        }
    }

    impl Stream for FrameStream {
        type Item = Result<Frame<Bytes>, Error>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.project().body.poll_frame(cx)
        }
    }
}
