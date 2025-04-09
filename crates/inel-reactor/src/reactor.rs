use std::{io::Result, task::Waker};

use inel_interface::Reactor;
use io_uring::squeue::Entry;

use crate::{
    buffer::StableBuffer,
    ring::{BufferSlotKey, RingResult},
    Cancellation, Key, Ring,
};

pub trait RingReactor {
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key;
    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation);
    fn check_result(&mut self, key: Key) -> Option<RingResult>;

    fn register_buffer<B: StableBuffer>(&mut self, buffer: &mut B) -> Result<BufferSlotKey>;
    fn unregister_buffer(&mut self, key: BufferSlotKey);
}

impl<R> RingReactor for R
where
    R: Reactor<Handle = Ring>,
{
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        self.with(|react| react.submit(entry, waker)).unwrap()
    }

    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        self.with(|react| react.cancel(key, entry, cancel));
    }

    fn check_result(&mut self, key: Key) -> Option<RingResult> {
        self.with(|react| react.check_result(key)).unwrap()
    }

    fn register_buffer<B: StableBuffer>(&mut self, buffer: &mut B) -> Result<BufferSlotKey> {
        self.with(|react| react.register_buffer(buffer)).unwrap()
    }

    fn unregister_buffer(&mut self, key: BufferSlotKey) {
        self.with(|react| react.unregister_buffer(key)).unwrap()
    }
}
