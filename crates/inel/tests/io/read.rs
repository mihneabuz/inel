use {inel::buffer::StableBufferExt, inel::io::AsyncReadOwned};

use crate::{setup_tracing, temp_file};

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

        let new = Box::new([0; 256]).fix().unwrap();
        let (new, res) = file.read_fixed_at(128, new).await;

        assert!(res.is_ok_and(|read| read == 128));

        new
    });

    assert_eq!(&new.inner().as_slice()[0..128], &[b'a'; 128]);
    assert_eq!(&new.inner().as_slice()[128..], &[0; 128]);

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
        let index = rand::random::<usize>() % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Box::new([0; 16_000]).fix().unwrap();
            let mut file = inel::fs::File::options()
                .readable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            futures::select! {
                (_, _) = file.read_fixed_at((index * 4_000) as u64, buf) => {
                    false
                },
                () = inel::time::sleep(std::time::Duration::from_micros(0)) => {
                    true
                }
            }
        });
    }

    std::fs::remove_file(&name).unwrap();
}
