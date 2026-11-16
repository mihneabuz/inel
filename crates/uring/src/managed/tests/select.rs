use std::{
    pin::{Pin, pin},
    task::Context,
    thread,
    time::Duration,
};

use rustix::{
    io::{self, Errno},
    pipe,
};

use crate::managed::{
    IoSrc, Op, ReadSelect, SelectBuf, SelectGroup, UringProxy,
    tests::{BadFd, Nop, TestRing, TestWaker, block_on},
};

fn read(ring: &TestRing, src: &impl IoSrc, group: &SelectGroup<TestRing>) -> ReadResult {
    let fut = pin!(ReadSelect::new(src, group).run_on(ring.clone()));
    block_on(ring, fut)
}

type ReadResult = Result<SelectBuf<TestRing>, Errno>;

/// Polls `fut` once, which submits it.
fn submit<F: Future>(fut: Pin<&mut F>) {
    let tw = TestWaker::new();
    let waker = tw.waker();
    assert!(fut.poll(&mut Context::from_waker(&waker)).is_pending());
}

#[test]
fn read_select() {
    let ring = TestRing::new();
    let group = SelectGroup::new(4, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"hello").unwrap();

    let buf = read(&ring, &rx, &group).unwrap();
    assert_eq!(&*buf, b"hello");
}

#[test]
fn waits_for_data() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        io::write(&tx, b"abc").unwrap();
    });

    let buf = read(&ring, &rx, &group).unwrap();
    writer.join().unwrap();
    assert_eq!(&*buf, b"abc");
}

#[test]
fn reads_at_most_one_buffer() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 4, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abcdef").unwrap();

    let first = read(&ring, &rx, &group).unwrap();
    let second = read(&ring, &rx, &group).unwrap();
    assert_eq!(&*first, b"abcd");
    assert_eq!(&*second, b"ef");
}

#[test]
fn eof_gives_empty_buffer() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    drop(tx);

    let buf = read(&ring, &rx, &group).unwrap();
    assert!(buf.is_empty());
}

#[test]
fn buffers_are_recycled() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    for i in 0..10 {
        io::write(&tx, &[i; 3]).unwrap();
        let buf = read(&ring, &rx, &group).unwrap();
        assert_eq!(&*buf, &[i; 3]);
        buf.release();
    }
}

#[test]
fn held_buffers_are_not_reused() {
    let ring = TestRing::new();
    let group = SelectGroup::new(4, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    let held = (0..4)
        .map(|i| {
            io::write(&tx, &[i; 8]).unwrap();
            read(&ring, &rx, &group).unwrap()
        })
        .collect::<Vec<_>>();

    for (i, buf) in (0..).zip(&held) {
        assert_eq!(&**buf, &[i; 8]);
    }
}

#[test]
fn concurrent_reads_get_different_buffers() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 16, ring.clone()).unwrap();
    let (rx1, tx1) = pipe::pipe().unwrap();
    let (rx2, tx2) = pipe::pipe().unwrap();

    let mut first = pin!(ReadSelect::new(&rx1, &group).run_on(ring.clone()));
    let mut second = pin!(ReadSelect::new(&rx2, &group).run_on(ring.clone()));
    submit(first.as_mut());
    submit(second.as_mut());

    io::write(&tx2, b"two").unwrap();
    io::write(&tx1, b"one").unwrap();

    let first = block_on(&ring, first).unwrap();
    let second = block_on(&ring, second).unwrap();
    assert_eq!(&*first, b"one");
    assert_eq!(&*second, b"two");
}

#[test]
fn two_groups() {
    let ring = TestRing::new();
    let small = SelectGroup::new(1, 2, ring.clone()).unwrap();
    let big = SelectGroup::new(1, 16, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abcdef").unwrap();

    assert_eq!(&*read(&ring, &rx, &small).unwrap(), b"ab");
    assert_eq!(&*read(&ring, &rx, &big).unwrap(), b"cdef");
}

#[test]
fn no_buffers() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"one").unwrap();

    let held = read(&ring, &rx, &group).unwrap();
    io::write(&tx, b"two").unwrap();
    assert_eq!(read(&ring, &rx, &group).err(), Some(Errno::NOBUFS));

    // nothing was read, so the data is still there once a buffer is back
    drop(held);
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"two");
}

#[test]
fn no_buffers_fails_without_waiting() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"one").unwrap();
    let _held = read(&ring, &rx, &group).unwrap();

    // the pipe is empty, but the kernel needs a buffer before it waits
    assert_eq!(read(&ring, &rx, &group).err(), Some(Errno::NOBUFS));
}

#[test]
fn bad_fd_keeps_buffer() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    assert_eq!(read(&ring, &BadFd, &group).err(), Some(Errno::BADF));

    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abc").unwrap();
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"abc");
}

#[test]
fn write_only_fd() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    assert_eq!(read(&ring, &tx, &group).err(), Some(Errno::BADF));

    io::write(&tx, b"abc").unwrap();
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"abc");
}

#[test]
// opening a directory needs -Zmiri-disable-isolation, and miri can't read it
#[cfg_attr(miri, ignore = "miri can't open directories")]
fn error_after_pick_recycles_buffer() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let dir = rustix::fd::OwnedFd::from(std::fs::File::open(".").unwrap());

    // the kernel picks the buffer before the read fails, and hands it back
    // with the error
    for _ in 0..3 {
        let fut = pin!(ReadSelect::new_at(&dir, &group, 0).run_on(ring.clone()));
        assert_eq!(block_on(&ring, fut).err(), Some(Errno::ISDIR));
    }
}

#[test]
fn drop_blocked() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    submit(pin!(ReadSelect::new(&rx, &group).run_on(ring.clone())));
    ring.drain();

    io::write(&tx, b"abc").unwrap();
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"abc");
}

#[test]
fn drop_in_flight() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 128, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, &[1; 64]).unwrap();

    let mut fut = Box::pin(ReadSelect::new(&rx, &group).run_on(ring.clone()));
    submit(fut.as_mut());

    // enter the ring so the read reaches the kernel, without reaping it
    let nop = pin!(Nop.run_on(ring.clone()));
    assert_eq!(block_on(&ring, nop), 0);

    drop(fut);
    ring.drain();

    // the read may or may not have happened, but the buffer is back
    io::write(&tx, &[2; 4]).unwrap();
    let buf = read(&ring, &rx, &group).unwrap();
    assert!(buf.ends_with(&[2; 4]));
}

#[test]
fn drop_completed_unread() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abc").unwrap();

    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);
    {
        let mut fut = pin!(ReadSelect::new(&rx, &group).run_on(ring.clone()));
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        while !tw.was_woken() {
            ring.wait();
        }
    }

    io::write(&tx, b"def").unwrap();
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"def");
}

#[test]
fn drop_completed_unread_last_handle() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abc").unwrap();

    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);
    {
        // the op holds the only handle, so dropping it frees the group while
        // its unread CQE still holds a buffer
        let mut fut = pin!(ReadSelect::new(&rx, &group).run_on(ring.clone()));
        drop(group);
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        while !tw.was_woken() {
            ring.wait();
        }
    }

    ring.drain();
}

#[test]
fn drop_unsubmitted() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, _tx) = pipe::pipe().unwrap();
    drop(ReadSelect::new(&rx, &group).run_on(ring));
}

#[test]
fn invalid_group() {
    let ring = TestRing::new();
    for (count, size) in [(0, 8), (1, 0), ((1 << 15) + 1, 8)] {
        let group = SelectGroup::new(count, size, ring.clone());
        assert_eq!(group.err(), Some(Errno::INVAL));
    }
}

#[test]
fn clone_outlives_original() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let other = group.clone();
    drop(group);

    let (rx, tx) = pipe::pipe().unwrap();
    for _ in 0..3 {
        io::write(&tx, b"abc").unwrap();
        assert_eq!(&*read(&ring, &rx, &other).unwrap(), b"abc");
    }
}

#[test]
fn buf_outlives_group() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();
    io::write(&tx, b"abc").unwrap();

    let buf = read(&ring, &rx, &group).unwrap();
    drop(group);
    assert_eq!(&*buf, b"abc");
    drop(buf);

    // the group was freed, and a new one can take its place
    let group = SelectGroup::new(2, 8, ring.clone()).unwrap();
    io::write(&tx, b"xyz").unwrap();
    assert_eq!(&*read(&ring, &rx, &group).unwrap(), b"xyz");
}

#[test]
fn op_outlives_group() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    // an op that was never submitted keeps the group alive too
    let op = ReadSelect::new(&rx, &group);
    drop(group);

    io::write(&tx, b"abc").unwrap();
    let fut = pin!(op.run_on(ring.clone()));
    assert_eq!(&*block_on(&ring, fut).unwrap(), b"abc");
}

#[test]
fn in_flight_op_outlives_group() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, tx) = pipe::pipe().unwrap();

    let mut fut = pin!(ReadSelect::new(&rx, &group).run_on(ring.clone()));
    submit(fut.as_mut());
    drop(group);

    io::write(&tx, b"abc").unwrap();
    let buf = block_on(&ring, fut).unwrap();
    assert_eq!(&*buf, b"abc");
}

#[test]
fn group_dropped_with_blocked_op() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let (rx, _tx) = pipe::pipe().unwrap();

    let mut fut = Box::pin(ReadSelect::new(&rx, &group).run_on(ring.clone()));
    submit(fut.as_mut());
    drop(group);
    drop(fut);
    ring.drain();
}

#[test]
fn cancelled_op_keeps_group_until_final_cqe() {
    let ring = TestRing::new();
    let group = SelectGroup::new(1, 8, ring.clone()).unwrap();
    let gid = group.id();
    let (rx, _tx) = pipe::pipe().unwrap();

    let mut fut = Box::pin(ReadSelect::new(&rx, &group).run_on(ring.clone()));
    submit(fut.as_mut());
    drop(group);
    drop(fut);

    // the cancellation still holds the last handle, so the slot is taken
    let other = SelectGroup::new(1, 8, ring.clone()).unwrap();
    assert_ne!(other.id(), gid);
    drop(other);

    // the final CQE drops the cancellation from inside `wait`, and the group
    // is freed at the end of it
    ring.drain();
    let reused = SelectGroup::new(1, 8, ring).unwrap();
    assert_eq!(reused.id(), gid);
}
