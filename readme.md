# 💍 inel

> _inel_ (pronunced *ee-nel*) means *ring* in romanian

Inel is an event loop async runtime built on top of `io_uring`. It is designed to be lightweight, relatively safe, and easy to use for writing efficient, I/O-heavy applications on Linux.

## Features

Inel supports modern `io_uring` features such as:
 - direct file descriptors
 - fixed buffers
 - provide buffers
 - multi-shot operations
 - operation chaining

## Example

```rust
fn main() -> std::io::Result<()> {
    inel::block_on(run())
}

async fn run() -> std::io::Result<()> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));

    // Create a TcpListener just like you would in sync rust
    let listener = inel::net::TcpListener::bind(addr).await?;

    // We can turn the listener into a multi-shot [futures::Stream]
    // yielding new connections
    let mut incoming = listener.incoming();

    while let Some(Ok(stream)) = incoming.next().await {
        // Wrap the raw stream with buffered adapter which
        // implements [futures::AsyncWrite] and [futures::AsyncRead]
        let stream = inel::compat::stream::BufStream::new(stream);

        // Spawn a new task for each connection
        inel::spawn(handle_connection(stream));
    }

    Ok(())
}

async fn handle_connection<S>(stream: S)
where
    S: futures::AsyncRead + futures::AsyncWrite,
{
    todo!()
}
```

## Compat

Inel provides compatibility layers for popular crates in the async Rust ecosystem:
 - `futures`: adaptors for AsyncRead, AsyncBufRead, AsyncWrite, Stream
 - `hyper`: support for both client/server, HTTP/1 and HTTP/2
 - `rustls`: for TLS-secured connections
 - `axum`: easily deploy axum apps on the inel runtime
