mod helpers;

use helpers::*;

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
