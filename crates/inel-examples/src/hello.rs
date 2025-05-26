use inel::io::AsyncWriteOwned;

#[inel::main]
async fn main() {
    let (_, res) = inel::io::stdout()
        .write_owned("Hello from inel!\n".to_string())
        .await;

    assert!(res.is_ok());
}
