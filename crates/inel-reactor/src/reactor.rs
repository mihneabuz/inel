#![allow(clippy::missing_safety_doc)]
use std::{io::Result, task::Waker};

use inel_interface::Reactor;
use io_uring::squeue::Entry;

use crate::{
    buffer::StableBuffer,
    ring::{BufferSlot, DirectSlot, RingResult},
    BufferGroupId, Cancellation, Key, Ring,
};

pub trait RingReactor {
    unsafe fn submit(&mut self, entry: Entry, waker: Waker) -> Key;
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
        self.with(|react| react.submit(entry, waker)).unwrap()
    }

    unsafe fn cancel(&mut self, key: Key, entry: Option<Entry>, cancel: Cancellation) {
        self.with(|react| react.cancel(key, entry, cancel));
    }

    fn check_result(&mut self, key: Key) -> Option<RingResult> {
        self.with(|react| react.check_result(key)).unwrap()
    }

    fn register_buffer<B: StableBuffer>(&mut self, buffer: &mut B) -> Result<BufferSlot> {
        self.with(|react| react.register_buffer(buffer)).unwrap()
    }

    fn unregister_buffer(&mut self, slot: BufferSlot) {
        self.with(|react| react.unregister_buffer(slot));
    }

    fn get_direct_slot(&mut self) -> Result<DirectSlot> {
        self.with(|react| react.get_direct_slot()).unwrap()
    }

    fn release_direct_slot(&mut self, slot: DirectSlot) {
        self.with(|react| react.release_direct_slot(slot));
    }

    fn get_buffer_group(&mut self) -> Result<BufferGroupId> {
        self.with(|react| react.get_buffer_group()).unwrap()
    }

    fn release_buffer_group(&mut self, id: BufferGroupId) {
        self.with(|react| react.release_buffer_group(id));
    }
}
