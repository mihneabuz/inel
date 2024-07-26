mod join;
mod task;
mod waker;

use std::{future::Future, task::Context};

use futures::channel::oneshot;
use tracing::debug;

pub use join::JoinHandle;
use task::TaskQueue;
use waker::waker;

pub struct Executor {
    queue: TaskQueue,
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

impl Executor {
    pub fn new() -> Self {
        Self {
            queue: TaskQueue::new(),
        }
    }

    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        let (sender, receiver) = oneshot::channel();

        self.queue.schedule(async move {
            let _ = sender.send(future.await);
        });

        JoinHandle::new(receiver)
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        self.block_on_with_wait(future, || {})
    }

    pub fn block_on_with_wait<F, W>(&self, future: F, wait: W) -> F::Output
    where
        F: Future + 'static,
        W: Fn(),
    {
        let mut handle = self.spawn(future);

        self.run_with_wait(wait);

        handle
            .try_join()
            .expect("Failed to complete future. Deadlock maybe?")
    }

    pub fn run(&self) {
        self.run_with_wait(|| {})
    }

    pub fn run_with_wait<W>(&self, wait: W)
    where
        W: Fn(),
    {
        while !self.queue.is_done() {
            debug!("Executing tasks");
            for task in self.queue.drain() {
                let waker = waker(task.clone());
                let mut cx = Context::from_waker(&waker);
                let _ = task.poll(&mut cx);
            }
        }

        (wait)();
    }
}
