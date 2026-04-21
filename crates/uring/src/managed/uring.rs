use core::{pin::Pin, task::Waker};

use rustix::io::Errno;

use crate::{
    managed::{
        cancellation::Cancellation,
        completion::{CompletionResult, Completions, Key},
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
    active: u64,
}

impl Uring {
    const IGNORE_KEY: u64 = u64::MAX;

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
            active: 0,
        })
    }

    pub const fn is_done(&self) -> bool {
        self.active == 0
    }

    pub fn submit<O: SingleOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let key = self.comp.insert_single(waker);

        // SAFETY: Op implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    sqe.user_data(key.as_u64());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        self.active += 1;

        key
    }

    pub fn submit_multi<O: MultiOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let key = self.comp.insert_multi(waker);

        // SAFETY: MultiOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    sqe.user_data(key.as_u64());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        self.active += 1;

        key
    }

    pub fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        // SAFETY: DetachOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep_detached(sqe);
                    sqe.user_data(Self::IGNORE_KEY);
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
            if cqe.user_data() == Self::IGNORE_KEY {
                return;
            }

            if !cqe.has_more() {
                self.active -= 1;
            }

            let key = Key::from_u64(cqe.user_data());
            self.comp.notify(key, CompletionResult::from_cqe(cqe));
        });
    }
}
