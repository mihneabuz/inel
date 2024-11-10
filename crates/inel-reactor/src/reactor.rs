use std::{io::Result, task::Waker};

use inel_interface::Reactor;
use io_uring::squeue::Entry;

use crate::{buffer::StableBuffer, BufferKey, Cancellation, Key, Ring};

pub(crate) trait RingReactor {
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key;
    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation);
    fn check_result(&mut self, key: Key) -> Option<i32>;

    unsafe fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferKey>
    where
        B: StableBuffer;

    unsafe fn unregister_buffer<B>(&mut self, buffer: &mut B, key: BufferKey)
    where
        B: StableBuffer;
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

    unsafe fn register_buffer<B>(&mut self, buffer: &mut B) -> Result<BufferKey>
    where
        B: StableBuffer,
    {
        self.with(|react| react.register_buffer(buffer))
    }

    unsafe fn unregister_buffer<B>(&mut self, buffer: &mut B, key: BufferKey)
    where
        B: StableBuffer,
    {
        self.with(|react| react.unregister_buffer(buffer, key))
    }
}
