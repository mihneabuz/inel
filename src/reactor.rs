use std::{
    cell::RefCell,
    os::fd::{AsFd, AsRawFd},
    task::Context,
};

pub struct Reactor {}

thread_local! {
    pub static REACTOR: RefCell<Reactor> = RefCell::new(Reactor::default());
}

impl Default for Reactor {
    fn default() -> Self {
        Self {}
    }
}

impl Reactor {
    pub fn wake_readable<Fd>(&mut self, fd: &Fd, cx: &mut Context)
    where
        Fd: AsFd + AsRawFd,
    {
        cx.waker().wake_by_ref();
    }

    pub fn wake_writeable<Fd>(&mut self, fd: &Fd, cx: &mut Context)
    where
        Fd: AsFd + AsRawFd,
    {
        cx.waker().wake_by_ref();
    }

    pub fn wait(&mut self) {
        std::thread::yield_now();
    }
}
