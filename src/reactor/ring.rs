use std::task::{Context, Waker};

use io_uring::{opcode, squeue::Entry, IoUring};
use oneshot::Sender;
use tracing::debug;

const CANCEL_KEY: u64 = 420;

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
    active: u64,
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
        let canceled = self.recv_cancels();

        if self.active - canceled == 0 {
            return;
        }

        self.ring
            .submit_and_wait(canceled as usize * 2 + 1)
            .unwrap();

        for entry in self.ring.completion() {
            debug!(?entry, "Ring completion");

            if entry.user_data() == CANCEL_KEY {
                continue;
            }

            self.active -= 1;

            let (key, result) = (entry.user_data(), entry.result());
            let completion = unsafe { Box::from_raw(key as *mut CompletionNotifier) };
            completion.notify(result);
        }
    }

    fn recv_cancels(&mut self) -> u64 {
        let mut count = 0;

        while let Ok(key) = self.cancel_receiver.try_recv() {
            debug!(?key, "Cancel");

            let cancel = opcode::AsyncCancel::new(key).build().user_data(CANCEL_KEY);
            unsafe {
                self.ring.submission().push(&cancel).unwrap();
            }

            count += 1;
        }

        count
    }
}
