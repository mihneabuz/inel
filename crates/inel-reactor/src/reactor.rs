use std::task::Waker;

use inel_interface::Reactor;
use io_uring::squeue::Entry;

use crate::{cancellation::Cancellation, completion::Key, Ring};

pub(crate) trait RingReactor {
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key;
    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation);
    fn check_result(&mut self, key: Key) -> Option<i32>;
}

impl<R> RingReactor for R
where
    R: Reactor<Handle = Ring>,
{
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        self.with(|react| react.submit(entry, waker))
    }

    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        self.with(|react| react.cancel(key, entry, cancel));
    }

    fn check_result(&mut self, key: Key) -> Option<i32> {
        self.with(|react| react.check_result(key))
    }
}
