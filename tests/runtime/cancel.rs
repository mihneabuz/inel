use futures::{select, FutureExt};
use inel::{AsyncRingRead, AsyncRingWrite};

use crate::helpers::*;

#[test]
fn block_on() {
    setup_tracing();

    let x = inel::block_on(async move {
        let mut stdin = std::io::stdin();
        let mut stdout = std::io::stdout();

        let mut x = 0;

        for _ in 0..4 {
            let buf = vec![0u8; 128];
            select! {
                _ = stdin.ring_read(buf).fuse() => {
                    x = 1;
                },

                _ = stdout.ring_write(vec![b'a'; 4]).fuse() => {
                    x = 2
                }
            };
        }

        x
    });

    assert_eq!(x, 2);
}
