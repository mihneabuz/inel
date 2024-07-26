use std::{cell::RefCell, rc::Rc, sync::Once};

use futures::{SinkExt, StreamExt};
use inel_executor::Executor;

static TRACING: Once = Once::new();
pub fn setup_tracing() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    });
}

pub fn output<T>(init: T) -> (Rc<RefCell<T>>, Rc<RefCell<T>>) {
    let location = Rc::new(RefCell::new(init));
    (location.clone(), location)
}

#[test]
fn block_on() {
    setup_tracing();
    let exe = Executor::new();

    let num = |a: i32| async move { a };
    let sum = |a: i32, b: i32| async move { a + b };

    let x = exe.block_on(async move {
        let a = num(1).await;
        let b = num(2).await;
        sum(a, b).await
    });

    assert_eq!(x, 3);
}

#[test]
fn mixed() {
    setup_tracing();
    let exe = Executor::new();

    let (value, location) = output(0);

    exe.spawn(async move {
        *location.borrow_mut() = 10;
    });

    let other = exe.block_on(async move { 20 });

    assert_eq!(value.take(), 10);
    assert_eq!(other, 20);
}

#[test]
fn spawn_before() {
    setup_tracing();
    let exe = Executor::new();

    let (x, x_location) = output(0);
    let (y, y_location) = output(0);

    exe.spawn(async move {
        *x_location.borrow_mut() = 1;
    });

    exe.spawn(async move {
        *y_location.borrow_mut() = 2;
    });

    exe.run();

    assert_eq!(x.take() + y.take(), 3);
}

#[test]
fn channel() {
    setup_tracing();
    let exe = Executor::new();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    exe.spawn(async move {
        sender.send(10).unwrap();
    });

    exe.spawn(async move {
        *location.borrow_mut() = receiver.await.unwrap();
    });

    exe.run();

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_oneshot() {
    setup_tracing();
    let exe = Executor::new();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    exe.spawn(async move {
        sender.send(10).unwrap();
    });

    exe.spawn(async move {
        *location.borrow_mut() = receiver.await.unwrap();
    });

    exe.run();

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_mpsc() {
    setup_tracing();
    let exe = Executor::new();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);
    let mut sender_clone = sender.clone();

    exe.spawn(async move {
        for i in 0..100 {
            sender.send(i).await.unwrap();
        }
    });

    exe.spawn(async move {
        for _ in 0..10 {
            sender_clone.send(10).await.unwrap();
        }
    });

    let total =
        exe.block_on(async move { receiver.fold(0, |acc, x| async move { acc + x }).await });

    assert_eq!(total, 10 * 10 + (100 * 99) / 2);
}

#[test]
fn channel_join() {
    setup_tracing();
    let exe = Executor::new();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);

    let sender_handle = exe.spawn(async move {
        for i in 0..5 {
            sender.send(i).await.unwrap();
        }
    });

    let total = exe.block_on(async move {
        sender_handle.join().await;
        receiver.fold(0, |acc, x| async move { acc + x }).await
    });

    assert_eq!(total, 10);
}
