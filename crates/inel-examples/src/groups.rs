use std::{io::Result, net::Shutdown};

use inel::{group::ReadBufferSet, net::TcpListener};

use futures::{AsyncBufReadExt, StreamExt};

fn main() -> Result<()> {
    inel::block_on(run())
}

async fn run() -> Result<()> {
    let buffers = ReadBufferSet::with_buffers(16, 4096).await.unwrap();

    let mut incoming = TcpListener::bind_direct(("127.0.0.1", 3030))
        .await?
        .incoming();

    while let Some(conn) = incoming.next().await {
        let Ok(conn) = conn else {
            continue;
        };

        let mut reader = buffers.supply_to(conn);
        inel::spawn(async move {
            let _ = reader.read_line(&mut String::new()).await;
            let _ = reader.inner().shutdown(Shutdown::Both).await;
        });
    }

    Ok(())
}
