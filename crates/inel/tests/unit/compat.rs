use crate::helpers::setup_tracing;

async fn pair() -> (inel::net::TcpListener, inel::net::TcpStream) {
    let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();

    let port = listener.local_addr().unwrap().port();

    let stream = inel::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();

    (listener, stream)
}

#[cfg(feature = "hyper")]
mod hyper {
    use super::*;

    use ::hyper::{
        body::{Bytes, Incoming},
        client, rt, server,
        service::service_fn,
        Error, Request, Response,
    };
    use http_body_util::{combinators::BoxBody, BodyExt, Full};
    use std::{convert::Infallible, rc::Rc};

    #[test]
    fn simple() {
        run_server(|stream| inel::compat::hyper::HyperStream::new(stream));
    }

    #[test]
    fn fixed() {
        run_server(|stream| inel::compat::hyper::HyperStream::new_fixed(stream).unwrap());
    }

    fn run_server<F, S>(wrap: F)
    where
        F: Fn(inel::net::TcpStream) -> S + 'static,
        S: rt::Read + rt::Write + Unpin + 'static,
    {
        setup_tracing();

        let wrap1 = Rc::new(wrap);
        let wrap2 = Rc::clone(&wrap1);

        inel::block_on(async {
            let (listener, stream) = pair().await;

            inel::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let hyper = wrap1(stream);

                let res = server::conn::http1::Builder::new()
                    .serve_connection(hyper, service_fn(echo))
                    .await;

                assert!(res.is_ok());
            });

            inel::spawn(async move {
                let hyper = wrap2(stream);
                let (mut sender, conn) =
                    hyper::client::conn::http1::handshake(hyper).await.unwrap();

                inel::spawn(async move {
                    assert!(conn.await.is_ok());
                });

                let req = Request::builder()
                    .uri("127.0.0.1")
                    .body(Full::new(Bytes::from("Lorem Ipsum".repeat(100))).boxed())
                    .unwrap();

                let res = sender.send_request(req).await;
                assert!(res.is_ok());

                let mut body = res.unwrap().into_body();
                while let Some(frame) = body.frame().await {
                    assert!(frame.is_ok());
                }
            });
        });
    }

    async fn echo(req: Request<Incoming>) -> Result<Response<BoxBody<Bytes, Error>>, Infallible> {
        Ok(Response::new(req.into_body().boxed()))
    }
}
