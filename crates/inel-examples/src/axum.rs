use axum::{
    Router,
    routing::{get, post},
};

use hyper::StatusCode;

fn main() -> std::io::Result<()> {
    let app = Router::new()
        .route("/hello", get(|| async { "Hello from axum + inel!" }))
        .route("/echo", post(|body: String| async { body }))
        .route(
            "/reverse",
            post(|body: String| async move { body.chars().rev().collect::<String>() }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Leave me alone!") });

    inel::block_on(inel::compat::axum::serve(("127.0.0.1", 3000), app))
}
