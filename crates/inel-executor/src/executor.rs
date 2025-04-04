use std::{future::Future, task::Context};

use futures::{channel::oneshot, FutureExt};
use inel_interface::Reactor;
use tracing::debug;

use task::TaskQueue;
use waker::waker;

use crate::{join::JoinHandle, task, waker};

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

        let task = future
            .then(|res| async {
                let _ = sender.send(res);
            })
            .fuse();

        self.queue.schedule(task);

        JoinHandle::new(receiver)
    }

    pub fn block_on<R, F>(&self, reactor: R, future: F) -> F::Output
    where
        F: Future + 'static,
        R: Reactor,
    {
        let mut handle = self.spawn(future);

        self.run(reactor);

        handle
            .try_join()
            .expect("Failed to complete future. Deadlock maybe?")
    }

    pub fn run<R>(&self, reactor: R)
    where
        R: Reactor,
    {
        while !self.queue.is_done() {
            debug!("Executing tasks");
            for task in self.queue.drain() {
                let waker = waker(task.clone());
                let mut cx = Context::from_waker(&waker);
                let _ = task.poll(&mut cx);
            }

            reactor.wait();
        }
    }
}
