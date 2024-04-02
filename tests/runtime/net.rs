use std::io::{Error, Read, Result, Write};

use inel::{AsyncRingRead, AsyncRingWrite};
use tracing::debug;

use crate::helpers::*;

#[test]
fn listener() {
    setup_tracing();

    const MESSAGE: &str = "hello world!";

    let mut port: u16 = 0;
    while port < 2000 {
        port = rand::random();
    }

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();

        debug!(to =? stream.local_addr(), "Connected");

        stream.write_all(MESSAGE.as_bytes()).unwrap();

        let mut s = String::new();
        stream.read_to_string(&mut s).unwrap();
        assert_eq!(&s, MESSAGE);
    });

    let res: Result<()> = inel::block_on(async move {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let (mut stream, peer) = listener.accept().await?;

        debug!(?peer, "Received connection");

        let buf = vec![0u8; 128];
        let (mut buf, len) = stream.ring_read(buf).await?;

        let s = String::from_utf8_lossy(&buf[0..len]).to_string();
        assert_eq!(&s, MESSAGE);

        buf.truncate(MESSAGE.len());
        buf.copy_from_slice(MESSAGE.as_bytes());

        stream.ring_write(buf).await?;

        Ok(())
    });

    assert!(res.is_ok());
}

#[test]
fn stream() {
    setup_tracing();

    const MESSAGE: &str = "hello world!";

    let mut port: u16 = 0;
    while port < 2000 {
        port = rand::random();
    }

    std::thread::spawn(move || {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        let (mut stream, peer) = listener.accept().unwrap();

        debug!(?peer, "Received connection");

        let mut buf = [0u8; 128];
        let len = stream.read(&mut buf).unwrap();

        let s = String::from_utf8_lossy(&buf[0..len]).to_string();

        assert_eq!(&s, MESSAGE);

        stream.write(MESSAGE.as_bytes()).unwrap();
    });

    let res: Result<()> = inel::block_on(async move {
        let mut stream = inel::net::TcpStream::connect(("127.0.0.1", port)).await?;

        let buf = MESSAGE.as_bytes().to_vec();
        stream.ring_write(buf).await?;

        let buf = vec![0u8; 128];
        let (buf, len) = stream.ring_read(buf).await?;

        let s = String::from_utf8_lossy(&buf[0..len]).to_string();

        assert_eq!(&s, MESSAGE);

        Ok(())
    });

    assert!(res.is_ok());
}

#[test]
fn both() {
    setup_tracing();

    const MESSAGE: &str = "hello world!";

    let mut port: u16 = 0;
    while port < 2000 {
        port = rand::random();
    }

    inel::spawn(async move {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", port)).await?;
        let (mut stream, peer) = listener.accept().await?;

        debug!(?peer, "Received connection");

        let buf = vec![0u8; 128];
        let (mut buf, len) = stream.ring_read(buf).await?;

        let s = String::from_utf8_lossy(&buf[0..len]).to_string();
        assert_eq!(&s, MESSAGE);

        buf.truncate(MESSAGE.len());
        buf.copy_from_slice(MESSAGE.as_bytes());

        stream.ring_write(buf).await?;

        Ok::<(), Error>(())
    });

    inel::spawn(async move {
        let mut stream = inel::net::TcpStream::connect(("127.0.0.1", port)).await?;

        let buf = MESSAGE.as_bytes().to_vec();
        stream.ring_write(buf).await?;

        let buf = vec![0u8; 128];
        let (buf, len) = stream.ring_read(buf).await?;

        let s = String::from_utf8_lossy(&buf[0..len]).to_string();

        assert_eq!(&s, MESSAGE);

        Ok::<(), Error>(())
    });

    inel::run();
}
