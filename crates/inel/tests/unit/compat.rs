use crate::helpers::setup_tracing;

pub async fn find_open_port() -> u16 {
    let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();

    let port = listener.local_addr().unwrap().port();

    std::mem::drop(listener);

    inel::time::Instant::new().await;

    port
}

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
        service::service_fn,
        Error, Request, Response, StatusCode,
    };
    use futures::{AsyncRead, AsyncWrite, Stream, StreamExt};

    #[test]
    fn simple() {
        run_server_http1(|stream| inel::compat::stream::BufStream::new(stream));
        run_server_http2(|stream| inel::compat::stream::BufStream::new(stream));
        assert!(inel::is_done());
    }

    #[test]
    fn fixed() {
        run_server_http1(|stream| inel::compat::stream::FixedBufStream::new(stream).unwrap());
        run_server_http2(|stream| inel::compat::stream::FixedBufStream::new(stream).unwrap());
        assert!(inel::is_done());
    }

    #[test]
    fn shared() {
        let group = inel::block_on(
            inel::group::BufferShareGroup::options()
                .buffer_capacity(512)
                .initial_read_buffers(4)
                .initial_write_buffers(0)
                .build(),
        )
        .unwrap();

        let group_clone = group.clone();

        run_server_http1(move |stream| inel::compat::stream::ShareBufStream::new(stream, &group));
        run_server_http2(move |stream| {
            inel::compat::stream::ShareBufStream::new(stream, &group_clone)
        });

        assert!(inel::is_done());
    }

    fn run_server_http1<F, S>(wrap: F)
    where
        F: Fn(inel::net::TcpStream) -> S + 'static,
        S: AsyncRead + AsyncWrite + Unpin + 'static,
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
                let mut incoming = listener.incoming();
                for _ in 0..connections {
                    let stream = wrap1(incoming.next().await.unwrap().unwrap());
                    let res = inel::compat::hyper::serve_http1(stream, service_fn(echo)).await;
                    assert!(res.is_ok());
                }
            });

            for _ in 0..connections {
                let wrap2 = Rc::clone(&wrap);
                inel::spawn(async move {
                    let stream = wrap2(
                        inel::net::TcpStream::connect(("127.0.0.1", port))
                            .await
                            .unwrap(),
                    );

                    let mut client = inel::compat::hyper::HyperClient::handshake_http1(stream)
                        .await
                        .unwrap();

                    let req = Request::builder()
                        .uri("/")
                        .method("POST")
                        .body("hello world!".repeat(1000))
                        .unwrap();

                    let res = client.send_request(req).await.unwrap();
                    let body = res.into_body();
                    let mut stream = FrameStream::new(body);
                    while let Some(frame) = stream.next().await {
                        assert!(frame.is_ok());
                    }
                });
            }
        });
    }

    fn run_server_http2<F, S>(wrap: F)
    where
        F: Fn(inel::net::TcpStream) -> S + 'static,
        S: AsyncRead + AsyncWrite + Unpin + 'static,
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
                let mut incoming = listener.incoming();
                for _ in 0..connections {
                    let stream = wrap1(incoming.next().await.unwrap().unwrap());
                    let res = inel::compat::hyper::serve_http2(stream, service_fn(echo)).await;
                    assert!(res.is_ok());
                }
            });

            for _ in 0..connections {
                let wrap2 = Rc::clone(&wrap);
                inel::spawn(async move {
                    let stream = wrap2(
                        inel::net::TcpStream::connect(("127.0.0.1", port))
                            .await
                            .unwrap(),
                    );

                    let mut client = inel::compat::hyper::HyperClient::handshake_http2(stream)
                        .await
                        .unwrap();

                    let req = Request::builder()
                        .uri("/")
                        .method("POST")
                        .body("hello world!".repeat(1000))
                        .unwrap();

                    let res = client.send_request(req).await.unwrap();
                    let body = res.into_body();
                    let mut stream = FrameStream::new(body);
                    while let Some(frame) = stream.next().await {
                        assert!(frame.is_ok());
                    }
                });
            }
        });
    }

    async fn echo(req: Request<Incoming>) -> Result<Response<Incoming>, Error> {
        Ok(Response::builder()
            .status(StatusCode::OK)
            .body(req.into_body())
            .unwrap())
    }

    struct FrameStream {
        body: Incoming,
    }

    impl FrameStream {
        pub fn new(body: Incoming) -> Self {
            Self { body }
        }
    }

    impl Stream for FrameStream {
        type Item = Result<Frame<Bytes>, Error>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Pin::new(&mut unsafe { self.get_unchecked_mut() }.body).poll_frame(cx)
        }
    }
}

#[cfg(feature = "rustls")]
mod rustls {
    use super::*;

    use std::io::{BufReader, Cursor};

    use ::rustls::{pki_types::ServerName, ClientConfig, RootCertStore, ServerConfig};
    use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
    use rustls_pemfile::{certs, private_key};

    use inel::{
        compat::stream::{BufStream, FixedBufStream},
        net::{DirectTcpStream, TcpListener, TcpStream},
    };

    const KEY: &[u8] = include_bytes!("../certs/end.rsa");
    const CERT: &[u8] = include_bytes!("../certs/end.cert");
    const CHAIN: &[u8] = include_bytes!("../certs/end.chain");

    fn tls_configs() -> (ServerConfig, ClientConfig, ServerName<'static>) {
        let cert = certs(&mut BufReader::new(Cursor::new(CERT)))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let key = private_key(&mut BufReader::new(Cursor::new(KEY))).unwrap();
        let server = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert, key.unwrap())
            .unwrap();

        let mut client_root_cert_store = RootCertStore::empty();
        let mut chain = BufReader::new(Cursor::new(CHAIN));
        let certs = certs(&mut chain).collect::<Result<Vec<_>, _>>().unwrap();
        client_root_cert_store.add_parsable_certificates(certs);

        let client = rustls::ClientConfig::builder()
            .with_root_certificates(client_root_cert_store)
            .with_no_client_auth();

        let domain = ServerName::try_from("testserver.com").unwrap().to_owned();

        (server, client, domain)
    }

    fn simple_pair() -> (BufStream<TcpStream>, BufStream<TcpStream>) {
        inel::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();

            let port = listener.local_addr().unwrap().port();

            let server =
                inel::spawn(async move { BufStream::new(listener.accept().await.unwrap().0) });

            let client = inel::spawn(async move {
                BufStream::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap())
            });

            (server.join().await.unwrap(), client.join().await.unwrap())
        })
    }

    fn direct_pair() -> (
        FixedBufStream<DirectTcpStream>,
        FixedBufStream<DirectTcpStream>,
    ) {
        inel::block_on(async {
            let port = find_open_port().await;

            let listener = TcpListener::bind_direct(("127.0.0.1", port)).await.unwrap();

            let server = inel::spawn(async move {
                FixedBufStream::new(listener.accept().await.unwrap().0).unwrap()
            });

            let client = inel::spawn(async move {
                FixedBufStream::new(
                    TcpStream::connect_direct(("127.0.0.1", port))
                        .await
                        .unwrap(),
                )
                .unwrap()
            });

            (server.join().await.unwrap(), client.join().await.unwrap())
        })
    }

    fn run<T, F>(connections: usize, f: F)
    where
        T: AsyncRead + AsyncWrite + Unpin + 'static,
        F: Fn() -> (T, T),
    {
        setup_tracing();

        let (server, client, domain) = tls_configs();
        let acceptor = inel::compat::rustls::TlsAcceptor::from(server);
        let connector = inel::compat::rustls::TlsConnector::from(client);

        for _ in 0..connections {
            let acceptor = acceptor.clone();
            let connector = connector.clone();
            let domain = domain.clone();

            let (server, client) = f();

            inel::spawn(async move {
                let mut server = acceptor.accept(server).await.unwrap();

                let mut buffer = vec![0; CERT.len()];
                server.read_exact(&mut buffer).await.unwrap();
                assert_eq!(&buffer, CERT);

                server.write_all(CHAIN).await.unwrap();
                server.flush().await.unwrap();

                let mut buffer = vec![0; KEY.len()];
                server.read_exact(&mut buffer).await.unwrap();
                assert_eq!(&buffer, KEY);
            });

            inel::spawn(async move {
                let mut client = connector.connect(domain, client).await.unwrap();

                client.write_all(CERT).await.unwrap();
                client.flush().await.unwrap();

                let mut buffer = vec![0; CHAIN.len()];
                client.read_exact(&mut buffer).await.unwrap();
                assert_eq!(&buffer, CHAIN);

                client.write_all(KEY).await.unwrap();
                client.flush().await.unwrap();
            });
        }

        inel::run();
    }

    #[test]
    fn simple() {
        run(20, simple_pair);
    }

    #[test]
    fn direct() {
        run(20, direct_pair);
    }
}

#[cfg(feature = "axum")]
mod axum {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use axum::{routing::get, Router};
    use futures::{FutureExt, Stream, StreamExt};
    use hyper::{
        body::{Body, Bytes, Frame, Incoming},
        Error, Request,
    };
    use inel::compat::{axum::Serve, stream::BufStream};

    use crate::compat::find_open_port;

    #[test]
    fn default() {
        run_server(Serve::default());
    }

    #[test]
    fn direct() {
        run_server(Serve::builder().with_direct_descriptors());
    }

    #[test]
    fn fixed() {
        run_server(Serve::builder().with_fixed_buffers());
    }

    #[test]
    fn shared() {
        let group = inel::block_on(inel::group::BufferShareGroup::new()).unwrap();
        run_server(Serve::builder().with_shared_buffers(group));
    }

    pub fn run_server(options: Serve) {
        const MESSAGE: &str = "Hello World!";

        let app = Router::new().route("/hello", get(|| async { MESSAGE }));
        let port = inel::block_on(find_open_port());

        let (send, mut recv) = futures::channel::oneshot::channel();

        inel::spawn(async move {
            futures::select! {
                res = options.serve(("127.0.0.1", port), app).fuse() => res.unwrap(),
                res = recv => res.unwrap()
            };
        })
        .detach();

        inel::spawn(async move {
            inel::time::sleep(std::time::Duration::from_millis(10)).await;

            for _ in 0..10 {
                let stream = inel::net::TcpStream::connect_direct(("127.0.0.1", port))
                    .await
                    .unwrap();

                let stream = BufStream::new(stream);

                let mut client = inel::compat::hyper::HyperClient::handshake_http1(stream)
                    .await
                    .unwrap();

                let req = Request::builder()
                    .uri("/hello")
                    .body(axum::body::Body::empty())
                    .unwrap();

                let res = client.send_request(req).await.unwrap();

                let mut stream = FrameStream::new(res.into_body());
                let frame = stream.next().await.unwrap();
                let data = frame.unwrap().into_data().unwrap();
                assert_eq!(data, MESSAGE.as_bytes());
            }

            send.send(()).unwrap();
        });

        inel::run();
    }

    struct FrameStream {
        body: Incoming,
    }

    impl FrameStream {
        pub fn new(body: Incoming) -> Self {
            Self { body }
        }
    }

    impl Stream for FrameStream {
        type Item = Result<Frame<Bytes>, Error>;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Pin::new(&mut unsafe { self.get_unchecked_mut() }.body).poll_frame(cx)
        }
    }
}
