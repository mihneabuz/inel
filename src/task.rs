use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc};

use flume::{Receiver, Sender};

pub struct Task {
    pub future: RefCell<Pin<Box<dyn Future<Output = ()>>>>,
    pub queue: Sender<Rc<Task>>,
}

impl Task {
    pub fn schedule(self: &Rc<Self>) {
        self.queue.send(Rc::clone(self)).unwrap()
    }
}

pub struct TaskQueue {
    sender: Sender<Rc<Task>>,
    receiver: Receiver<Rc<Task>>,
    tasks: RefCell<Vec<Rc<Task>>>,
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskQueue {
    pub fn new() -> Self {
        let (sender, receiver) = flume::unbounded();
        Self {
            sender,
            receiver,
            tasks: RefCell::new(Vec::new()),
        }
    }

    pub fn collect(&self) {
        self.tasks.borrow_mut().extend(self.receiver.try_iter());
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

    pub fn pop(&self) -> Option<Rc<Task>> {
        self.tasks.borrow_mut().pop()
    }

    pub fn len(&self) -> usize {
        self.tasks.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.borrow().is_empty()
    }
}
