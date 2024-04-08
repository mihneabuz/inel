use futures::{SinkExt, StreamExt};

use crate::helpers::*;

#[test]
fn block_on() {
    setup_tracing();

    let num = |a: i32| async move { a };
    let sum = |a: i32, b: i32| async move { a + b };

    let x = inel::block_on(async move {
        let a = num(1).await;
        let b = num(2).await;
        sum(a, b).await
    });

    assert_eq!(x, 3);
}

#[test]
fn mixed() {
    setup_tracing();

    let (value, location) = output(0);

    inel::spawn(async move {
        *location.borrow_mut() = 10;
    });

    let other = inel::block_on(async move { 20 });

    assert_eq!(value.take(), 10);
    assert_eq!(other, 20);
}

#[test]
fn spawn_before() {
    setup_tracing();

    let (x, x_location) = output(0);
    let (y, y_location) = output(0);

    inel::spawn(async move {
        *x_location.borrow_mut() = 1;
    });

    inel::spawn(async move {
        *y_location.borrow_mut() = 2;
    });

    inel::run();

    assert_eq!(x.take() + y.take(), 3);
}

#[test]
fn spawn_inside() {
    setup_tracing();

    let (x, x_location) = output(0);
    let (y, y_location) = output(0);

    inel::block_on(async {
        inel::spawn(async move {
            *x_location.borrow_mut() = 1;
        });

        inel::spawn(async move {
            *y_location.borrow_mut() = 2;
        });
    });

    assert_eq!(x.take() + y.take(), 3);
}

#[test]
fn join() {
    setup_tracing();

    let x = inel::block_on(async {
        let a = inel::spawn(async move { 1 });
        let b = inel::spawn(async move { 2 });

        a.join().await.unwrap() + b.join().await.unwrap()
    });

    assert_eq!(x, 3);
}

#[test]
fn channel() {
    setup_tracing();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    inel::block_on(async {
        inel::spawn(async move {
            sender.send(10).unwrap();
        });

        inel::spawn(async move {
            *location.borrow_mut() = receiver.await.unwrap();
        });
    });

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_oneshot() {
    setup_tracing();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    inel::block_on(async {
        inel::spawn(async move {
            sender.send(10).unwrap();
        });

        inel::spawn(async move {
            *location.borrow_mut() = receiver.await.unwrap();
        });
    });

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_mpsc() {
    setup_tracing();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);
    let mut sender_clone = sender.clone();

    let total = inel::block_on(async move {
        inel::spawn(async move {
            for i in 0..100 {
                sender.send(i).await.unwrap();
            }
        });

        inel::spawn(async move {
            for _ in 0..10 {
                sender_clone.send(10).await.unwrap();
            }
        });

        receiver.fold(0, |acc, x| async move { acc + x }).await
    });

    assert_eq!(total, 10 * 10 + (100 * 99) / 2);
}

#[test]
fn channel_join() {
    setup_tracing();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);

    let total = inel::block_on(async move {
        let sender_handle = inel::spawn(async move {
            for i in 0..5 {
                sender.send(i).await.unwrap();
            }
        });

        sender_handle.join().await;

        receiver.fold(0, |acc, x| async move { acc + x }).await
    });

    assert_eq!(total, 10);
}

#[test]
#[should_panic]
fn channel_deadlock() {
    setup_tracing();

    // let (mut sender, mut receiver) = futures::channel::mpsc::channel(5);
    // inel::block_on(async move {
    //     let sender_handle = inel::spawn(async move {
    //         for i in 0..10 {
    //             sender.send(i).await.unwrap();
    //         }
    //     });
    //
    //     sender_handle.join().await;
    //
    //     while let Some(value) = receiver.next().await {
    //         let _ = value;
    //     }
    //
    //     unreachable!();
    // });
}
