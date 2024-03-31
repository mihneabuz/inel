mod helpers;

use std::{net::IpAddr, str::FromStr, thread, time::Duration};

use helpers::*;

#[test]
fn tcp_listener() {
    setup_tracing();

    let port = 3322;

    let runtime = thread::spawn(move || {
        inel::block_on(async move {
            let listener = inel::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
            let (_, peer) = listener.accept().await.unwrap();
            assert_eq!(peer.ip(), IpAddr::from_str("127.0.0.1").unwrap());

            1
        })
    });

    thread::sleep(Duration::from_millis(20));

    let res = std::net::TcpStream::connect(("127.0.0.1", port));

    assert!(res.is_ok());
    assert_eq!(runtime.join().unwrap(), 1);
}
