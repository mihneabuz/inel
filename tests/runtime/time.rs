use std::{cell::RefCell, time::Duration};

use futures::StreamExt;

#[test]
fn immediate() {
    let x = inel::block_on(async { inel::time::immediate(10).await });

    assert_eq!(x, 10)
}

#[test]
fn timeout() {
    let x = inel::block_on(async {
        let x = 5;

        inel::time::sleep(Duration::from_millis(5)).await;
        inel::time::sleep(Duration::from_millis(10)).await;
        inel::time::sleep(Duration::from_millis(1)).await;

        x + 5
    });

    assert_eq!(x, 10)
}

#[test]
fn interval() {
    let x = inel::block_on(async {
        let interval = inel::time::interval(Duration::from_millis(10));

        let x = RefCell::new(0);

        interval
            .take(10)
            .enumerate()
            .map(|(i, _)| i)
            .for_each(|_| async {
                *x.borrow_mut() += 1;
            })
            .await;

        x.take()
    });

    assert_eq!(x, 10)
}
