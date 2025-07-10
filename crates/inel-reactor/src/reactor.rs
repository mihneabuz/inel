#![allow(clippy::missing_safety_doc)]
use std::{io::Result, task::Waker};

use inel_interface::Reactor;
use io_uring::squeue::Entry;

use crate::{
    buffer::StableBuffer,
    cancellation::Cancellation,
    ring::{BufferGroupId, BufferSlot, DirectSlot, Key, Ring, RingResult},
};

pub(crate) trait RingReactor {
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key;
    unsafe fn submit_detached(&mut self, entry: Entry);
    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation);
    fn check_result(&mut self, key: Key) -> Option<RingResult>;

    fn register_buffer<B: StableBuffer>(&mut self, buffer: &mut B) -> Result<BufferSlot>;
    fn unregister_buffer(&mut self, slot: BufferSlot);

    fn get_direct_slot(&mut self) -> Result<DirectSlot>;
    fn release_direct_slot(&mut self, slot: DirectSlot);

    fn get_buffer_group(&mut self) -> Result<BufferGroupId>;
    fn release_buffer_group(&mut self, id: BufferGroupId);
}

impl<R> RingReactor for R
where
    R: Reactor<Handle = Ring>,
{
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key {
        self.with(|ring| ring.submit(entry, waker)).unwrap()
    }

    unsafe fn submit_detached(&mut self, entry: Entry) {
        self.with(|ring| ring.submit_detached(entry));
    }

    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        self.with(|ring| ring.cancel(key, entry, cancel));
    }

    fn check_result(&mut self, key: Key) -> Option<RingResult> {
        self.with(|ring| ring.check_result(key)).unwrap()
    }

    fn register_buffer<B: StableBuffer>(&mut self, buffer: &mut B) -> Result<BufferSlot> {
        self.with(|ring| ring.register_buffer(buffer)).unwrap()
    }

    fn unregister_buffer(&mut self, slot: BufferSlot) {
        self.with(|ring| ring.unregister_buffer(slot));
    }

    fn get_direct_slot(&mut self) -> Result<DirectSlot> {
        self.with(|ring| ring.get_direct_slot()).unwrap()
    }

    fn release_direct_slot(&mut self, slot: DirectSlot) {
        self.with(|ring| ring.release_direct_slot(slot));
    }

    fn get_buffer_group(&mut self) -> Result<BufferGroupId> {
        self.with(|ring| ring.get_buffer_group()).unwrap()
    }

    fn release_buffer_group(&mut self, id: BufferGroupId) {
        self.with(|ring| ring.release_buffer_group(id));
    }
}
