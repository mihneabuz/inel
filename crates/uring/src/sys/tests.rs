//! Raw ring tests. These run against the real kernel and against the fake
//! (under miri), which also checks that the fake behaves like the kernel.

use std::{thread, time::Duration};

use rustix::{
    fd::{AsRawFd, OwnedFd},
    io::{self, Errno},
    io_uring::*,
    pipe,
};

use super::{IoUring, Sqe, TIMEOUT_MULTISHOT};

struct Completion {
    user_data: u64,
    res: i32,
    flags: IoringCqeFlags,
}

impl Completion {
    const fn more(&self) -> bool {
        self.flags.contains(IoringCqeFlags::MORE)
    }
}

fn ring() -> IoUring {
    IoUring::options()
        .with_sq_entries(8)
        .with_cq_entries(16)
        .build()
        .unwrap()
}

fn push(ring: &mut IoUring, build: impl FnOnce(&mut Sqe)) {
    unsafe { ring.push_sqe(build) }.unwrap();
}

/// Submits pending SQEs and waits until exactly `count` completions are reaped.
fn reap(ring: &mut IoUring, count: usize) -> Vec<Completion> {
    let out = reap_at_least(ring, count);
    assert_eq!(out.len(), count, "more completions than expected");
    out
}

fn reap_at_least(ring: &mut IoUring, count: usize) -> Vec<Completion> {
    let mut out = Vec::new();
    while out.len() < count {
        ring.submit_and_wait(1).unwrap();
        ring.for_each_cqe(|cqe| {
            out.push(Completion {
                user_data: cqe.user_data(),
                res: cqe.result(),
                flags: cqe.flags(),
            });
        });
    }
    out
}

fn find(cqes: &[Completion], user_data: u64) -> &Completion {
    cqes.iter()
        .find(|cqe| cqe.user_data == user_data)
        .unwrap_or_else(|| panic!("no completion for {user_data}"))
}

const fn err(errno: Errno) -> i32 {
    -errno.raw_os_error()
}

const fn ts(ms: u64) -> Timespec {
    Timespec {
        tv_sec: (ms / 1000) as _,
        tv_nsec: ((ms % 1000) * 1_000_000) as _,
    }
}

fn pipe() -> (OwnedFd, OwnedFd) {
    pipe::pipe().unwrap()
}

#[test]
fn setup_rounds_entries() {
    let ring = IoUring::options()
        .with_sq_entries(5)
        .with_cq_entries(9)
        .build()
        .unwrap();
    assert_eq!(ring.sq_size(), 8);
    assert_eq!(ring.cq_size(), 16);
}

#[test]
fn nops_complete_with_user_data() {
    let mut ring = ring();
    for i in 1..=4 {
        push(&mut ring, |sqe| {
            sqe.prep_nop();
            sqe.user_data(i);
        });
    }

    let cqes = reap(&mut ring, 4);
    let mut seen = cqes.iter().map(|cqe| cqe.user_data).collect::<Vec<_>>();
    seen.sort_unstable();
    assert_eq!(seen, [1, 2, 3, 4]);
    assert!(cqes.iter().all(|cqe| cqe.res == 0 && !cqe.more()));
}

#[test]
fn sq_full() {
    let mut ring = ring();
    for _ in 0..8 {
        push(&mut ring, Sqe::prep_nop);
    }
    assert_eq!(
        unsafe { ring.push_sqe(Sqe::prep_nop) }.unwrap_err(),
        Errno::NOBUFS
    );

    reap(&mut ring, 8);
    push(&mut ring, Sqe::prep_nop);
    reap(&mut ring, 1);
}

#[test]
fn cq_overflow_is_not_dropped() {
    let mut ring = ring();

    // 24 completions for a CQ of 16, nothing reaped in between
    for batch in 0..3 {
        for i in 0..8 {
            push(&mut ring, |sqe| {
                sqe.prep_nop();
                sqe.user_data(batch * 8 + i);
            });
        }
        ring.submit().unwrap();
    }

    let cqes = reap(&mut ring, 24);
    let mut seen = cqes.iter().map(|cqe| cqe.user_data).collect::<Vec<_>>();
    seen.sort_unstable();
    assert_eq!(seen, (0..24).collect::<Vec<_>>());
}

#[test]
fn skip_success() {
    let mut ring = ring();
    push(&mut ring, |sqe| {
        sqe.prep_nop();
        sqe.skip_success_cqe();
        sqe.user_data(1);
    });
    push(&mut ring, |sqe| {
        sqe.prep_nop();
        sqe.user_data(2);
    });
    let timeout = ts(5);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 0, IoringTimeoutFlags::empty());
        sqe.user_data(3);
    });

    let cqes = reap(&mut ring, 2);
    assert!(cqes.iter().all(|cqe| cqe.user_data != 1));
}

#[test]
fn timeout_expires() {
    let mut ring = ring();
    let timeout = ts(1);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 0, IoringTimeoutFlags::empty());
        sqe.user_data(7);
    });

    let cqes = reap(&mut ring, 1);
    assert_eq!(cqes[0].user_data, 7);
    assert_eq!(cqes[0].res, err(Errno::TIME));
}

#[test]
fn timeout_completion_count() {
    let mut ring = ring();
    let timeout = ts(60_000);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 2, IoringTimeoutFlags::empty());
        sqe.user_data(1);
    });
    for i in 2..=3 {
        push(&mut ring, |sqe| {
            sqe.prep_nop();
            sqe.user_data(i);
        });
    }

    let cqes = reap(&mut ring, 3);
    assert_eq!(find(&cqes, 1).res, 0);
}

#[test]
fn timeout_multishot() {
    let mut ring = ring();
    let timeout = ts(1);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 3, TIMEOUT_MULTISHOT);
        sqe.user_data(1);
    });

    let cqes = reap(&mut ring, 3);
    assert!(cqes.iter().all(|cqe| cqe.res == err(Errno::TIME)));
    let more = cqes.iter().map(Completion::more).collect::<Vec<_>>();
    assert_eq!(more, [true, true, false]);
}

#[test]
fn cancel_timeout() {
    let mut ring = ring();
    let timeout = ts(60_000);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 0, IoringTimeoutFlags::empty());
        sqe.user_data(1);
    });
    push(&mut ring, |sqe| {
        sqe.prep_cancel(1, IoringAsyncCancelFlags::empty());
        sqe.user_data(2);
    });

    let cqes = reap(&mut ring, 2);
    assert_eq!(find(&cqes, 1).res, err(Errno::CANCELED));
    assert_eq!(find(&cqes, 2).res, 0);
}

#[test]
fn cancel_multishot_timeout() {
    let mut ring = ring();
    let timeout = ts(1);
    push(&mut ring, |sqe| {
        sqe.prep_timeout(&timeout, 0, TIMEOUT_MULTISHOT);
        sqe.user_data(1);
    });

    let cqes = reap_at_least(&mut ring, 2);
    assert!(cqes.iter().all(Completion::more));

    push(&mut ring, |sqe| {
        sqe.prep_cancel(1, IoringAsyncCancelFlags::empty());
        sqe.user_data(2);
    });

    // more shots may have fired before the cancel
    let mut done = false;
    let mut cancel = None;
    while !done || cancel.is_none() {
        for cqe in reap_at_least(&mut ring, 1) {
            match cqe.user_data {
                1 if cqe.more() => assert!(!done),
                1 => {
                    assert_eq!(cqe.res, err(Errno::CANCELED));
                    done = true;
                }
                _ => cancel = Some(cqe.res),
            }
        }
    }
    assert_eq!(cancel, Some(0));
}

#[test]
fn cancel_missing() {
    let mut ring = ring();
    push(&mut ring, |sqe| {
        sqe.prep_cancel(42, IoringAsyncCancelFlags::empty());
        sqe.user_data(1);
    });

    let cqes = reap(&mut ring, 1);
    assert_eq!(cqes[0].res, err(Errno::NOENT));
}

#[test]
fn read_pipe() {
    let mut ring = ring();
    let (rx, tx) = pipe();
    io::write(&tx, b"hello").unwrap();

    let mut buf = [0u8; 16];
    push(&mut ring, |sqe| {
        sqe.prep_read(rx.as_raw_fd(), buf.as_mut_ptr(), buf.len() as u32, u64::MAX);
    });

    let cqes = reap(&mut ring, 1);
    assert_eq!(cqes[0].res, 5);
    assert_eq!(&buf[..5], b"hello");
}

#[test]
fn read_pipe_waits_for_data() {
    let mut ring = ring();
    let (rx, tx) = pipe();

    let mut buf = [0u8; 16];
    push(&mut ring, |sqe| {
        sqe.prep_read(rx.as_raw_fd(), buf.as_mut_ptr(), buf.len() as u32, u64::MAX);
    });
    ring.submit().unwrap();

    let writer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(5));
        io::write(&tx, b"abc").unwrap();
    });

    let cqes = reap(&mut ring, 1);
    writer.join().unwrap();
    assert_eq!(cqes[0].res, 3);
    assert_eq!(&buf[..3], b"abc");
}

#[test]
fn write_pipe() {
    let mut ring = ring();
    let (rx, tx) = pipe();

    let data = *b"world";
    push(&mut ring, |sqe| {
        sqe.prep_write(tx.as_raw_fd(), data.as_ptr(), data.len() as u32, u64::MAX);
    });

    let cqes = reap(&mut ring, 1);
    assert_eq!(cqes[0].res, 5);

    let mut buf = [0u8; 5];
    io::read(&rx, &mut buf).unwrap();
    assert_eq!(&buf, b"world");
}

#[test]
fn read_bad_fd() {
    let mut ring = ring();
    let mut buf = [0u8; 4];
    push(&mut ring, |sqe| {
        sqe.prep_read(-1, buf.as_mut_ptr(), buf.len() as u32, u64::MAX);
    });

    let cqes = reap(&mut ring, 1);
    assert_eq!(cqes[0].res, err(Errno::BADF));
}

#[test]
fn cancel_blocked_read() {
    let mut ring = ring();
    let (rx, _tx) = pipe();

    let mut buf = [0u8; 4];
    push(&mut ring, |sqe| {
        sqe.prep_read(rx.as_raw_fd(), buf.as_mut_ptr(), buf.len() as u32, u64::MAX);
        sqe.user_data(1);
    });
    ring.submit().unwrap();

    push(&mut ring, |sqe| {
        sqe.prep_cancel(1, IoringAsyncCancelFlags::empty());
        sqe.user_data(2);
    });

    let cqes = reap(&mut ring, 2);
    assert_eq!(find(&cqes, 1).res, err(Errno::CANCELED));
    // the read may have been picked up already, it still gets cancelled
    // because it can't make progress
    assert!([0, err(Errno::ALREADY)].contains(&find(&cqes, 2).res));
}

#[test]
#[cfg_attr(
    miri,
    ignore = "needs -Zmiri-disable-isolation, run with --include-ignored"
)]
fn openat_and_pread() {
    let mut ring = ring();
    let path = c"Cargo.toml";
    push(&mut ring, |sqe| {
        sqe.prep_openat(
            rustix::fs::CWD.as_raw_fd(),
            path.as_ptr(),
            OFlags::RDONLY,
            0,
        );
    });

    let cqes = reap(&mut ring, 1);
    let fd = cqes[0].res;
    assert!(fd >= 0, "openat failed: {fd}");

    let mut buf = [0u8; 9];
    push(&mut ring, |sqe| {
        sqe.prep_read(fd, buf.as_mut_ptr(), buf.len() as u32, 0);
    });
    // reads may be short (miri does it on purpose)
    let n = reap(&mut ring, 1)[0].res;
    assert!(n > 0, "read failed: {n}");
    assert_eq!(&buf[..n as usize], &b"[package]"[..n as usize]);

    drop(unsafe { <OwnedFd as rustix::fd::FromRawFd>::from_raw_fd(fd) });
}
