use core::pin::Pin;
use std::{
    pin::pin,
    task::{Context, Poll},
};

use futures::{Stream, future::FusedFuture};
use rustix::{io::Errno, io_uring::IoringTimeoutFlags};

use crate::{
    managed::{
        UringProxy,
        submission::Submission,
        tests::{CancelAll, Nop, OwnedOp, TestRing, TestWaker, Timeout, block_on, dropped, flag},
    },
    sys::TIMEOUT_MULTISHOT,
};

const ETIME: i32 = -Errno::TIME.raw_os_error();
const ECANCELED: i32 = -Errno::CANCELED.raw_os_error();

fn collect<S: Stream>(ring: &TestRing, mut stream: Pin<&mut S>) -> Vec<S::Item> {
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);
    let mut items = Vec::new();
    loop {
        match stream.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(item)) => items.push(item),
            Poll::Ready(None) => return items,
            Poll::Pending => ring.wait(),
        }
    }
}

#[test]
fn single_nop() {
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(Submission::new(Nop, ring.clone()));

    assert!(fut.as_mut().poll(&mut cx).is_pending());
    ring.wait();
    assert!(tw.was_woken());
    assert_eq!(fut.as_mut().poll(&mut cx), Poll::Ready(0));
}

#[test]
fn single_timeout() {
    let ring = TestRing::new();
    let fut = pin!(Submission::new(
        Timeout::new(1, 0, IoringTimeoutFlags::empty()),
        ring.clone()
    ));
    assert_eq!(block_on(&ring, fut), ETIME);
}

#[test]
fn single_fused_after_completion() {
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(Submission::new(Nop, ring.clone()));

    assert!(!fut.is_terminated());
    let _ = fut.as_mut().poll(&mut cx);
    assert!(!fut.is_terminated());

    ring.wait();
    let _ = fut.as_mut().poll(&mut cx);
    assert!(fut.is_terminated());
}

#[test]
fn single_drop_before_poll() {
    let (f, df) = flag();
    let ring = TestRing::new();

    // Drop in Initial state — op dropped normally
    let fut = Submission::new(OwnedOp::new(df), ring.clone());
    drop(fut);
    assert!(dropped(&f));
}

#[test]
fn single_drop_while_submitted() {
    let (f, df) = flag();
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    {
        let mut fut = pin!(Submission::new(OwnedOp::new(df), ring.clone()));
        let _ = fut.as_mut().poll(&mut cx);
        // Dropped in Submitted state — Op::cancel moves resource into Cancellation
    }

    // Resource held in CompletionHandler::Cancelled
    assert!(!dropped(&f));

    // The op completes (or is cancelled), the Cancelled handler drops the handle
    ring.drain();
    assert!(dropped(&f));
}

#[test]
fn multi_nop() {
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut stream = pin!(Submission::new(Nop, ring.clone()));

    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    ring.wait();
    assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(Some(0)));
    assert_eq!(stream.as_mut().poll_next(&mut cx), Poll::Ready(None));
}

#[test]
fn multi_stream_items() {
    let ring = TestRing::new();
    let stream = pin!(Submission::new(
        Timeout::new(1, 3, TIMEOUT_MULTISHOT),
        ring.clone()
    ));

    assert_eq!(collect(&ring, stream), [ETIME, ETIME, ETIME]);
    assert!(ring.is_done());
}

#[test]
fn multi_drop_while_streaming() {
    let (f, df) = flag();
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    {
        let mut stream = pin!(Submission::new(OwnedOp::new(df), ring.clone()));
        let _ = stream.as_mut().poll_next(&mut cx);
        // Dropped in Submitted state
    }

    assert!(!dropped(&f));
    ring.drain();
    assert!(dropped(&f));
}

#[test]
fn multi_drop_infinite_stream() {
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    {
        let mut stream = pin!(Submission::new(
            Timeout::new(1, 0, TIMEOUT_MULTISHOT),
            ring.clone()
        ));

        let mut items = 0;
        while items < 2 {
            match stream.as_mut().poll_next(&mut cx) {
                Poll::Ready(Some(res)) => {
                    assert_eq!(res, ETIME);
                    items += 1;
                }
                Poll::Ready(None) => panic!("infinite stream ended"),
                Poll::Pending => ring.wait(),
            }
        }
    }

    // The stream only ends if the cancellation reached the kernel
    ring.drain();
}

#[test]
fn single_cancelled_result() {
    let ring = TestRing::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(Submission::new(
        Timeout::new(60_000, 0, IoringTimeoutFlags::empty()),
        ring.clone()
    ));
    assert!(fut.as_mut().poll(&mut cx).is_pending());

    // cancel everything from the outside
    ring.with(|ring| {
        ring.submit_detached(CancelAll);
    });

    assert_eq!(block_on(&ring, fut), ECANCELED);
}
