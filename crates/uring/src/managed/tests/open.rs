// Opening files needs -Zmiri-disable-isolation, so these don't run under miri.
#![cfg(not(miri))]

use std::{
    ffi::{CStr, CString},
    fs,
    path::{Path, PathBuf},
    pin::pin,
    task::Context,
};

use rustix::{
    fs::{Mode, OFlags},
    io::{Errno, FdFlags, fcntl_getfd},
};

use crate::managed::{
    IoSrc, Op, Open, Read, UringFd, UringProxy, Write,
    tests::{Nop, TestRing, TestWaker, block_on},
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

/// A file only this test opens, so we can tell whether its fd leaked even with
/// other tests opening files in parallel.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("inel-{}-{name}", std::process::id()));
        fs::write(&path, b"").unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn cpath(&self) -> CString {
        CString::new(self.0.clone().into_os_string().into_encoded_bytes()).unwrap()
    }

    /// Whether any fd of the process refers to the file.
    fn is_open(&self) -> bool {
        fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(|entry| fs::read_link(entry.ok()?.path()).ok())
            .any(|target| Path::new(&target) == self.0)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Submits an open of `path` and drops it unread, either once its result is
/// stored or right away (racing the open against its cancel).
fn open_and_lose(ring: &TestRing, path: &CStr, wait_for_result: bool) {
    let mut fut = Box::pin(Open::new(path, OFlags::RDONLY, Mode::empty()).run_on(ring.clone()));
    let tw = TestWaker::new();
    assert!(
        fut.as_mut()
            .poll(&mut Context::from_waker(&tw.waker()))
            .is_pending()
    );
    while wait_for_result && !tw.was_woken() {
        ring.wait();
    }
    drop(fut);
}

/// Waits until the ring closed `file`'s lost fd. The close is submitted with
/// the next op, and may run after it in the fake.
fn assert_closed(ring: &TestRing, file: &TempFile) {
    ring.drain();
    for _ in 0..100 {
        if !file.is_open() {
            return;
        }
        block_on(ring, pin!(Nop.run_on(ring.clone())));
    }
    panic!("the lost fd was never closed");
}

#[test]
fn lost_result_closes_fd() {
    let ring = TestRing::new();
    let file = TempFile::new("lost_result_closes_fd");
    open_and_lose(&ring, &file.cpath(), true);
    assert_closed(&ring, &file);
}

#[test]
fn dropped_before_completion_closes_fd() {
    let ring = TestRing::new();
    let file = TempFile::new("dropped_before_completion_closes_fd");
    open_and_lose(&ring, &file.cpath(), false);
    assert_closed(&ring, &file);
}

#[test]
fn ring_drop_closes_lost_fd() {
    let ring = TestRing::new();
    let file = TempFile::new("ring_drop_closes_lost_fd");
    open_and_lose(&ring, &file.cpath(), true);
    assert!(file.is_open());
    ring.shutdown();
    drop(ring);
    assert!(!file.is_open());
}
