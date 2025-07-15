use std::convert::Infallible;

use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::{
    Error, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};

use inel::compat::stream::BufStream;
use inel::io::AsyncWriteOwned;

fn main() -> std::io::Result<()> {
    inel::block_on(run())
}

async fn run() -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = inel::net::TcpListener::bind(addr).await?;

    print("Started server\n".to_string()).await;

    while let Ok((stream, peer)) = listener.accept().await {
        print(format!("Received connection from {peer}")).await;

        inel::spawn(async move {
            let stream = BufStream::new(stream);
            if let Err(err) = inel::compat::hyper::serve_http1(stream, service_fn(service)).await {
                print(format!("Error: {err:?}")).await;
            }
        });
    }

    Ok(())
}

async fn service(req: Request<Incoming>) -> Result<Response<BoxBody<Bytes, Error>>, Infallible> {
    match req.uri().path() {
        "/" => Ok(Response::new(req.into_body().boxed())),
        "/hello" => Ok(Response::new(
            Full::new(Bytes::from("Hello from hyper + inel!\n"))
                .map_err(|never| match never {})
                .boxed(),
        )),
        _ => {
            let mut not_found = Response::new(
                Empty::<Bytes>::new()
                    .map_err(|never| match never {})
                    .boxed(),
            );
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

async fn print(message: String) {
    let _ = inel::io::stdout().write_owned(message).await;
}
