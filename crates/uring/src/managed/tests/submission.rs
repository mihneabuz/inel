use core::pin::Pin;
use std::{pin::pin, task::Context};

use futures::{Stream, future::FusedFuture};

use crate::{
    managed::{
        UringProxy,
        cancellation::Cancellation,
        completion::CompletionResult,
        submission::{MultiOp, Op, SingleOp, Submission},
        tests::{DropFlag, Nop, TestUring, TestWaker, dropped, flag, ok, ok_more},
    },
    sys::Sqe,
};

/// Op that holds a resource moved into Cancellation on cancel.
struct OwnedOp {
    resource: Box<DropFlag>,
}

impl OwnedOp {
    fn new(df: DropFlag) -> Self {
        Self {
            resource: Box::new(df),
        }
    }
}

impl Op for OwnedOp {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_nop();
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.resource.into(), true)
    }
}

impl SingleOp for OwnedOp {
    type Output = ();
    fn result(self, _: CompletionResult) {}
}

impl MultiOp for OwnedOp {
    type Output = ();
    fn result(self: Pin<&mut Self>, _: CompletionResult) {}
}

#[test]
fn single_nop() {
    let ring = TestUring::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(Submission::new(Nop, ring.clone()));

    assert!(fut.as_mut().poll(&mut cx).is_pending());
    ring.wait();
    assert!(fut.as_mut().poll(&mut cx).is_ready());
}

#[test]
fn single_result_passed_through() {
    let ring = TestUring::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut fut = pin!(Submission::new(Nop, ring.clone()));

    assert!(fut.as_mut().poll(&mut cx).is_pending());
    ring.complete_next(ok(42));

    match fut.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(val) => assert_eq!(val, 42),
        std::task::Poll::Pending => panic!("expected Ready"),
    }
}

#[test]
fn single_fused_after_completion() {
    let ring = TestUring::new();
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
    let ring = TestUring::new();

    // Drop in Initial state — op dropped normally
    let fut = Submission::new(OwnedOp::new(df), ring.clone());
    drop(fut);
    assert!(dropped(&f));
}

#[test]
fn single_drop_while_submitted() {
    let (f, df) = flag();
    let ring = TestUring::new();
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

    // Kernel ack: wait() processes the SQE, Cancelled handler drops the handle
    ring.wait();
    assert!(dropped(&f));
}

#[test]
fn multi_nop() {
    let ring = TestUring::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut stream = pin!(Submission::new(Nop, ring.clone()));

    assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    ring.wait();
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        std::task::Poll::Ready(Some(_))
    ));
    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        std::task::Poll::Ready(None)
    ));
}

#[test]
fn multi_stream_items() {
    let ring = TestUring::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    let mut stream = pin!(Submission::new(Nop, ring.clone()));

    // First poll submits, returns Pending
    assert!(stream.as_mut().poll_next(&mut cx).is_pending());

    let key = ring.complete_next(ok_more(10));

    match stream.as_mut().poll_next(&mut cx) {
        std::task::Poll::Ready(Some(val)) => assert_eq!(val, 10),
        other => panic!("expected Ready(Some(10)), got {other:?}"),
    }

    ring.complete_key(key, ok_more(20));

    match stream.as_mut().poll_next(&mut cx) {
        std::task::Poll::Ready(Some(val)) => assert_eq!(val, 20),
        other => panic!("expected Ready(Some(20)), got {other:?}"),
    }

    // Final completion without MORE terminates the stream
    ring.complete_key(key, ok(30));

    match stream.as_mut().poll_next(&mut cx) {
        std::task::Poll::Ready(Some(val)) => assert_eq!(val, 30),
        other => panic!("expected Ready(Some(30)), got {other:?}"),
    }

    assert!(matches!(
        stream.as_mut().poll_next(&mut cx),
        std::task::Poll::Ready(None)
    ));
}

#[test]
fn multi_drop_while_streaming() {
    let (f, df) = flag();
    let ring = TestUring::new();
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);

    {
        let mut stream = pin!(Submission::new(OwnedOp::new(df), ring.clone()));
        let _ = stream.as_mut().poll_next(&mut cx);
        // Dropped in Submitted state
    }

    assert!(!dropped(&f));
    ring.wait();
    assert!(dropped(&f));
}
