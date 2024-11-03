mod helpers;
mod read;
mod write;

use std::{
    os::fd::{FromRawFd, IntoRawFd},
    time::Duration,
};

use helpers::*;
use inel_macro::test_repeat;

#[test]
fn create_file() {
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
fn open_file() {
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
fn create_dir() {
    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        let res = inel::fs::DirBuilder::new().create(name_clone).await;
        assert!(res.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_dir(&name).unwrap();
}

#[test]
fn create_dir_recursive() {
    let base = temp_file();
    let name = base.join("hello").join("world").join("!");
    let name_clone = name.clone();

    inel::block_on(async move {
        let res = inel::fs::DirBuilder::new()
            .recursive(true)
            .create(name_clone)
            .await;

        assert!(res.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn file_metadata() {
    setup_tracing();

    let data = "Hello world!\n";
    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, data.as_bytes()).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert!(stats.is_file());
        assert_eq!(stats.len() as usize, data.len());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn dir_metadata() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::create_dir(&name).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert!(stats.is_dir());
    });

    std::fs::remove_dir(&name).unwrap();
}

#[test]
#[test_repeat(10)]
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
    std::thread::sleep(Duration::from_millis(10));

    inel::block_on(async move {
        let file = unsafe { inel::fs::File::from_raw_fd(fd) };
        let meta = file.metadata().await;
        assert!(meta.is_err());
        std::mem::forget(file);
    });

    std::fs::remove_file(&name).unwrap();
}
