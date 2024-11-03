use core::{cell::RefCell, future::Future};

use inel_executor::{Executor, JoinHandle};
use inel_reactor::Ring;

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::new());
    static REACTOR: RefCell<Ring> = RefCell::new(Ring::with_capacity(1024));
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
pub use inel_reactor::buffer::{StableBuffer, StableMutBuffer};
pub mod fs;
pub mod time;
mod traits;
pub use traits::*;

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
    EXECUTOR.with_borrow(|exe| exe.block_on(GlobalReactor, future))
}

#[inline]
pub fn run() {
    EXECUTOR.with_borrow(|exe| exe.run(GlobalReactor));
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
