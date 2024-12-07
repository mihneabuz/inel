mod fs;
pub mod helpers;
mod io;

use std::sync::mpsc;

#[test]
fn sanity() {
    let (send, recv) = oneshot::channel::<i32>();

    inel::block_on(async move {
        send.send(10).unwrap();
    });

    assert_eq!(recv.recv(), Ok(10));
}

#[test]
fn spawn() {
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
    let start = std::time::Instant::now();

    inel::block_on(async move {
        inel::time::sleep(std::time::Duration::from_millis(10)).await;
    });

    let diff = std::time::Instant::now() - start;
    assert!(diff.as_millis() >= 10);
}

#[test]
fn main() {
    use core::sync::atomic::Ordering;
    use std::sync::atomic::AtomicBool;

    static CALLED: AtomicBool = AtomicBool::new(false);

    #[inel::main]
    async fn main() {
        inel::spawn(async {
            CALLED.swap(true, Ordering::SeqCst);
        });
    }

    main();

    assert!(CALLED.load(Ordering::SeqCst));
}
