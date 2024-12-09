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
