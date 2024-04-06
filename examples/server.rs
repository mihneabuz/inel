use std::io::Result;

use tracing::{error, info, level_filters::LevelFilter};

use inel::AsyncRingWrite;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();

    let message = "Hello world!\n";
    let buffer = message.as_bytes().to_vec();

    inel::block_on(async move {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 3000)).await?;
        while let Ok((mut stream, peer)) = listener.accept().await {
            info!(?peer, "Received connection");

            let buffer_clone = buffer.clone();
            inel::spawn(async move {
                let (_, res) = stream.ring_write(buffer_clone).await;
                if let Err(e) = res {
                    error!(error =? e, "Failed to write to stream");
                }
            });
        }

        Ok(())
    })
}
