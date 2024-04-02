use futures::{SinkExt, StreamExt};
use tracing_subscriber::EnvFilter;

use inel::{AsyncRingRead, AsyncRingWrite};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    inel::block_on(async {
        let (mut sender, mut receiver) = futures::channel::mpsc::channel(10);

        inel::spawn(async move {
            let mut stdin = std::io::stdin();
            let mut buf = vec![0u8; 1024];

            while let Ok((read, len)) = stdin.ring_read(buf).await {
                if len <= 1 {
                    break;
                }

                let line = String::from_utf8_lossy(&read[0..len]).to_string();

                sender.send(line).await.unwrap();

                buf = read;
            }
        });

        inel::spawn(async move {
            let mut stdout = std::io::stdout();

            while let Some(line) = receiver.next().await {
                let message = format!("Echo: {line}");
                stdout.ring_write(message.into()).await.unwrap();
            }
        });
    });
}
