use std::{cell::RefCell, future::Future};

use inel_executor::{Executor, JoinHandle};

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::new())
}

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|exe| exe.spawn(future))
}

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|exe| exe.block_on_with_wait(future, || todo!()))
}

pub fn run() {
    EXECUTOR.with_borrow(|exe| exe.run_with_wait(|| todo!()))
}
