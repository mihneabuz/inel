// Opening files needs -Zmiri-disable-isolation, so these don't run under miri.
#![cfg(not(miri))]

use std::{ffi::CStr, fs, marker::PhantomData, path::PathBuf, pin::pin};

use rustix::{
    fs::{Mode, OFlags},
    io::Errno,
};

use crate::managed::{
    DirectFd, IoSrc, OpenDirect, RawSrc, Read, Write,
    submission::Submission,
    tests::{TestRing, TestWaker, block_on},
};

const CARGO_TOML: &CStr = c"Cargo.toml";

fn open(ring: &TestRing, path: &CStr, flags: OFlags) -> Result<DirectFd<TestRing>, Errno> {
    let op = OpenDirect::new(path, flags, Mode::empty());
    block_on(ring, pin!(Submission::new(op, ring.clone())))
}

/// Opens `path` into a full table, retrying while the close freeing a slot
/// hasn't run yet (the fake runs requests in any order).
fn open_retrying(ring: &TestRing, path: &CStr) -> DirectFd<TestRing> {
    for _ in 0..100 {
        match open(ring, path, OFlags::RDONLY) {
            Err(Errno::NFILE) => continue,
            res => return res.unwrap(),
        }
    }
    panic!("the table stayed full");
}

fn read_at(ring: &TestRing, src: &impl IoSrc, len: usize, at: usize) -> Result<Vec<u8>, Errno> {
    let op = Read::new_at(src, vec![0; len], at);
    let (buf, res) = block_on(ring, pin!(Submission::new(op, ring.clone())));
    res.map(|n| buf[..n].to_vec())
}

/// A slot of the table, owned by nobody.
struct Slot(u32);

impl IoSrc for Slot {
    fn raw_src(&self) -> RawSrc<'_> {
        RawSrc::Direct(self.0, PhantomData)
    }
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
fn cloexec_is_ignored() {
    let ring = TestRing::new();
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY | OFlags::CLOEXEC).unwrap();
    assert_eq!(read_at(&ring, &fd, 1, 0).unwrap(), b"[");
}

#[test]
fn write_direct() {
    let ring = TestRing::new();
    let path = std::env::temp_dir().join(format!("inel-{}-write_direct", std::process::id()));
    fs::write(&path, b"0123456789").unwrap();
    let cpath = std::ffi::CString::new(path.clone().into_os_string().into_encoded_bytes()).unwrap();

    let fd = open(&ring, &cpath, OFlags::RDWR).unwrap();
    let op = Write::new_at(&fd, "ab", 3);
    let (_, res) = block_on(&ring, pin!(Submission::new(op, ring.clone())));
    drop(fd);
    let contents = fs::read(&path).unwrap();
    let _ = fs::remove_file(PathBuf::from(&path));

    assert_eq!(res, Ok(2));
    assert_eq!(contents, b"012ab56789");
}

#[test]
fn empty_slot_is_bad() {
    let ring = TestRing::new();
    assert_eq!(read_at(&ring, &Slot(5), 1, 0).err(), Some(Errno::BADF));
}

#[test]
fn slot_out_of_table_is_bad() {
    let ring = TestRing::with_direct_files(4);
    assert_eq!(read_at(&ring, &Slot(4), 1, 0).err(), Some(Errno::BADF));
}

#[test]
fn distinct_slots() {
    let ring = TestRing::new();
    let a = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    let b = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    assert_ne!(a.index(), b.index());
}

#[test]
fn table_full() {
    let ring = TestRing::with_direct_files(2);
    let _a = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    let _b = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    let res = open(&ring, CARGO_TOML, OFlags::RDONLY);
    assert_eq!(res.err(), Some(Errno::NFILE));
}

#[test]
fn drop_closes_slot() {
    let ring = TestRing::with_direct_files(1);
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    let slot = fd.index();
    drop(fd);

    // the only slot comes back
    let fd = open_retrying(&ring, CARGO_TOML);
    assert_eq!(fd.index(), slot);
}

#[test]
fn closed_slot_is_bad() {
    let ring = TestRing::new();
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    let slot = fd.index();
    drop(fd);

    // the close may run after the read in the fake
    for _ in 0..100 {
        if read_at(&ring, &Slot(slot), 1, 0) == Err(Errno::BADF) {
            return;
        }
    }
    panic!("the slot was never closed");
}

#[test]
fn drop_during_cancelled_read_closes_after_it() {
    let ring = TestRing::with_direct_files(1);
    let fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();

    let mut fut = Box::pin(Submission::new(
        Read::new_at(&fd, vec![0; 8], 0),
        ring.clone(),
    ));
    let tw = TestWaker::new();
    let _ = fut
        .as_mut()
        .poll(&mut std::task::Context::from_waker(&tw.waker()));
    drop(fut);
    // the read may still be in flight, it keeps the file it already resolved
    drop(fd);
    ring.drain();

    open_retrying(&ring, CARGO_TOML);
}

#[test]
fn shutdown_with_open_fd() {
    let ring = TestRing::new();
    let _fd = open(&ring, CARGO_TOML, OFlags::RDONLY).unwrap();
    crate::managed::UringProxy::shutdown(&ring);
}
