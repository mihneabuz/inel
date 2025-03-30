use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use flume::{Receiver, Sender};

type BoxFuture = Pin<Box<dyn Future<Output = ()>>>;

pub struct Task {
    scheduled: Cell<bool>,
    future: RefCell<BoxFuture>,
    queue: Sender<Rc<Task>>,
}

impl Task {
    pub fn poll(&self, cx: &mut Context) -> Poll<()> {
        self.scheduled.set(false);
        self.future.borrow_mut().as_mut().poll(cx)
    }

    pub fn schedule(self: &Rc<Self>) {
        if !self.scheduled.replace(true) {
            self.queue.send(Rc::clone(self)).unwrap()
        }
    }
}

pub struct TaskQueue {
    sender: Sender<Rc<Task>>,
    receiver: Receiver<Rc<Task>>,
}

impl TaskQueue {
    pub fn new() -> Self {
        let (sender, receiver) = flume::unbounded();
        Self { sender, receiver }
    }

    pub fn schedule<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        let task = Rc::new(Task {
            scheduled: Cell::new(false),
            future: RefCell::new(Box::pin(future)),
            queue: self.sender.clone(),
        });

        task.schedule();
    }

    pub fn drain(&self) -> impl Iterator<Item = Rc<Task>> + '_ {
        self.receiver.try_iter()
    }

    pub fn is_done(&self) -> bool {
        self.receiver.sender_count() == 1
    }
}
