use crate::managed::{
    UringProxy,
    cancellation::Cancellation,
    tests::{dropped, flag},
};

use super::{TestUring, TestWaker, ok, ok_more};

#[test]
fn single_submit_complete_result() {
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let key = ring.nop_submit(tw.waker());
    assert!(!ring.is_done());
    assert!(ring.result(key).is_none());

    ring.nop_complete(key, ok(42));
    assert!(tw.was_woken());

    let res = ring.result(key).expect("should have result");
    assert_eq!(res.result(), 42);
    assert!(ring.is_done());
}

#[test]
fn single_waker_not_woken_before_completion() {
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    ring.nop_submit(tw.waker());
    assert!(!tw.was_woken());
}

#[test]
fn single_waker_update() {
    let mut ring = TestUring::new();
    let tw1 = TestWaker::new();
    let tw2 = TestWaker::new();

    let key = ring.nop_submit(tw1.waker());
    ring.update(key, tw2.waker());
    ring.nop_complete(key, ok(0));

    assert!(!tw1.was_woken());
    assert!(tw2.was_woken());
}

#[test]
fn multi_completion_stream() {
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let key = ring.nop_submit_multi(tw.waker());
    assert!(!ring.is_done());

    ring.nop_complete(key, ok_more(1));
    assert!(tw.was_woken());
    assert!(!ring.is_done());

    ring.nop_complete(key, ok_more(2));
    assert!(tw.was_woken());
    assert!(!ring.is_done());

    ring.nop_complete(key, ok(3));
    assert!(tw.was_woken());
    assert!(ring.is_done());

    assert_eq!(ring.result(key).unwrap().result(), 1);
    assert_eq!(ring.result(key).unwrap().result(), 2);
    assert_eq!(ring.result(key).unwrap().result(), 3);
}

#[test]
fn cancel_pending_single() {
    let (f, tracked) = flag();
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let key = ring.nop_submit(tw.waker());
    let handle: Cancellation = Box::new(tracked).into();

    let accepted = ring.nop_cancel(key, handle);
    assert!(accepted);
    assert!(!dropped(&f));

    ring.nop_complete(key, ok(-125));
    assert!(dropped(&f));
}

#[test]
fn cancel_already_completed_single() {
    let (f, tracked) = flag();
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let key = ring.nop_submit(tw.waker());
    ring.nop_complete(key, ok(0));

    let handle: Cancellation = Box::new(tracked).into();
    let accepted = ring.nop_cancel(key, handle);
    assert!(!accepted);
    assert!(dropped(&f));
}

#[test]
fn cancel_pending_multi() {
    let (f, tracked) = flag();
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let key = ring.nop_submit_multi(tw.waker());
    ring.nop_complete(key, ok_more(1));

    let handle: Cancellation = Box::new(tracked).into();
    let accepted = ring.nop_cancel(key, handle);
    assert!(accepted);
    assert!(!dropped(&f));

    ring.nop_complete(key, ok(-125));
    assert!(dropped(&f));
}

#[test]
fn keys_reused_after_completion() {
    let mut ring = TestUring::new();
    let tw = TestWaker::new();

    let k0 = ring.nop_submit(tw.waker());
    ring.nop_complete(k0, ok(0));
    ring.result(k0);

    let k1 = ring.nop_submit(tw.waker());
    assert_eq!(k0.as_u64(), k1.as_u64());
}

#[test]
fn concurrent_single_ops() {
    let mut ring = TestUring::new();
    let tw1 = TestWaker::new();
    let tw2 = TestWaker::new();
    let tw3 = TestWaker::new();

    let k0 = ring.nop_submit(tw1.waker());
    let k1 = ring.nop_submit(tw2.waker());
    let k2 = ring.nop_submit(tw3.waker());

    ring.nop_complete(k1, ok(20));
    assert!(!tw1.was_woken());
    assert!(tw2.was_woken());
    assert!(!tw3.was_woken());

    ring.nop_complete(k2, ok(30));
    ring.nop_complete(k0, ok(10));

    assert_eq!(ring.result(k0).unwrap().result(), 10);
    assert_eq!(ring.result(k1).unwrap().result(), 20);
    assert_eq!(ring.result(k2).unwrap().result(), 30);
    assert!(ring.is_done());
}
