use std::{cell::RefCell, future::Future, rc::Rc, task::Context, time::Instant};

use tracing::debug;

use crate::{reactor::REACTOR, task::TaskQueue, waker::waker};

#[derive(Default)]
pub struct Executor {
    queue: TaskQueue,
}

thread_local! {
    pub static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::default());
}

impl Executor {
    pub fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.queue.schedule(future)
    }

    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + 'static,
    {
        let result: Rc<RefCell<Option<F::Output>>> = Rc::new(RefCell::new(None));

        let result_location = result.clone();
        self.spawn(async move {
            let value = future.await;
            result_location.borrow_mut().replace(value);
        });

        self.run();

        result.take().unwrap()
    }

    pub fn run(&self) {
        self.queue.collect();

        while !self.queue.is_empty() {
            // run current tasks
            debug!(count = self.queue.len(), "Executing tasks");
            while let Some(task) = self.queue.pop() {
                let waker = waker(task.clone());
                let mut cx = Context::from_waker(&waker);
                let _ = task.future.borrow_mut().as_mut().poll(&mut cx);
            }

            // wait for io
            debug!("Waiting for IO");
            let time = Instant::now();
            REACTOR.with_borrow_mut(|reactor| reactor.wait());
            debug!(waited =? time.elapsed(), "Resuming");

            // collect any tasks that woke up
            self.queue.collect();
        }
    }
}
