use futures::future::FusedFuture;
use inel::{
    buffer::{Fixed, StableBufferExt},
    io::AsyncWriteOwned,
};

use crate::helpers::{setup_tracing, temp_file};

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
fn fixed() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let buf = Fixed::register(Box::new([b'a'; 256])).unwrap();
        let (buf, res) = file.write_fixed_at(128, buf).await;

        assert!(res.is_ok_and(|wrote| wrote == 256));
        assert_eq!(&buf[..], &[b'a'; 256]);

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(&buf[..], &data.as_bytes()[128..]);
    assert_eq!(&data.as_bytes()[..128], &[0; 128]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn view() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let buf = Box::new([b'a'; 256]);
        let (buf, res) = file.write_owned(buf.view(128..)).await;

        assert!(res.is_ok_and(|wrote| wrote == 128));
        assert_eq!(buf.inner(), &Box::new([b'a'; 256]));

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    let buf = buf.unview();
    assert_eq!(buf.as_slice()[..128], [b'a'; 128]);
    assert_eq!(&buf.as_slice()[128..], data.as_bytes());

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

        assert!(file.sync_all().await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn cancel() {
    setup_tracing();

    let name = temp_file();
    std::fs::write(&name, [0u8; 128_000]).unwrap();

    for _ in 0..1000 {
        let index = rand::random::<u64>() as usize % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Fixed::register(Box::new([b'a'; 16_000])).unwrap();
            let mut file = inel::fs::File::options()
                .writable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            let mut write = file.write_fixed_at((index * 4_000) as u64, buf);

            futures::select! {
                (_, _) = write => {
                    false
                },
                () = inel::time::instant() => {
                    assert!(!write.is_terminated());
                    true
                }
            }
        });
    }

    for _ in 0..1000 {
        let index = rand::random::<u64>() as usize % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Box::new([b'a'; 16_000]);
            let mut file = inel::fs::File::options()
                .writable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            let mut write = file.write_owned_at((index * 4_000) as u64, buf);

            futures::select! {
                (_, _) = write => {
                    false
                },
                () = inel::time::instant() => {
                    assert!(!write.is_terminated());
                    true
                }
            }
        });
    }

    std::fs::remove_file(&name).unwrap();
}
