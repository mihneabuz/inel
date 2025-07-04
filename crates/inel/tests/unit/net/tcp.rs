use std::{
    io::{Read, Write},
    os::fd::{FromRawFd, IntoRawFd},
    thread::JoinHandle,
};

use futures::{select, AsyncBufReadExt, AsyncWriteExt, FutureExt, SinkExt, StreamExt};
use inel::{
    buffer::StableBufferExt,
    io::{AsyncReadOwned, AsyncWriteOwned, Split},
};
use inel_macro::test_repeat;

use crate::helpers::setup_tracing;

#[test]
#[test_repeat(10)]
fn listen() {
    setup_tracing();

    let handle = inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();

        let port = listener.local_addr().unwrap().port();
        assert!(port > 0);

        let handle = std::thread::spawn(move || {
            let stream = std::net::TcpStream::connect(("127.0.0.1", port));
            assert!(stream.is_ok());
        });

        let conn = listener.accept().await;
        assert!(conn.is_ok());

        handle
    });

    assert!(handle.join().is_ok());
}

#[test]
#[test_repeat(10)]
fn connect() {
    setup_tracing();

    let (port_send, port_recv) = std::sync::mpsc::channel::<u16>();

    let handle = std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        port_send
            .send(listener.local_addr().unwrap().port())
            .unwrap();

        assert!(listener.accept().is_ok());
    });

    let port = port_recv.recv().unwrap();
    inel::block_on(async move {
        let stream = inel::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        assert!(stream.local_addr().is_ok());
        assert!(stream.peer_addr().is_ok());
        assert!(stream.shutdown(std::net::Shutdown::Both).await.is_ok());
    });

    assert!(handle.join().is_ok());
}

fn echo_server() -> (u16, JoinHandle<()>) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let mut conn = listener.accept().unwrap().0;
        let mut buf = Box::new([0; 4096]);
        while let Ok(read) = conn.read(buf.as_mut_slice()) {
            conn.write(&buf[0..read]).unwrap();

            if read == 0 {
                break;
            }
        }
    });

    (port, handle)
}

fn echo_client(port: u16) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();

        let mut buf = Box::new([0; 512]);
        for _ in 0..100 {
            client.write("Hello World!\n".as_bytes()).unwrap();
            let read = client.read(buf.as_mut_slice()).unwrap();
            assert_eq!(&buf[0..read], "Hello World!\n".as_bytes());
        }
    })
}

#[test]
#[test_repeat(10)]
fn client() {
    setup_tracing();

    let (port, handle) = echo_server();

    inel::block_on(async move {
        let mut client = inel::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        assert_eq!(format!("{:?}", client), "TcpStream".to_string());

        let mut buf = Box::new([0; 512]);
        let mut read = Ok(0);
        for _ in 0..100 {
            let _ = client.write_owned("Hello World!\n".to_string()).await;
            (buf, read) = client.read_owned(buf).await;
            assert_eq!(&buf[0..read.unwrap()], "Hello World!\n".as_bytes());
        }

        client.shutdown(std::net::Shutdown::Both).await.unwrap();
    });

    handle.join().unwrap();
}

#[test]
#[test_repeat(10)]
fn server() {
    setup_tracing();

    let (port, listener) = inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        assert_eq!(format!("{:?}", listener), "TcpListener".to_string());

        (port, listener)
    });

    let handle = echo_client(port);

    inel::block_on(async move {
        let mut conn = listener.accept().await.unwrap().0;

        let mut fut = conn.read_owned(Box::new([0; 2048]));
        loop {
            let (buf, res) = fut.await;
            let read = res.unwrap();
            if read == 0 {
                break;
            }

            let (buf, res) = conn.write_owned(buf.view(0..read)).await;
            assert!(res.is_ok_and(|wrote| wrote == read));

            fut = conn.read_owned(buf.unview());
        }

        conn.shutdown(std::net::Shutdown::Both).await.unwrap();
    });

    handle.join().unwrap();
}

#[test]
fn error() {
    setup_tracing();

    inel::block_on(async {
        let res = inel::net::TcpStream::connect(("127.??.0.1", 123)).await;
        assert!(res.is_err());

        let empty: Vec<std::net::SocketAddr> = Vec::new();
        let res = inel::net::TcpStream::connect(empty.as_slice()).await;
        assert!(res.is_err());

        let res = inel::net::TcpStream::connect(("127.0.0.1", 100)).await;
        assert!(res.is_err());

        let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        let res = inel::net::TcpListener::bind(("127.0.0.1", port)).await;
        assert!(res.is_err());
    });
}

#[test]
fn raw_fd() {
    setup_tracing();

    let (client, listener) = inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        let client = inel::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();

        (client.into_raw_fd(), listener.into_raw_fd())
    });

    inel::spawn(async move {
        let mut client = unsafe { inel::net::TcpStream::from_raw_fd(client) };
        let _ = client.write_owned("Hello World!\n".to_string()).await;
    });

    inel::spawn(async move {
        let listener = unsafe { inel::net::TcpListener::from_raw_fd(listener) };
        let mut conn = listener.accept().await.unwrap().0;
        let (buf, res) = conn.read_owned(Box::new([0; 512])).await;
        assert_eq!(&buf[0..res.unwrap()], "Hello World!\n".as_bytes());
    });

    inel::run();
}

#[test]
fn read_write() {
    setup_tracing();

    inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("::1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        inel::spawn(async move {
            for _ in 0..10 {
                let client = inel::net::TcpStream::connect(("::1", port)).await.unwrap();
                let mut writer = inel::io::BufWriter::new(client);
                let data = "Hello World!\n".repeat(4096).as_bytes().to_vec();
                assert!(writer.write_all(&data).await.is_ok());
                assert!(writer.flush().await.is_ok());
                let client = writer.into_inner();
                assert!(client.shutdown(std::net::Shutdown::Both).await.is_ok());
            }
        });

        inel::spawn(async move {
            let mut incoming = listener.incoming();
            for _ in 0..10 {
                let conn = incoming.next().await.unwrap().unwrap();
                let mut lines = inel::io::BufReader::new(conn).lines();
                while let Some(line) = lines.next().await {
                    assert!(line.is_ok());
                    assert_eq!(line.unwrap().as_str(), "Hello World!");
                }
            }
        });
    });

    assert!(inel::is_done())
}

#[test]
fn full() {
    setup_tracing();

    let clients = 20;

    inel::block_on(async move {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();

        let (tx, rx) = futures::channel::mpsc::unbounded();
        let run_server = async move {
            let mut incoming = listener.incoming();
            while let Some(Ok(stream)) = incoming.next().await {
                tracing::info!("server accepted");
                let mut completed = tx.clone();

                inel::spawn(async move {
                    let (reader, mut writer) = stream.split_buffered();
                    let reader = reader.fix().unwrap();
                    let mut lines = reader.lines();

                    while let Some(Ok(line)) = lines.next().await {
                        if &line == "exit" {
                            break;
                        }

                        let data = line + "\n";
                        assert!(writer.write_all(data.as_bytes()).await.is_ok());
                        assert!(writer.flush().await.is_ok());
                    }

                    tracing::info!("server completed");
                    assert!(completed.send(()).await.is_ok());
                });
            }
        }
        .fuse();

        let finish = async move {
            let _ = rx.take(clients).for_each(|_| async {}).await;
            tracing::info!("all done");
        }
        .fuse();

        let server = inel::spawn(async move {
            let mut run = Box::pin(run_server);
            let mut done = Box::pin(finish);
            loop {
                select! {
                    _ = run => {},
                    _ = done => {
                        break;
                    }
                };
            }
        });

        for client in 0..clients {
            inel::spawn(async move {
                tracing::info!(client, "starting");

                let stream = inel::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .unwrap();

                tracing::info!(client, "connected");

                let (mut reader, mut writer) = stream.split();

                for i in 1..=20 {
                    let old = "Hello World!".repeat(i) + "\n";
                    let (old, res) = writer.write_owned(old).await;
                    assert!(res.is_ok_and(|wrote| wrote == old.len()));

                    let (new, res) = reader.read_owned(Box::new([0; 4096])).await;
                    assert!(res.as_ref().is_ok_and(|read| *read == old.len()));
                    assert_eq!(old.as_bytes(), &new.as_slice()[0..res.unwrap()]);
                }

                let _ = writer.write_owned("exit".to_string()).await;
                let _ = writer.shutdown(std::net::Shutdown::Both).await;

                tracing::info!(client, "done");
            });
        }

        let _ = server.join().await;
    });
}

mod direct {
    use super::*;

    fn find_open_port() -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    #[test_repeat(10)]
    fn listen() {
        setup_tracing();

        let handle = inel::block_on(async {
            let port = find_open_port();

            let listener = inel::net::TcpListener::bind_direct(("127.0.0.1", port))
                .await
                .unwrap();

            let handle = std::thread::spawn(move || {
                let stream = std::net::TcpStream::connect(("127.0.0.1", port));
                assert!(stream.is_ok());
            });

            let conn = listener.accept().await;
            assert!(conn.is_ok());

            handle
        });

        assert!(handle.join().is_ok());
        assert!(inel::is_done());
    }

    #[test]
    #[test_repeat(10)]
    fn connect() {
        setup_tracing();

        let (port_send, port_recv) = std::sync::mpsc::channel::<u16>();

        let handle = std::thread::spawn(move || {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            port_send
                .send(listener.local_addr().unwrap().port())
                .unwrap();

            assert!(listener.accept().is_ok());
        });

        let port = port_recv.recv().unwrap();
        inel::block_on(async move {
            let stream = inel::net::TcpStream::connect_direct(("127.0.0.1", port))
                .await
                .unwrap();
            assert!(stream.shutdown(std::net::Shutdown::Both).await.is_ok());
        });

        assert!(handle.join().is_ok());
        assert!(inel::is_done());
    }

    #[test]
    #[test_repeat(10)]
    fn client() {
        setup_tracing();

        let (port, handle) = echo_server();

        inel::block_on(async move {
            let mut client = inel::net::TcpStream::connect_direct(("127.0.0.1", port))
                .await
                .unwrap();

            assert_eq!(format!("{:?}", client), "DirectTcpStream".to_string());

            let mut buf = Box::new([0; 512]);
            let mut read = Ok(0);
            for _ in 0..100 {
                let _ = client.write_owned("Hello World!\n".to_string()).await;
                (buf, read) = client.read_owned(buf).await;
                assert_eq!(&buf[0..read.unwrap()], "Hello World!\n".as_bytes());
            }

            client.shutdown(std::net::Shutdown::Both).await.unwrap();
        });

        handle.join().unwrap();
        assert!(inel::is_done());
    }

    #[test]
    #[test_repeat(10)]
    fn server() {
        setup_tracing();

        let port = find_open_port();

        let listener = inel::block_on(async move {
            let listener = inel::net::TcpListener::bind_direct(("127.0.0.1", port))
                .await
                .unwrap();

            assert_eq!(format!("{:?}", listener), "DirectTcpListener".to_string());

            listener
        });

        let handle = echo_client(port);

        inel::block_on(async move {
            let mut conn = listener.accept().await.unwrap().0;

            let mut fut = conn.read_owned(Box::new([0; 2048]));
            loop {
                let (buf, res) = fut.await;
                let read = res.unwrap();
                if read == 0 {
                    break;
                }

                let (buf, res) = conn.write_owned(buf.view(0..read)).await;
                assert!(res.is_ok_and(|wrote| wrote == read));

                fut = conn.read_owned(buf.unview());
            }

            conn.shutdown(std::net::Shutdown::Both).await.unwrap();
        });

        handle.join().unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn error() {
        setup_tracing();

        inel::block_on(async {
            let res = inel::net::TcpStream::connect_direct(("127.??.0.1", 123)).await;
            assert!(res.is_err());

            let empty: Vec<std::net::SocketAddr> = Vec::new();
            let res = inel::net::TcpStream::connect_direct(empty.as_slice()).await;
            assert!(res.is_err());

            let res = inel::net::TcpStream::connect_direct(("127.0.0.1", 100)).await;
            assert!(res.is_err());

            let port = find_open_port();
            inel::net::TcpListener::bind_direct(("127.0.0.1", port))
                .await
                .unwrap();

            let res = inel::net::TcpListener::bind_direct(("127.0.0.1", port)).await;
            assert!(res.is_err());
        });

        assert!(inel::is_done());
    }

    #[test]
    fn read_write() {
        setup_tracing();

        let clients = 20;
        let port = find_open_port();

        inel::block_on(async move {
            let listener = inel::net::TcpListener::bind_direct(("::1", port))
                .await
                .unwrap();

            inel::spawn(async move {
                for _ in 0..clients {
                    let client = inel::net::TcpStream::connect_direct(("::1", port))
                        .await
                        .unwrap();
                    let mut writer = inel::io::BufWriter::new(client);
                    let data = "Hello World!\n".repeat(4096).as_bytes().to_vec();
                    assert!(writer.write_all(&data).await.is_ok());
                    assert!(writer.flush().await.is_ok());
                    let client = writer.into_inner();
                    assert!(client.shutdown(std::net::Shutdown::Both).await.is_ok());
                }
            });

            inel::spawn(async move {
                for _ in 0..clients {
                    let conn = listener.accept().await.unwrap().0;
                    let mut lines = inel::io::BufReader::new(conn).lines();
                    while let Some(line) = lines.next().await {
                        assert!(line.is_ok());
                        assert_eq!(line.unwrap().as_str(), "Hello World!");
                    }
                }
            });
        });

        assert!(inel::is_done());
    }

    #[test]
    fn full() {
        setup_tracing();

        let clients = 12;
        let port = find_open_port();

        inel::block_on(async move {
            let listener = inel::net::TcpListener::bind_direct(("127.0.0.1", port))
                .await
                .unwrap();

            let (tx, rx) = futures::channel::mpsc::unbounded();
            let run_server = async move {
                let mut incoming = listener.incoming();
                while let Some(Ok(stream)) = incoming.next().await {
                    tracing::info!("server accepted");
                    let mut completed = tx.clone();

                    inel::spawn(async move {
                        let (reader, mut writer) = stream.split_buffered();
                        let reader = reader.fix().unwrap();
                        let mut lines = reader.lines();

                        while let Some(Ok(line)) = lines.next().await {
                            if &line == "exit" {
                                break;
                            }

                            let data = line + "\n";
                            assert!(writer.write_all(data.as_bytes()).await.is_ok());
                            assert!(writer.flush().await.is_ok());
                        }

                        tracing::info!("server completed");
                        assert!(completed.send(()).await.is_ok());
                    });
                }
            }
            .fuse();

            let finish = async move {
                let _ = rx.take(clients).for_each(|_| async {}).await;
                tracing::info!("all done");
            }
            .fuse();

            let server = inel::spawn(async move {
                let mut run = Box::pin(run_server);
                let mut done = Box::pin(finish);
                loop {
                    select! {
                        _ = run => {},
                        _ = done => {
                            break;
                        }
                    };
                }
            });

            for client in 0..clients {
                inel::spawn(async move {
                    tracing::info!(client, "starting");

                    let stream = inel::net::TcpStream::connect_direct(("127.0.0.1", port))
                        .await
                        .unwrap();

                    tracing::info!(client, "connected");

                    let (mut reader, mut writer) = stream.split();

                    for i in 1..=20 {
                        let old = "Hello World!".repeat(i) + "\n";
                        let (old, res) = writer.write_owned(old).await;
                        assert!(res.is_ok_and(|wrote| wrote == old.len()));

                        let (new, res) = reader.read_owned(Box::new([0; 4096])).await;
                        assert!(res.as_ref().is_ok_and(|read| *read == old.len()));
                        assert_eq!(old.as_bytes(), &new.as_slice()[0..res.unwrap()]);
                    }

                    let _ = writer.write_owned("exit".to_string()).await;
                    let _ = writer.shutdown(std::net::Shutdown::Both).await;

                    tracing::info!(client, "done");
                });
            }

            let _ = server.join().await;
        });

        assert!(inel::is_done());
    }
}
