use core::{cell::RefCell, future::Future};

use inel_executor::{Executor, JoinHandle};
use inel_reactor::Ring;

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::new());
    static REACTOR: RefCell<Ring> = RefCell::new(Ring::new());
}

struct GlobalReactor;
impl inel_interface::Reactor for GlobalReactor {
    type Handle = Ring;

    fn wait(&self) {
        REACTOR.with_borrow_mut(|react| react.wait());
    }

    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Self::Handle) -> T,
    {
        REACTOR.with_borrow_mut(|react| f(react))
    }
}

pub use inel_macro::main;

#[inline]
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|exe| exe.spawn(future))
}

#[inline]
pub fn block_on<F>(future: F) -> F::Output
where
    F: Future + 'static,
{
    EXECUTOR.with_borrow(|exe| exe.block_on_with_reactor(GlobalReactor, future))
}

#[inline]
pub fn run() {
    EXECUTOR.with_borrow(|exe| exe.run_with_reactor(GlobalReactor));
}

#[cfg(feature = "sys")]
pub mod sys {
    use super::GlobalReactor;

    pub use inel_reactor::{buffer::*, op, Submission};

    #[allow(private_interfaces)]
    #[inline]
    pub const fn reactor() -> GlobalReactor {
        GlobalReactor
    }

    #[allow(private_interfaces)]
    #[inline]
    pub fn wrap<O: op::Op>(op: O) -> Submission<O, GlobalReactor> {
        op.run_on(reactor())
    }
}
