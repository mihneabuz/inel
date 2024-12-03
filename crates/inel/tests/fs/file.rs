use std::{
    os::fd::{FromRawFd, IntoRawFd},
    time::Duration,
};

use crate::helpers::*;
use inel::io::AsyncWriteOwned;

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

        let res = file.sync().await;
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

        let (_, res) = file.write_owned("Appended!\n".to_string()).await;
        assert!(res.is_ok());

        let res = file.sync().await;
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

        assert!(format!("{:?}", stats).len() > 0);
    });

    inel::spawn(async move {
        let file = inel::fs::File::open(dir_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert_eq!(stats.is_file(), false);
        assert_eq!(stats.is_dir(), true);
        assert_eq!(stats.is_symlink(), false);
    });

    inel::run();

    std::fs::remove_file(&file).unwrap();
    std::fs::remove_dir(&dir).unwrap();
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
fn drop_close() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::File::create(&name).unwrap();

    let fd = inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let fd = file.into_raw_fd();
        let _ = unsafe { inel::fs::File::from_raw_fd(fd) };
        fd
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));
    std::thread::sleep(Duration::from_millis(60));

    inel::block_on(async move {
        let file = unsafe { inel::fs::File::from_raw_fd(fd) };
        let meta = file.metadata().await;
        assert!(meta.is_err());
        std::mem::forget(file);
    });

    std::fs::remove_file(&name).unwrap();
}
