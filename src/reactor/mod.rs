pub mod handle;
pub mod read;
pub mod ring;
pub mod socket;
pub mod write;

use std::{cell::RefCell, task::Context};

use io_uring::squeue::Entry;
use oneshot::Sender;

use ring::{CancelHandle, Reactor};

pub(crate) fn submit(entry: Entry, sender: Sender<i32>, cx: &mut Context) -> CancelHandle {
    REACTOR.with_borrow_mut(|reactor| reactor.submit(entry, sender, cx))
}

pub(crate) fn wait() {
    REACTOR.with_borrow_mut(|reactor| reactor.wait())
}

thread_local! {
    pub static REACTOR: RefCell<Reactor> = RefCell::new(Reactor::default());
}
