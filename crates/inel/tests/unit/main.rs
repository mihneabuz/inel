mod fs;
mod io;
mod net;

#[cfg(feature = "compat")]
mod compat;

mod helpers;

use std::sync::mpsc;

use futures::future::FusedFuture;
use helpers::setup_tracing;
use inel_reactor::RingOptions;

#[test]
fn sanity() {
    setup_tracing();

    let (send, recv) = mpsc::channel::<i32>();

    inel::block_on(async move {
        send.send(10).unwrap();
    });

    assert_eq!(recv.recv(), Ok(10));
}

#[test]
fn spawn() {
    setup_tracing();

    let (send, recv) = mpsc::channel::<i32>();

    for i in 0..10 {
        let send = send.clone();
        inel::spawn(async move {
            send.send(i).unwrap();
        });
    }

    inel::run();

    assert_eq!(recv.try_iter().sum::<i32>(), 45);
}

#[test]
fn combo() {
    setup_tracing();

    let (send, recv) = mpsc::channel();

    inel::block_on(async move {
        for i in 0..10 {
            let send = send.clone();
            inel::spawn(async move {
                send.send(i).unwrap();
            });
        }
    });

    assert_eq!(recv.try_iter().sum::<i32>(), 45);
}

#[test]
fn sleep() {
    setup_tracing();

    let start = std::time::Instant::now();

    inel::block_on(async move {
        inel::time::sleep(std::time::Duration::from_millis(10)).await;
    });

    let diff = std::time::Instant::now() - start;
    assert!(diff.as_millis() >= 10);
}

#[test]
fn init() {
    setup_tracing();

    inel::init(
        RingOptions::default()
            .submissions(128)
            .fixed_buffers(0)
            .auto_direct_files(512)
            .manual_direct_files(0),
    );

    assert!(inel::is_done());

    inel::block_on(async {
        inel::time::instant().await;
    });

    assert!(inel::is_done());
}

#[test]
fn main() {
    use core::sync::atomic::Ordering;
    use std::sync::atomic::AtomicBool;

    static CALLED: AtomicBool = AtomicBool::new(false);

    setup_tracing();

    #[inel::main]
    async fn main() {
        inel::spawn(async {
            CALLED.swap(true, Ordering::SeqCst);
        });
    }

    main();

    assert!(CALLED.load(Ordering::SeqCst));
}

#[test]
fn select() {
    setup_tracing();

    inel::block_on(async {
        let mut sleep = inel::time::sleep(std::time::Duration::from_millis(10));

        let canceled = futures::select! {
            () = sleep => {
                false
            },

            () = inel::time::instant() => {
                assert!(!sleep.is_terminated());
                true
            }
        };

        assert!(canceled);
    });
}
