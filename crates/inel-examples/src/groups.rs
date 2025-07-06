use std::{io::Result, net::Shutdown};

use inel::{io::ReadBuffers, net::TcpListener};

use futures::{AsyncBufReadExt, StreamExt};

fn main() -> Result<()> {
    inel::block_on(run())
}

async fn run() -> Result<()> {
    let buffers = ReadBuffers::new(4096, 16).await.unwrap();

    let mut incoming = TcpListener::bind_direct(("127.0.0.1", 3030))
        .await?
        .incoming();

    while let Some(conn) = incoming.next().await {
        let Ok(conn) = conn else {
            continue;
        };

        let mut reader = buffers.provide_to(conn);
        inel::spawn(async move {
            let _ = reader.read_line(&mut String::new()).await;
            let _ = reader.inner().shutdown(Shutdown::Both).await;
        });
    }

    Ok(())
}
