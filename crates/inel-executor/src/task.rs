use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use flume::{Receiver, Sender};

pub struct Task {
    future: RefCell<Pin<Box<dyn Future<Output = ()>>>>,
    queue: Sender<Rc<Task>>,
}

impl Task {
    pub fn poll(&self, cx: &mut Context) -> Poll<()> {
        self.future.borrow_mut().as_mut().poll(cx)
    }

    pub fn schedule(self: &Rc<Self>) {
        self.queue.send(Rc::clone(self)).unwrap()
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
        let task = Task {
            future: RefCell::new(Box::pin(future)),
            queue: self.sender.clone(),
        };

        self.sender.send(Rc::new(task)).unwrap();
    }

    pub fn drain(&self) -> impl Iterator<Item = Rc<Task>> + '_ {
        self.receiver
            .recv()
            .into_iter()
            .chain(self.receiver.try_iter())
    }

    pub fn is_done(&self) -> bool {
        self.receiver.sender_count() == 1
    }
}
