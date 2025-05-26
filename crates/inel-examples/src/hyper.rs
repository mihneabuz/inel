use std::convert::Infallible;

use futures::StreamExt;
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::{
    Error, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};

use inel::compat::hyper::HyperStream;
use inel::io::AsyncWriteOwned;

fn main() {
    inel::block_on(async { run().await.unwrap() })
}

async fn print(message: String) {
    let _ = inel::io::stdout().write_owned(message).await;
}

async fn run() -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = inel::net::TcpListener::bind(addr).await?;

    print("Started server\n".to_string()).await;

    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        print("Received connection\n".to_string()).await;

        inel::spawn(async move {
            let hyper = HyperStream::new(stream);

            let res = http1::Builder::new()
                .serve_connection(hyper, service_fn(service))
                .await;

            if let Err(e) = res {
                print(format!("Error: {:?}", e)).await;
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
