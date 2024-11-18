use inel::io::AsyncWriteOwned;

use crate::{setup_tracing, temp_file};

#[test]
fn simple() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let buf = Box::new([b'a'; 256]);
        let (buf, res) = file.write_owned(buf).await;

        assert!(res.is_ok_and(|wrote| wrote == 256));
        assert_eq!(buf, Box::new([b'a'; 256]));

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(buf.as_slice(), data.as_bytes());

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn offset() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let buf = Box::new([b'a'; 256]);
        let (buf, res) = file.write_owned_at(128, buf).await;

        assert!(res.is_ok_and(|wrote| wrote == 256));
        assert_eq!(buf, Box::new([b'a'; 256]));

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(buf.as_slice(), &data.as_bytes()[128..]);
    assert_eq!(&data.as_bytes()[..128], &[0; 128]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn sync() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let (_, res) = file.write_owned(Box::new([b'a'; 256])).await;
        assert!(res.is_ok_and(|wrote| wrote == 256));

        assert!(file.sync().await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}
