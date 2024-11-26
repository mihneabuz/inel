use futures::{AsyncBufReadExt, StreamExt};
use helpers::{setup_tracing, temp_file};
use inel::io::{AsyncReadOwned, AsyncWriteOwned};

mod bufreader;
mod bufwriter;
mod helpers;
mod read;
mod write;

#[test]
fn split() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        let file = inel::fs::File::options()
            .readable(true)
            .writable(true)
            .create(true)
            .open(name_clone)
            .await
            .unwrap();

        let (mut reader, mut writer) = file.split();

        let read = inel::spawn(async move {
            let (buf, res) = reader.read_owned(Box::new([0; 256])).await;
            assert!(res.is_ok_and(|r| r == 0));
            assert_eq!(buf, Box::new([0; 256]));

            let res = reader.metadata().await;
            assert!(res.is_ok());

            true
        });

        let write = inel::spawn(async move {
            let (buf, res) = writer.write_owned(Box::new([b'A'; 512])).await;
            assert!(res.is_ok_and(|w| w == 512));
            assert_eq!(buf, Box::new([b'A'; 512]));

            let res = writer.sync().await;
            assert!(res.is_ok());

            true
        });

        assert_eq!(read.join().await, Some(true));
        assert_eq!(write.join().await, Some(true));
    });

    std::fs::remove_file(&name).unwrap();
}
