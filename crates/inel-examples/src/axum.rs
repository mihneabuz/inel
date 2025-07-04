use axum::{
    Router,
    routing::{get, post},
};
use futures::StreamExt;
use hyper::{StatusCode, server::conn::http1};
use tower::Service;

use inel::compat::hyper::HyperStream;
use inel::io::AsyncWriteOwned;

fn main() {
    let app = Router::new()
        .route("/hello", get(|| async { "Hello from axum + inel!" }))
        .route("/echo", post(|body: String| async { body }))
        .route(
            "/reverse",
            post(|body: String| async move { body.chars().rev().collect::<String>() }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Leave me alone!") });

    inel::block_on(async { run(app).await.unwrap() })
}

async fn run(app: Router) -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = inel::net::TcpListener::bind(addr).await?;

    print("Started server\n".to_string()).await;

    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        print("Received connection\n".to_string()).await;

        let tower_service = app.clone();

        inel::spawn(async move {
            let hyper = HyperStream::new(stream);

            let hyper_service =
                hyper::service::service_fn(move |request| tower_service.clone().call(request));

            let res = http1::Builder::new()
                .serve_connection(hyper, hyper_service)
                .await;

            if let Err(e) = res {
                print(format!("Error: {e:?}")).await;
            }
        });
    }

    Ok(())
}

async fn print(message: String) {
    let _ = inel::io::stdout().write_owned(message).await;
}
