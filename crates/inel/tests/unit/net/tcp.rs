use std::{
    io::{Read, Write},
    os::fd::{FromRawFd, IntoRawFd},
    thread::JoinHandle,
};

use futures::{AsyncBufReadExt, AsyncWriteExt, StreamExt};
use inel::{
    buffer::StableBufferExt,
    io::{AsyncReadOwned, AsyncWriteOwned},
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

    let (port_send, port_recv) = oneshot::channel::<u16>();

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
fn full() {
    setup_tracing();

    inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("::1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let h1 = inel::spawn(async move {
            let client = inel::net::TcpStream::connect(("::1", port)).await.unwrap();
            let mut writer = inel::io::BufWriter::new(client);
            let data = "Hello World!\n".repeat(4096).as_bytes().to_vec();
            assert!(writer.write_all(&data).await.is_ok());
            assert!(writer.flush().await.is_ok());
        });

        let h2 = inel::spawn(async move {
            let (conn, addr) = listener.accept().await.unwrap();
            assert!(addr.is_ipv6());
            let mut lines = inel::io::BufReader::new(conn).lines();
            while let Some(line) = lines.next().await {
                assert!(line.is_ok());
                assert_eq!(line.unwrap().as_str(), "Hello World!");
            }
        });

        assert!(h1.join().await.is_some());
        assert!(h2.join().await.is_some());
    });
}
