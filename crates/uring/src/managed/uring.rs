use core::{pin::Pin, task::Waker};

use rustix::io::Errno;

use crate::{
    managed::{
        buf_rings::{BufGroupId, BufRings},
        cancellation::Cancellation,
        completion::{CompletionEntry, CompletionResult, Completions, Key, UserData},
        submission::{AsyncCancel, DetachOp, MultiOp, SingleOp},
    },
    sys::IoUring,
};

pub trait UringProxy {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T;

    fn submit<O: SingleOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        self.with(|ring| ring.submit(op, waker))
    }

    fn submit_multi<O: MultiOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        self.with(|ring| ring.submit_multi(op, waker))
    }

    fn submit_detached<O: DetachOp>(&mut self, op: O) {
        self.with(|ring| ring.submit_detached(op))
    }

    fn cancel(&self, key: Key, handle: Cancellation, entry: bool) {
        self.with(|ring| ring.cancel(key, handle, entry))
    }

    fn update(&self, key: Key, waker: Waker) {
        self.with(|ring| ring.update(key, waker))
    }

    fn result(&self, key: Key) -> Option<CompletionResult> {
        self.with(|ring| ring.result(key))
    }

    fn wait(&self) {
        self.with(|ring| ring.wait());
    }

    fn is_done(&self) -> bool {
        self.with(|ring| ring.is_done())
    }
}

impl<P: UringProxy> UringProxy for &P {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T,
    {
        (*self).with(f)
    }
}

pub struct Uring {
    ring: IoUring,
    comp: Completions,

    buf_rings: BufRings,
}

impl Uring {
    const SUBMISSION_QUEUE_FULL_ERROR: &str =
        "Submission queue full. Consider configuring with more `sq_entries`.";
    const FAILED_TO_ENTER_ERROR: &str = "Failed to enter io_uring";

    pub fn new() -> Result<Self, Errno> {
        let ring = IoUring::options()
            .with_sq_entries(128)
            .with_cq_entries(256)
            .with_single_issuer()
            .with_defer_taskrun()
            .with_coop_taskrun()
            .with_taskrun_flag()
            .with_submit_all()
            .build()?;

        let comp = Completions::new(ring.sq_size());

        Ok(Self {
            ring,
            comp,
            buf_rings: BufRings::new(),
        })
    }

    pub const fn is_done(&self) -> bool {
        self.comp.is_empty()
    }

    pub fn submit<O: SingleOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let key = self.comp.insert_single(waker);

        // SAFETY: Op implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    let user_data = UserData::build(sqe, key);
                    sqe.user_data(user_data.as_raw());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        key
    }

    pub fn submit_multi<O: MultiOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let key = self.comp.insert_multi(waker);

        // SAFETY: MultiOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    let user_data = UserData::build(sqe, key);
                    sqe.user_data(user_data.as_raw());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        key
    }

    pub fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        // SAFETY: DetachOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep_detached(sqe);
                    sqe.user_data(UserData::IGNORE);
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    pub fn update(&mut self, key: Key, waker: Waker) {
        self.comp.update(key, waker);
    }

    pub fn cancel(&mut self, key: Key, handle: Cancellation, entry: bool) {
        if !self.comp.cancel(key, handle) {
            return;
        }

        if entry {
            self.submit_detached(AsyncCancel::new(key));
        }
    }

    pub fn result(&mut self, key: Key) -> Option<CompletionResult> {
        self.comp.result(key)
    }

    pub fn wait(&mut self) {
        self.ring
            .submit_and_wait(1)
            .expect(Self::FAILED_TO_ENTER_ERROR);

        self.ring.for_each_cqe(|cqe| {
            let completion = CompletionEntry::from_cqe(cqe);

            if completion.should_ignore() {
                return;
            }

            if !self.comp.notify(completion.key(), completion.result()) {
                if let Some(bid) = cqe.buffer_id() {
                    self.buf_rings.recycle(completion.bgid(), bid);
                }
            }
        });
    }

    fn create_buf_group(&mut self, max_entries: u16) -> Result<BufGroupId, Errno> {
        let bgid = self.buf_rings.insert(max_entries);
        unsafe {
            self.ring
                .register_buf_ring(self.buf_rings.get_mut(bgid), bgid.0, 0)?;
        }
        Ok(bgid)
    }

    fn destroy_buf_group(&mut self, bgid: BufGroupId) {
        self.buf_rings.remove(bgid);
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        let _ = self.ring.unregister_buffers();
        let _ = self.ring.unregister_files();
    }
}

pub struct BufGroup<U: UringProxy> {
    id: BufGroupId,
    ring: U,
}

impl<U: UringProxy> Drop for BufGroup<U> {
    fn drop(&mut self) {
        self.ring.with(|ring| ring.destroy_buf_group(self.id))
    }
}

impl<U: UringProxy> BufGroup<U> {
    pub fn new(max_entries: u16, ring: U) -> Result<Self, Errno> {
        ring.with(|ring| ring.create_buf_group(max_entries))
            .map(|id| Self { id, ring })
    }
}
