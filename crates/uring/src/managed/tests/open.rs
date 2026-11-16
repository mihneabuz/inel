// Opening files needs -Zmiri-disable-isolation, so these don't run under miri.
#![cfg(not(miri))]

use std::{ffi::CStr, fs, path::PathBuf, pin::pin};

use rustix::{
    fs::{Mode, OFlags},
    io::{Errno, FdFlags, fcntl_getfd},
};

use crate::managed::{
    IoSrc, Op, Open, Read, UringFd, Write,
    tests::{TestRing, block_on},
};

const CARGO_TOML: &CStr = c"Cargo.toml";

fn open(ring: &TestRing, path: &CStr, flags: OFlags) -> Result<UringFd<TestRing>, Errno> {
    let op = Open::new(path, flags, Mode::empty());
    block_on(ring, pin!(op.run_on(ring.clone())))
}

fn read_at(ring: &TestRing, src: &impl IoSrc, len: usize, at: usize) -> Result<Vec<u8>, Errno> {
    let op = Read::new_at(src, vec![0; len], at);
    let (buf, res) = block_on(ring, pin!(op.run_on(ring.clone())));
    res.map(|n| buf[..n].to_vec())
}

#[test]
fn open_and_read() {
    let ring = TestRing::new();
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();

    assert_eq!(read_at(&ring, &fd, 8, 1).unwrap(), b"package]");
}

#[test]
fn open_missing() {
    let ring = TestRing::new();
    let res = open(&ring, c"does/not/exist", OFlags::RDONLY);
    assert_eq!(res.err(), Some(Errno::NOENT));
}

#[test]
fn cloexec_is_added() {
    let ring = TestRing::new();
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    assert!(fcntl_getfd(&fd).unwrap().contains(FdFlags::CLOEXEC));
}

#[test]
fn write_regular() {
    let ring = TestRing::new();
    let path = std::env::temp_dir().join(format!("inel-{}-write_regular", std::process::id()));
    fs::write(&path, b"0123456789").unwrap();
    let cpath = std::ffi::CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();

    let fd = open(&ring, &cpath, OFlags::RDWR).unwrap();
    let op = Write::new_at(&fd, "ab", 3);
    let (_, res) = block_on(&ring, pin!(op.run_on(ring.clone())));
    drop(fd);
    ring.drain();
    let contents = fs::read(&path).unwrap();
    let _ = fs::remove_file(PathBuf::from(&path));

    assert_eq!(res, Ok(2));
    assert_eq!(contents, b"012ab56789");
}

#[test]
fn drop_then_drain() {
    let ring = TestRing::new();
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    drop(fd);
    ring.drain();
}

#[test]
fn shutdown_with_open_fd() {
    let ring = TestRing::new();
    let _fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    crate::managed::UringProxy::shutdown(&ring);
}
