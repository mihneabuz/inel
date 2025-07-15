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

    #[test]
    fn simple() {
        run_server(|stream| inel::compat::hyper::HyperStream::new_buffered(stream));
    }

    #[test]
    fn fixed() {
        run_server(|stream| inel::compat::hyper::HyperStream::new_buffered_fixed(stream).unwrap());
    }

    #[test]
    fn shared() {
        let group = inel::block_on(inel::group::BufferShareGroup::new()).unwrap();
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
                let mut incoming = listener.incoming();
                for _ in 0..connections {
                    let stream = incoming.next().await.unwrap().unwrap();
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
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            std::mem::drop(listener);

            inel::time::Instant::new().await;

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
