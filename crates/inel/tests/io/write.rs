use inel::{buffer::StableBufferExt, io::AsyncWriteOwned};

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
fn fixed() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();

        let buf = Box::new([b'a'; 256]).fix().unwrap();
        let (buf, res) = file.write_fixed_at(128, buf).await;

        assert!(res.is_ok_and(|wrote| wrote == 256));
        assert_eq!(buf.inner(), &Box::new([b'a'; 256]));

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(buf.into_inner().as_slice(), &data.as_bytes()[128..]);
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

        assert!(file.sync().await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn cancel() {
    setup_tracing();

    let name = temp_file();
    std::fs::write(&name, [0u8; 128_000]).unwrap();

    for _ in 0..1000 {
        let index = rand::random::<usize>() % 16;
        let name_clone = name.clone();

        inel::block_on(async move {
            let buf = Box::new([b'a'; 16_000]).fix().unwrap();
            let mut file = inel::fs::File::options()
                .writable(true)
                .direct(true)
                .open(name_clone)
                .await
                .unwrap();

            futures::select! {
                (_, _) = file.write_fixed_at((index * 4_000) as u64, buf) => {
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
