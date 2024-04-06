use futures::{AsyncBufReadExt, StreamExt};
use tracing::{error, level_filters::LevelFilter};

use inel::AsyncRingWrite;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::INFO)
        .init();

    inel::block_on(async {
        let mut lines = inel::io::RingBufReader::new(std::io::stdin())
            .lines()
            .enumerate();

        while let Some((num, line)) = lines.next().await {
            let Ok(line) = line else {
                error!("Failed to read line");
                continue;
            };

            let buf = format!("{num:#3}: {line}\n").as_bytes().to_vec();
            let (_, res) = std::io::stdout().ring_write(buf).await;
            if let Err(err) = res {
                error!(?err, "Failed to write line");
            }
        }
    });
}
