use axum::{
    Router,
    response::Redirect,
    routing::{get, post},
};

use hyper::{StatusCode, Uri};
use inel::compat;
use rustls::ServerConfig;
use rustls_pemfile::{certs, private_key};

const KEY: &[u8] = include_bytes!("../certs/localhost.key");
const CERT: &[u8] = include_bytes!("../certs/localhost.cert");

fn tls() -> ServerConfig {
    let cert = certs(&mut std::io::BufReader::new(std::io::Cursor::new(CERT)))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let key = private_key(&mut std::io::BufReader::new(std::io::Cursor::new(KEY)))
        .unwrap()
        .unwrap();

    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert, key)
        .unwrap()
}

fn main() {
    let app = Router::new()
        .route("/hello", get(|| async { "Hello from axum + inel!" }))
        .route("/echo", post(|body: String| async { body }))
        .route(
            "/reverse",
            post(|body: String| async move { body.chars().rev().collect::<String>() }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Leave me alone!") });

    let redirect = Router::new().fallback(async |uri: Uri| {
        let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
        Redirect::permanent(&format!("https://localhost:3443{path_and_query}"))
    });

    inel::spawn(compat::axum::serve(("127.0.0.1", 3080), redirect));
    inel::spawn(compat::axum::serve_tls(("127.0.0.1", 3443), tls(), app));

    inel::run();
}
