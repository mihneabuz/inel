use std::{pin::pin, task::Context, thread, time::Duration};

use rustix::{
    io::{self, Errno},
    pipe,
};

use crate::managed::{
    Accumulating, IoBuf, IoBufMut,
    ops::Read,
    submission::Submission,
    tests::{BadFd, Nop, TestRing, TestWaker, block_on},
};

fn boxed(len: usize) -> Box<[u8]> {
    vec![0u8; len].into_boxed_slice()
}

#[test]
fn read_box() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello").unwrap();

    let fut = pin!(Submission::new(Read::new(&rx, boxed(16)), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Ok(5));
    assert_eq!(&buf[..5], b"hello");
}

#[test]
fn read_box_waits_for_data() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();

    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        io::write(&tx, b"abc").unwrap();
    });

    let fut = pin!(Submission::new(Read::new(&rx, boxed(16)), ring.clone()));
    let (buf, res) = block_on(&ring, fut);
    writer.join().unwrap();

    assert_eq!(res, Ok(3));
    assert_eq!(&buf[..3], b"abc");
}

#[test]
fn read_box_bad_fd() {
    let ring = TestRing::new();
    let fut = pin!(Submission::new(Read::new(&BadFd, boxed(16)), ring.clone()));
    let (_, res) = block_on(&ring, fut);

    assert_eq!(res, Err(Errno::BADF));
}

#[test]
fn read_vec_error_keeps_contents() {
    let ring = TestRing::new();

    let fut = pin!(Submission::new(
        Read::new(&BadFd, b"keep".to_vec()),
        ring.clone()
    ));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Err(Errno::BADF));
    assert_eq!(buf, b"keep");
}

#[test]
fn read_box_drop_blocked() {
    let ring = TestRing::new();
    let (rx, _tx) = pipe::pipe().unwrap();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    {
        let mut fut = pin!(Submission::new(Read::new(&rx, boxed(16)), ring.clone()));
        assert!(fut.as_mut().poll(&mut cx).is_pending());
    }

    // only returns once the cancellation reached the kernel
    ring.drain();
}

/// Drops a read that reached the kernel while data is available, so the
/// kernel may still be writing into the buffer while the op is converted
/// into a `Cancellation`.
fn drop_in_flight<B: IoBufMut>(buf: B) {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    io::write(&tx, &[1; 64]).unwrap();
    let mut read = Box::pin(Submission::new(Read::new(&rx, buf), ring.clone()));
    assert!(read.as_mut().poll(&mut cx).is_pending());

    // enter the ring so the read reaches the kernel, without reaping it
    let nop = pin!(Submission::new(Nop, ring.clone()));
    assert_eq!(block_on(&ring, nop), 0);

    drop(read);
    ring.drain();
}

#[test]
fn read_box_drop_in_flight() {
    drop_in_flight(boxed(64));
}

#[test]
fn read_vec_drop_in_flight() {
    drop_in_flight(Vec::<u8>::with_capacity(64));
}

#[test]
fn read_view_drop_in_flight() {
    drop_in_flight(boxed(128).view(32..96));
}

#[test]
fn read_drop_unsubmitted() {
    let ring = TestRing::new();
    let (rx, _tx) = pipe::pipe().unwrap();
    drop(Submission::new(Read::new(&rx, boxed(16)), ring.clone()));
    drop(Submission::new(Read::new(&rx, vec![1, 2, 3]), ring));
}

#[test]
fn read_vec_replaces_contents() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello").unwrap();

    let mut buf = Vec::with_capacity(16);
    buf.extend_from_slice(b">> ");

    let fut = pin!(Submission::new(Read::new(&rx, buf), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Ok(5));
    assert_eq!(buf, b"hello");
    assert_eq!(buf.capacity(), 16);
}

#[test]
fn read_accumulating_appends() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello").unwrap();

    let mut buf = Accumulating::with_capacity(16);
    buf.extend_from_slice(b">> ");

    let fut = pin!(Submission::new(Read::new(&rx, buf), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Ok(5));
    assert_eq!(*buf, b">> hello");
    assert_eq!(buf.capacity(), 16);
}

#[test]
fn read_accumulating_full_reads_nothing() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello").unwrap();

    let buf = Accumulating::new(vec![7u8; 4]);
    let fut = pin!(Submission::new(Read::new(&rx, buf), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Ok(0));
    assert_eq!(*buf, [7; 4]);
}

#[test]
// opening files needs -Zmiri-disable-isolation, and even then the fake can't
// do reads at the current position on regular files: it waits for readiness
// through epoll, which miri doesn't support for files
#[cfg_attr(
    miri,
    ignore = "the fake can't read files at the current position under miri"
)]
fn read_at_offset() {
    let ring = TestRing::new();
    let file = std::fs::File::open("Cargo.toml").unwrap();

    let fut = pin!(Submission::new(
        Read::new_at(&file, boxed(8), 1),
        ring.clone()
    ));
    let (buf, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(8));
    assert_eq!(&*buf, b"package]");

    // a positioned read doesn't move the file position
    let fut = pin!(Submission::new(Read::new(&file, boxed(9)), ring.clone()));
    let (buf, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(9));
    assert_eq!(&*buf, b"[package]");
}

#[test]
// opening files needs -Zmiri-disable-isolation, and even then the fake can't
// do reads at the current position on regular files: it waits for readiness
// through epoll, which miri doesn't support for files
#[cfg_attr(
    miri,
    ignore = "the fake can't read files at the current position under miri"
)]
fn read_current_advances() {
    let ring = TestRing::new();
    let file = std::fs::File::open("Cargo.toml").unwrap();

    let fut = pin!(Submission::new(Read::new(&file, boxed(1)), ring.clone()));
    let (buf, _) = block_on(&ring, fut);
    assert_eq!(&*buf, b"[");

    let fut = pin!(Submission::new(Read::new(&file, boxed(8)), ring.clone()));
    let (buf, res) = block_on(&ring, fut);
    assert_eq!(res, Ok(8));
    assert_eq!(&*buf, b"package]");
}

#[test]
fn read_into_box_view() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello world").unwrap();

    let buf = vec![b'.'; 8].into_boxed_slice().view(2..6);
    let fut = pin!(Submission::new(Read::new(&rx, buf), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    assert_eq!(res, Ok(4));
    assert_eq!((buf.begin(), buf.end()), (2, 6));
    assert_eq!(&*buf.into_inner(), b"..hell..");
}

#[test]
fn read_into_vec_view_overwrites() {
    let ring = TestRing::new();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abc").unwrap();

    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(b"0123456789");

    let fut = pin!(Submission::new(Read::new(&rx, buf.view(4..)), ring.clone()));
    let (buf, res) = block_on(&ring, fut);

    // overwritten in place, the length doesn't change
    assert_eq!(res, Ok(3));
    assert_eq!(buf.into_inner(), b"0123abc789");
}
