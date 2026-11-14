use std::{
    pin::{Pin, pin},
    task::Context,
};

use rustix::{io::Errno, pipe};

use crate::managed::{
    Read, ReadSelect, SelectGroup, UringProxy,
    submission::Submission,
    tests::{Nop, TestRing, TestWaker, block_on},
};

const ECANCELED: i32 = -Errno::CANCELED.raw_os_error();

/// Polls `fut` once, which submits it.
fn submit<F: Future>(fut: Pin<&mut F>) {
    let tw = TestWaker::new();
    let waker = tw.waker();
    assert!(fut.poll(&mut Context::from_waker(&waker)).is_pending());
}

#[test]
fn idle() {
    let ring = TestRing::new();
    ring.shutdown();
    assert!(ring.is_done());
}

#[test]
fn completes_awaited_ops() {
    let ring = TestRing::new();
    let mut fut = pin!(Submission::new(Nop, ring.clone()));
    submit(fut.as_mut());

    ring.shutdown();
    // the nop may have run before the cancel reached it
    assert!([0, ECANCELED].contains(&block_on(&ring, fut)));
}

#[test]
fn cancels_blocked_read() {
    let ring = TestRing::new();
    let (rx, _tx) = pipe::pipe().unwrap();
    let mut fut = pin!(Submission::new(Read::new(&rx, vec![0; 8]), ring.clone()));
    submit(fut.as_mut());

    ring.shutdown();
    let (_, res) = block_on(&ring, fut);
    assert_eq!(res, Err(Errno::CANCELED));
}

#[test]
fn waits_for_dropped_ops() {
    let ring = TestRing::new();
    let (rx, _tx) = pipe::pipe().unwrap();
    {
        let mut fut = pin!(Submission::new(Read::new(&rx, vec![0; 8]), ring.clone()));
        submit(fut.as_mut());
    }

    ring.shutdown();
    assert!(ring.is_done());
}

/// Under miri a leak fails the test, so this checks that after `shutdown`
/// dropping the ring frees the group.
#[test]
fn frees_groups_after_shutdown() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 8, ring.clone()).unwrap();
    let (rx, _tx) = pipe::pipe().unwrap();
    {
        let mut fut = pin!(Submission::new(ReadSelect::new(&rx, &group), ring.clone()));
        submit(fut.as_mut());
    }
    drop(group);

    ring.shutdown();
}

#[test]
#[cfg_attr(miri, ignore = "leaks the group on purpose")]
fn drop_without_shutdown_leaks_groups_in_use() {
    let ring = TestRing::new();
    let group = SelectGroup::new(2, 8, ring.clone()).unwrap();
    let (rx, _tx) = pipe::pipe().unwrap();
    {
        let mut fut = pin!(Submission::new(ReadSelect::new(&rx, &group), ring.clone()));
        submit(fut.as_mut());
    }
    drop(group);

    // the read is still in flight and may pick a buffer from the group
    drop(ring);
}
