use tracing::debug;
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    inel::block_on(async {
        let listener = inel::net::TcpListener::bind(("127.0.0.1", 3000)).unwrap();

        while let Ok((stream, peer)) = listener.accept().await {
            debug!(?peer, "Connection received");
        }
    })
}
