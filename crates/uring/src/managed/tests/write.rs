use std::{
    fs::{self, File},
    path::PathBuf,
    pin::pin,
    task::Context,
};

use rustix::{
    io::{self, Errno},
    pipe,
};

use crate::managed::{
    IoBuf,
    ops::Write,
    submission::Submission,
    tests::{BadFd, Nop, TestRing, TestWaker, block_on},
};

/// Writes `buf` into a pipe and checks that exactly `expected` came out.
fn write_pipe<B: IoBuf>(buf: B, expected: &[u8]) -> B {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();

    let fut = pin!(Submission::new(Write::new(&tx, buf), ring.clone()));
    let (buf, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(expected.len()));

    drop(tx);
    let mut out = vec![0u8; expected.len() + 16];
    let n = io::read(&rx, &mut out).unwrap();
    assert_eq!(&out[..n], expected);
    buf
}

#[test]
fn write_static_slice() {
    write_pipe(&b"hello"[..], b"hello");
}

#[test]
fn write_static_str() {
    write_pipe("hello", b"hello");
}

#[test]
fn write_string() {
    let buf = write_pipe(String::from("hello"), b"hello");
    assert_eq!(buf, "hello");
}

#[test]
fn write_box() {
    let buf = write_pipe(b"hello".to_vec().into_boxed_slice(), b"hello");
    assert_eq!(&*buf, b"hello");
}

#[test]
fn write_vec_sends_len() {
    let mut vec = Vec::with_capacity(32);
    vec.extend_from_slice(b"hello");

    let buf = write_pipe(vec, b"hello");
    assert_eq!(buf, b"hello");
    assert_eq!(buf.capacity(), 32);
}

#[test]
fn write_view() {
    let buf = write_pipe(b"0123456789".to_vec().view(3..7), b"3456");
    assert_eq!(buf.into_inner(), b"0123456789");
}

#[test]
fn write_view_clamps_past_end() {
    let mut vec = Vec::with_capacity(32);
    vec.extend_from_slice(b"hello");

    // the spare capacity is never sent
    write_pipe(vec.view(2..20), b"llo");
}

#[test]
fn write_bad_fd() {
    let ring = TestRing::new();
    let fut = pin!(Submission::new(
        Write::new(&BadFd, b"keep".to_vec()),
        ring.clone()
    ));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Err(Errno::BADF));
    assert_eq!(buf, b"keep");
}

#[test]
fn write_closed_pipe() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    drop(rx);

    let fut = pin!(Submission::new(Write::new(&tx, "hello"), ring.clone()));
    let (_, res) = block_on(&ring, fut);

    assert_eq!(res, Err(Errno::PIPE));
}

#[test]
fn write_drop_unsubmitted() {
    let ring = TestRing::new();
    let (_rx, tx) = pipe::pipe().unwrap();
    drop(Submission::new(
        Write::new(&tx, b"abc".to_vec()),
        ring.clone(),
    ));
    drop(Submission::new(Write::new(&tx, String::from("abc")), ring));
}

/// Drops a write that reached the kernel, so the kernel may still be reading
/// from the buffer while the op is converted into a `Cancellation`.
fn drop_in_flight<B: IoBuf>(buf: B) {
    let ring = TestRing::new();
    let (_rx, tx) = pipe::pipe().unwrap();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut write = Box::pin(Submission::new(Write::new(&tx, buf), ring.clone()));
    assert!(write.as_mut().poll(&mut cx).is_pending());

    // enter the ring so the write reaches the kernel, without reaping it
    let nop = pin!(Submission::new(Nop, ring.clone()));
    assert_eq!(block_on(&ring, nop), 0);

    drop(write);
    ring.drain();
}

#[test]
fn write_static_drop_in_flight() {
    drop_in_flight(&[1u8; 64][..]);
}

#[test]
fn write_box_drop_in_flight() {
    drop_in_flight(vec![1u8; 64].into_boxed_slice());
}

#[test]
fn write_vec_drop_in_flight() {
    drop_in_flight(vec![1u8; 64]);
}

#[test]
fn write_string_drop_in_flight() {
    drop_in_flight(String::from("hello"));
}

#[test]
fn write_view_drop_in_flight() {
    drop_in_flight(vec![1u8; 128].view(32..96));
}

/// A file in the temp dir, removed on drop.
struct TempFile(PathBuf);

impl TempFile {
    fn new(name: &str, contents: &[u8]) -> (Self, File) {
        let path = std::env::temp_dir().join(format!("inel-{}-{name}", std::process::id()));
        fs::write(&path, contents).unwrap();
        let file = File::options().read(true).write(true).open(&path).unwrap();
        (Self(path), file)
    }

    fn contents(&self) -> Vec<u8> {
        fs::read(&self.0).unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
// opening files needs -Zmiri-disable-isolation, and even then the fake can't
// do writes at the current position on regular files: it waits for readiness
// through epoll, which miri doesn't support for files
#[cfg_attr(
    miri,
    ignore = "the fake can't write files at the current position under miri"
)]
fn write_at_offset() {
    let ring = TestRing::new();
    let (tmp, file) = TempFile::new("write_at_offset", b"0123456789");

    let fut = pin!(Submission::new(Write::new_at(&file, "ab", 3), ring.clone()));
    let (_, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(2));
    assert_eq!(tmp.contents(), b"012ab56789");

    // a positioned write doesn't move the file position
    let fut = pin!(Submission::new(Write::new(&file, "X"), ring.clone()));
    let (_, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(1));
    assert_eq!(tmp.contents(), b"X12ab56789");
}

#[test]
// see write_at_offset
#[cfg_attr(
    miri,
    ignore = "the fake can't write files at the current position under miri"
)]
fn write_current_advances() {
    let ring = TestRing::new();
    let (tmp, file) = TempFile::new("write_current_advances", b"0123456789");

    let fut = pin!(Submission::new(Write::new(&file, "ab"), ring.clone()));
    let (_, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(2));

    let fut = pin!(Submission::new(Write::new(&file, "cd"), ring.clone()));
    let (_, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(2));

    assert_eq!(tmp.contents(), b"abcd456789");
}
