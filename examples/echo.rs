use futures::{SinkExt, StreamExt};
use tracing::level_filters::LevelFilter;

use inel::{AsyncRingRead, AsyncRingWrite};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();

    inel::block_on(async {
        let (mut sender, mut receiver) = futures::channel::mpsc::channel(10);

        inel::spawn(async move {
            let mut stdin = std::io::stdin();
            let mut buf = vec![0u8; 1024];

            loop {
                let (read, len) = stdin.ring_read(buf).await;
                if !len.as_ref().is_ok_and(|len| *len > 1) {
                    break;
                }

                let line = String::from_utf8_lossy(&read[0..len.unwrap()]).to_string();

                sender.send(line).await.unwrap();

                buf = read;
            }
        });

        inel::spawn(async move {
            let mut stdout = std::io::stdout();

            while let Some(line) = receiver.next().await {
                let message = format!("Echo: {line}");
                let (_, len) = stdout.ring_write(message.into()).await;
                len.unwrap();
            }
        });
    });
}
