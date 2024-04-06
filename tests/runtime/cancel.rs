use std::time::Duration;

use futures::{select, FutureExt};
use inel::AsyncRingRead;

use crate::helpers::*;

#[test]
fn block_on() {
    setup_tracing();

    let x = inel::block_on(async move {
        let mut stdin = std::io::stdin();

        let mut x = 0;

        for _ in 0..4 {
            let buf = vec![0u8; 128];
            select! {
                _ = stdin.ring_read(buf).fuse() => {
                    x = 1;
                },

                _ = inel::time::sleep(Duration::from_millis(1)).fuse() => {
                    x = 2
                }
            };
        }

        x
    });

    assert_eq!(x, 2);
}

#[test]
fn sleep() {
    setup_tracing();

    let x = inel::block_on(async move {
        let timeout = inel::time::Timeout::new(Duration::from_millis(10));
        let immediate = inel::time::immediate(());

        let x = select! {
            _ = timeout.fuse() => {
                2
            },

            _ = immediate.fuse() => {
                10
            }
        };

        inel::time::sleep(Duration::from_millis(20)).await;

        x
    });

    assert_eq!(x, 10);
}
