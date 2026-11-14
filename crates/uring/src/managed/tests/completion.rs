use crate::managed::{
    cancellation::Cancellation,
    completion::Completions,
    tests::{TestWaker, dropped, flag, ok, ok_more},
};

#[test]
fn single_submit_complete_result() {
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_single(tw.waker());
    assert!(comp.result(key).is_none());

    comp.notify(key, ok(42));
    assert!(tw.was_woken());

    let res = comp.result(key).expect("should have result");
    assert_eq!(res.result(), 42);
}

#[test]
fn single_waker_not_woken_before_completion() {
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_single(tw.waker());
    assert!(comp.result(key).is_none());
    assert!(!tw.was_woken());
}

#[test]
fn single_waker_update() {
    let mut comp = Completions::new(16);
    let tw1 = TestWaker::new();
    let tw2 = TestWaker::new();

    let key = comp.insert_single(tw1.waker());
    comp.update(key, &tw2.waker());
    comp.notify(key, ok(0));

    assert!(!tw1.was_woken());
    assert!(tw2.was_woken());
}

#[test]
fn multi_completion_stream() {
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_multi(tw.waker());

    comp.notify(key, ok_more(1));
    assert!(tw.was_woken());

    comp.notify(key, ok_more(2));
    assert!(tw.was_woken());

    comp.notify(key, ok(3));
    assert!(tw.was_woken());

    assert_eq!(comp.result(key).unwrap().result(), 1);
    assert_eq!(comp.result(key).unwrap().result(), 2);
    assert_eq!(comp.result(key).unwrap().result(), 3);
}

#[test]
fn cancel_pending_single() {
    let (f, tracked) = flag();
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_single(tw.waker());
    let handle: Cancellation = Box::new(tracked).into();

    let accepted = comp.cancel(key, handle, |_| {});
    assert!(accepted);
    assert!(!dropped(&f));

    comp.notify(key, ok(-125));
    assert!(dropped(&f));
}

#[test]
fn cancel_already_completed_single() {
    let (f, tracked) = flag();
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_single(tw.waker());
    comp.notify(key, ok(0));

    let handle: Cancellation = Box::new(tracked).into();
    let accepted = comp.cancel(key, handle, |_| {});
    assert!(!accepted);
    assert!(dropped(&f));
}

#[test]
fn cancel_pending_multi() {
    let (f, tracked) = flag();
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_multi(tw.waker());
    comp.notify(key, ok_more(1));

    let handle: Cancellation = Box::new(tracked).into();
    let accepted = comp.cancel(key, handle, |_| {});
    assert!(accepted);
    assert!(!dropped(&f));

    comp.notify(key, ok(-125));
    assert!(dropped(&f));
}

#[test]
fn keys_reused_after_completion() {
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let k0 = comp.insert_single(tw.waker());
    comp.notify(k0, ok(0));
    comp.result(k0);

    let k1 = comp.insert_single(tw.waker());
    assert_eq!(k0, k1);
}

#[test]
fn concurrent_single_ops() {
    let mut comp = Completions::new(16);
    let tw1 = TestWaker::new();
    let tw2 = TestWaker::new();
    let tw3 = TestWaker::new();

    let k0 = comp.insert_single(tw1.waker());
    let k1 = comp.insert_single(tw2.waker());
    let k2 = comp.insert_single(tw3.waker());

    comp.notify(k1, ok(20));
    assert!(!tw1.was_woken());
    assert!(tw2.was_woken());
    assert!(!tw3.was_woken());

    comp.notify(k2, ok(30));
    comp.notify(k0, ok(10));

    assert_eq!(comp.result(k0).unwrap().result(), 10);
    assert_eq!(comp.result(k1).unwrap().result(), 20);
    assert_eq!(comp.result(k2).unwrap().result(), 30);
}

#[test]
fn cancel_multi_waits_for_final_completion() {
    let (f, tracked) = flag();
    let mut comp = Completions::new(16);
    let tw = TestWaker::new();

    let key = comp.insert_multi(tw.waker());
    let handle: Cancellation = Box::new(tracked).into();
    assert!(comp.cancel(key, handle, |_| {}));

    // shots that fired before the cancel reached the kernel
    comp.notify(key, ok_more(1));
    comp.notify(key, ok_more(2));
    assert!(!dropped(&f));

    comp.notify(key, ok(-125));
    assert!(dropped(&f));
}
