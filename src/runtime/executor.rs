use std::{cell::RefCell, future::Future, task::Context};

use futures::channel::oneshot;
use tracing::debug;

use crate::{
    reactor,
    runtime::{join::JoinHandle, task::TaskQueue, waker::waker},
};

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|executor| executor.block_on(future))
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|executor| executor.spawn(future))
}

pub fn run() {
    EXECUTOR.with_borrow(|executor| executor.run())
}

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::default());
}

#[derive(Default)]
struct Executor {
    queue: TaskQueue,
}

impl Executor {
    fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + 'static,
    {
        let (sender, receiver) = oneshot::channel();

        self.queue.schedule(async move {
            let _ = sender.send(future.await);
        });

        JoinHandle::new(receiver)
    }

    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        let mut handle = self.spawn(future);

        self.run();

        handle
            .try_join()
            .expect("Failed to complete future. Deadlock maybe?")
    }

    fn run(&self) {
        while !self.queue.is_done() {
            debug!("Executing tasks");
            while let Some(task) = self.queue.pop() {
                let waker = waker(task.clone());
                let mut cx = Context::from_waker(&waker);
                let _ = task.poll(&mut cx);
            }

            debug!("Waiting for IO");
            reactor::wait();
        }
    }
}
