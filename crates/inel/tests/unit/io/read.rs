use futures::future::FusedFuture;
use inel::buffer::Fixed;

use {inel::buffer::StableBufferExt, inel::io::AsyncReadOwned};

use crate::helpers::{setup_tracing, temp_file};

#[test]
fn simple() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let new = Box::new([0; 256]);
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let (new, res) = file.read_owned(new).await;

        assert!(res.is_ok_and(|read| read == 256));

        new
    });

    assert_eq!(new, old);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn offset() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let new = Box::new([0; 256]);
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let (new, res) = file.read_owned_at(128, new).await;

        assert!(res.is_ok_and(|read| read == 128));

        new
    });

    assert_eq!(&new.as_slice()[0..128], &[b'a'; 128]);
    assert_eq!(&new.as_slice()[128..], &[0; 128]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn fixed() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let new = Fixed::new(256).unwrap();
        let (new, res) = file.read_fixed_at(128, new).await;

        assert!(res.is_ok_and(|read| read == 128));

        new
    });

    assert_eq!(&new[0..128], &[b'a'; 128]);
    assert_eq!(&new[128..], &[0; 128]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn view() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let new = Box::new([0; 256]);
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let (new, res) = file.read_owned(new.view(128..)).await;

        assert!(res.is_ok_and(|read| read == 128));

        new
    });

    assert_eq!(&new.inner()[128..], &old[128..]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn cancel() {
    setup_tracing();

    let name = temp_file();
    std::fs::write(&name, [b'a'; 128_000]).unwrap();

    for _ in 0..1000 {
        let index = rand::random::<u64>() as usize % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Fixed::new(16_000).unwrap();
            let mut file = inel::fs::File::options()
                .readable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            let mut read = file.read_fixed_at((index * 4_000) as u64, buf);

            futures::select! {
                (_, _) = read => {
                    false
                },
                () = inel::time::instant() => {
                    assert!(!read.is_terminated());
                    true
                }
            }
        });
    }

    for _ in 0..1000 {
        let index = rand::random::<u64>() as usize % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Box::new([0; 16_000]);
            let mut file = inel::fs::File::options()
                .readable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            let mut read = file.read_owned_at((index * 4_000) as u64, buf);

            futures::select! {
                (_, _) = read => {
                    false
                },
                () = inel::time::instant() => {
                    assert!(!read.is_terminated());
                    true
                }
            }
        });
    }

    std::fs::remove_file(&name).unwrap();
}
