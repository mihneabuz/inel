use core::{cell::RefCell, future::Future};

use inel_executor::{Executor, JoinHandle};
use inel_reactor::ring::Ring;

pub use inel_reactor::ring::RingOptions;

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::new(Executor::new());
    static REACTOR: RefCell<Ring> = RefCell::new(Ring::default());
}

struct GlobalReactor;
impl inel_interface::Reactor for GlobalReactor {
    type Handle = Ring;

    fn wait(&self) {
        REACTOR.with_borrow_mut(|react| react.wait());
    }

    fn with<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut Self::Handle) -> T,
    {
        REACTOR.try_with(|react| f(&mut react.borrow_mut())).ok()
    }
}

pub use inel_macro::main;

pub mod buffer;
pub mod fs;
pub mod group;
pub mod io;
pub mod net;
pub mod time;
mod util;

#[cfg(feature = "compat")]
pub mod compat;

mod source;

pub fn init(options: RingOptions) {
    if REACTOR.with_borrow(|react| !react.is_done()) {
        panic!("Tried to init already running reactor");
    }

    REACTOR.set(options.build());
}

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

#[inline]
pub fn is_done() -> bool {
    REACTOR.with_borrow(|react| react.is_done())
}

#[cfg(feature = "sys")]
pub mod sys {
    pub use inel_reactor::{buffer::*, op, submission::Submission};

    #[allow(private_interfaces)]
    pub const fn reactor() -> crate::GlobalReactor {
        crate::GlobalReactor
    }
}
