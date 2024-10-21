use std::future::Future;

pub trait Executor {
    fn spawn<F>(&self, future: F)
    where
        F: Future + 'static;
}

pub struct NopExecutor;
impl Executor for NopExecutor {
    fn spawn<F>(&self, _future: F)
    where
        F: Future + 'static,
    {
    }
}

pub trait Reactor {
    type Handle;

    fn wait(&self);
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Self::Handle) -> T;
}
