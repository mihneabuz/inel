use std::os::fd::{FromRawFd, IntoRawFd};

use crate::helpers::*;
use futures::{AsyncReadExt, AsyncWriteExt};
use inel::io::{AsyncReadOwned, AsyncWriteOwned, Split};

#[test]
fn create() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        assert!(inel::fs::File::create(name_clone).await.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn open() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::File::create(&name).unwrap();

    inel::block_on(async move {
        assert!(inel::fs::File::open(name_clone).await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn truncate() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, "Hello World!\n").unwrap();

    inel::block_on(async move {
        let res = inel::fs::File::options()
            .writable(true)
            .truncate(true)
            .open(name_clone)
            .await;

        assert!(res.is_ok());

        let mut file = res.unwrap();

        let (_, res) = file.write_owned("Overwritten!\n".to_string()).await;
        assert!(res.is_ok());

        let res = file.sync_all().await;
        assert!(res.is_ok());
    });

    assert_eq!(
        std::fs::read_to_string(&name).unwrap(),
        "Overwritten!\n".to_string()
    );

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn append() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, "Hello World!\n").unwrap();

    inel::block_on(async move {
        let res = inel::fs::File::options()
            .writable(true)
            .readable(true)
            .append(true)
            .open(name_clone)
            .await;

        assert!(res.is_ok());

        let mut file = res.unwrap();
        assert_eq!(format!("{:?}", file), String::from("File"));

        let (_, res) = file.write_owned("Appended!\n".to_string()).await;
        assert!(res.is_ok());

        let res = file.sync_all().await;
        assert!(res.is_ok());
    });

    assert_eq!(
        std::fs::read_to_string(&name).unwrap(),
        "Hello World!\nAppended!\n".to_string()
    );

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn metadata() {
    setup_tracing();

    let data = "Hello world!\n";
    let file = temp_file();
    let file_clone = file.clone();

    let dir = temp_file();
    let dir_clone = dir.clone();

    std::fs::write(&file, data.as_bytes()).unwrap();
    std::fs::create_dir(&dir).unwrap();

    inel::spawn(async move {
        let file = inel::fs::File::open(file_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert_eq!(stats.is_file(), true);
        assert_eq!(stats.is_dir(), false);
        assert_eq!(stats.is_symlink(), false);
        assert_eq!(stats.len() as usize, data.len());
        assert_eq!(stats.is_empty(), false);
        assert!(stats.user_id() > 0);
        assert!(stats.group_id() > 0);
        assert!(!stats.accessed().unwrap().elapsed().unwrap().is_zero());
        assert!(!stats.created().unwrap().elapsed().unwrap().is_zero());
        assert!(!stats.modified().unwrap().elapsed().unwrap().is_zero());

        assert!(format!("{:?}", stats).len() > 0);
    });

    inel::spawn(async move {
        let file = inel::fs::File::open(dir_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert_eq!(stats.is_file(), false);
        assert_eq!(stats.is_dir(), true);
        assert_eq!(stats.is_symlink(), false);
        assert!(stats.user_id() > 0);
        assert!(stats.group_id() > 0);
        assert!(!stats.accessed().unwrap().elapsed().unwrap().is_zero());
        assert!(!stats.created().unwrap().elapsed().unwrap().is_zero());
        assert!(!stats.modified().unwrap().elapsed().unwrap().is_zero());
    });

    inel::run();

    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

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

            let res = writer.sync_data().await;
            assert!(res.is_ok());

            true
        });

        assert_eq!(read.join().await, Some(true));
        assert_eq!(write.join().await, Some(true));
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn split_buffered() {
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

        let (mut reader, mut writer) = file.split_buffered();

        let read = inel::spawn(async move {
            let mut buf = Box::new([0; 256]);
            let res = reader.read(&mut buf[..]).await;
            assert!(res.is_ok_and(|r| r == 0));
            assert_eq!(buf, Box::new([0; 256]));

            true
        });

        let write = inel::spawn(async move {
            let buf = Box::new([b'A'; 512]);
            let res = writer.write(&buf[..]).await;
            assert!(res.is_ok_and(|w| w == 512));
            assert_eq!(buf, Box::new([b'A'; 512]));

            true
        });

        assert_eq!(read.join().await, Some(true));
        assert_eq!(write.join().await, Some(true));
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn raw_fd() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let fd = inel::block_on(async move {
        inel::fs::File::create(name_clone)
            .await
            .unwrap()
            .into_raw_fd()
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    inel::block_on(async move {
        let mut file = unsafe { inel::fs::File::from_raw_fd(fd) };
        let (_, res) = file.write_owned("Hello World!\n".to_string()).await;
        assert!(res.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn no_options() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::File::create(&name).unwrap();

    inel::block_on(async move {
        assert!(inel::fs::File::options().open(name_clone).await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn errors() {
    setup_tracing();

    let mut name = temp_file();
    name.push("nope");

    inel::block_on(async move {
        let err = inel::fs::File::open_direct(name).await;
        assert!(err.is_err());

        let file = unsafe { inel::fs::File::from_raw_fd(6543) };
        assert!(file.sync_all().await.is_err());
        assert!(file.metadata().await.is_err());
        std::mem::forget(file);
    });
}

mod direct {
    use super::*;

    #[test]
    fn create() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        inel::block_on(async move {
            assert!(inel::fs::File::create_direct(name_clone).await.is_ok());
        });

        assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn open() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        std::fs::File::create(&name).unwrap();

        inel::block_on(async move {
            assert!(inel::fs::File::open_direct(name_clone).await.is_ok());
        });

        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn append() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        std::fs::write(&name, "Hello World!\n").unwrap();

        inel::block_on(async move {
            let res = inel::fs::File::options()
                .writable(true)
                .readable(true)
                .append(true)
                .open_direct(name_clone)
                .await;

            assert!(res.is_ok());

            let mut file = res.unwrap();
            assert_eq!(format!("{:?}", file), String::from("DirectFile"));

            let (_, res) = file.write_owned("Appended!\n".to_string()).await;
            assert!(res.is_ok());

            let res = file.sync_all().await;
            assert!(res.is_ok());
        });

        assert_eq!(
            std::fs::read_to_string(&name).unwrap(),
            "Hello World!\nAppended!\n".to_string()
        );

        std::fs::remove_file(&name).unwrap();
    }

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
                .open_direct(name_clone)
                .await
                .unwrap();

            let (mut reader, mut writer) = file.split();

            let read = inel::spawn(async move {
                let (buf, res) = reader.read_owned(Box::new([0; 256])).await;
                assert!(res.is_ok_and(|r| r == 0));
                assert_eq!(buf, Box::new([0; 256]));

                true
            });

            let write = inel::spawn(async move {
                let (buf, res) = writer.write_owned(Box::new([b'A'; 512])).await;
                assert!(res.is_ok_and(|w| w == 512));
                assert_eq!(buf, Box::new([b'A'; 512]));

                let res = writer.sync_data().await;
                assert!(res.is_ok());

                true
            });

            assert_eq!(read.join().await, Some(true));
            assert_eq!(write.join().await, Some(true));
        });

        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn error() {
        setup_tracing();

        let mut name = temp_file();
        name.push("nope");

        inel::block_on(async move {
            let err = inel::fs::File::open_direct(name).await;
            assert!(err.is_err());
        });
    }
}
