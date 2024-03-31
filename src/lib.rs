pub(crate) mod executor;
pub(crate) mod reactor;
pub(crate) mod task;
pub(crate) mod waker;

pub mod io;
pub mod net;

use std::future::Future;

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + 'static,
{
    executor::EXECUTOR.with_borrow(|executor| executor.block_on(future))
}

pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    executor::EXECUTOR.with_borrow(|executor| executor.spawn(future))
}

pub fn run() {
    executor::EXECUTOR.with_borrow(|executor| executor.run())
}
