use std::{
    cmp::min,
    task::{Context, Waker},
};

use io_uring::{opcode, squeue::Entry, IoUring};
use oneshot::Sender;
use tracing::debug;

const CANCEL: u64 = 420;

struct CompletionNotifier {
    waker: Waker,
    sender: Sender<i32>,
}

impl CompletionNotifier {
    fn notify(self, result: i32) {
        if self.sender.send(result).is_ok() {
            self.waker.wake();
        }
    }
}

pub struct CancelHandle {
    key: u64,
    sender: flume::Sender<u64>,
}

impl CancelHandle {
    pub fn cancel(self) {
        self.sender.send(self.key).unwrap();
    }
}

pub struct Reactor {
    active: usize,
    ring: IoUring,
    cancel_sender: flume::Sender<u64>,
    cancel_receiver: flume::Receiver<u64>,
}

impl Default for Reactor {
    fn default() -> Self {
        let (sender, receiver) = flume::unbounded();

        Self {
            active: 0,
            ring: IoUring::new(128).unwrap(),
            cancel_sender: sender,
            cancel_receiver: receiver,
        }
    }
}

impl Reactor {
    pub fn submit(&mut self, entry: Entry, sender: Sender<i32>, cx: &mut Context) -> CancelHandle {
        let notifier = Box::new(CompletionNotifier {
            waker: cx.waker().clone(),
            sender,
        });

        let key = Box::into_raw(notifier) as u64;
        let entry = entry.user_data(key);

        debug!(?entry, "Ring submission");
        self.active += 1;

        unsafe {
            self.ring.submission().push(&entry).unwrap();
        };

        CancelHandle {
            key,
            sender: self.cancel_sender.clone(),
        }
    }

    pub fn wait(&mut self) {
        if self.active == 0 {
            return;
        }

        let canceled = self.recv_cancels();
        let count = min(2 * canceled + 1, canceled + self.active);
        self.ring.submit_and_wait(count).unwrap();

        for entry in self.ring.completion().filter(|e| e.user_data() != CANCEL) {
            debug!(?entry, "Ring completion");

            let (key, result) = (entry.user_data(), entry.result());
            let completion = unsafe { Box::from_raw(key as *mut CompletionNotifier) };
            completion.notify(result);

            self.active -= 1;
        }
    }

    fn recv_cancels(&mut self) -> usize {
        self.cancel_receiver
            .try_iter()
            .map(|key| {
                debug!(?key, "Cancel");

                let cancel = opcode::AsyncCancel::new(key).build().user_data(CANCEL);
                unsafe {
                    self.ring.submission().push(&cancel).unwrap();
                }
            })
            .count()
    }
}
