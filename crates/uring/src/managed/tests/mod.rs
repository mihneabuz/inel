mod cancellation;
mod completion;
mod submission;

use core::mem;
use std::{
    cell::RefCell,
    collections::VecDeque,
    pin::{Pin, pin},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Wake, Waker},
};

use crate::{
    managed::{UringProxy, submission::Op},
    sys::Sqe,
};

use super::{
    cancellation::Cancellation,
    completion::{CompletionResult, Completions, Key},
    submission::{DetachOp, MultiOp, SingleOp},
};

const IGNORE_KEY: u64 = u64::MAX;

#[derive(Clone)]
pub struct TestUring {
    ring: Rc<RefCell<TestUringInner>>,
}

pub struct TestUringInner {
    mock: VecDeque<TestSubmission>,
    comp: Completions,
    active: u64,
}

pub struct TestSubmission {
    sqe: Sqe,
    user_data: u64,
}

impl TestSubmission {
    fn new() -> Self {
        Self {
            sqe: unsafe { mem::zeroed() },
            user_data: 0,
        }
    }
}

impl UringProxy for TestUring {
    fn with<F, T>(&self, _: F) -> T
    where
        F: FnOnce(&mut super::Uring) -> T,
    {
        panic!("TestUring does not have a real Uring instance");
    }

    fn submit<O: SingleOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut this = self.ring.borrow_mut();
        let key = this.comp.insert_single(waker);

        let mut sub = TestSubmission::new();
        unsafe { op.prep(&mut sub.sqe) };
        sub.user_data = key.as_u64();
        this.mock.push_back(sub);

        this.active += 1;

        key
    }

    fn submit_multi<O: MultiOp>(&self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut this = self.ring.borrow_mut();
        let key = this.comp.insert_multi(waker);

        let mut sub = TestSubmission::new();
        unsafe { op.prep(&mut sub.sqe) };
        sub.user_data = key.as_u64();
        this.mock.push_back(sub);

        this.active += 1;

        key
    }

    fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        let mut sub = TestSubmission::new();
        op.prep_detached(&mut sub.sqe);
        sub.user_data = IGNORE_KEY;
        self.ring.borrow_mut().mock.push_back(sub);
    }

    fn cancel(&self, key: Key, handle: Cancellation, _: bool) {
        self.ring.borrow_mut().comp.cancel(key, handle);
    }

    fn update(&self, key: Key, waker: Waker) {
        self.ring.borrow_mut().comp.update(key, waker)
    }

    fn result(&self, key: Key) -> Option<CompletionResult> {
        self.ring.borrow_mut().comp.result(key)
    }

    fn wait(&self) {
        let mut this = self.ring.borrow_mut();
        while let Some(sub) = this.mock.pop_front() {
            if sub.user_data == IGNORE_KEY {
                continue;
            }
            this.active -= 1;
            let key = Key::from_u64(sub.user_data);
            this.comp.notify(key, ok(0));
        }
    }

    fn is_done(&self) -> bool {
        self.ring.borrow_mut().active == 0
    }
}

impl TestUring {
    pub fn new() -> Self {
        Self {
            ring: Rc::new(RefCell::new(TestUringInner {
                mock: VecDeque::new(),
                comp: Completions::new(128),
                active: 0,
            })),
        }
    }

    pub fn nop_submit(&mut self, waker: Waker) -> Key {
        let nop = pin!(Nop);
        self.submit(nop, waker)
    }

    pub fn nop_submit_multi(&self, waker: Waker) -> Key {
        let nop = pin!(Nop);
        self.submit_multi(nop, waker)
    }

    pub fn nop_cancel(&self, key: Key, handle: Cancellation) -> bool {
        self.ring.borrow_mut().comp.cancel(key, handle)
    }

    pub fn nop_complete(&mut self, key: Key, result: CompletionResult) {
        self.complete_key(key, result);
    }

    /// Pop the next non-detached SQE, complete it with the given result,
    /// and return its key.
    pub fn complete_next(&self, result: CompletionResult) -> Key {
        let mut this = self.ring.borrow_mut();
        while let Some(sub) = this.mock.pop_front() {
            if sub.user_data == IGNORE_KEY {
                continue;
            }
            if !result.has_more() {
                this.active -= 1;
            }
            let key = Key::from_u64(sub.user_data);
            this.comp.notify(key, result);
            return key;
        }
        panic!("No pending SQEs to complete");
    }

    /// Complete a known key with the given result (for multi-shot follow-ups).
    pub fn complete_key(&self, key: Key, result: CompletionResult) {
        let mut this = self.ring.borrow_mut();
        if !result.has_more() {
            this.active -= 1;
        }
        this.comp.notify(key, result);
    }
}

pub struct Nop;

impl Op for Nop {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_nop();
    }
}

impl SingleOp for Nop {
    type Output = i32;
    fn result(self, cqe: CompletionResult) -> i32 {
        cqe.result()
    }
}

impl MultiOp for Nop {
    type Output = i32;
    fn result(self: Pin<&mut Self>, cqe: CompletionResult) -> i32 {
        cqe.result()
    }
}

pub struct TestWaker {
    woken: AtomicBool,
}

impl TestWaker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            woken: AtomicBool::new(false),
        })
    }

    pub fn waker(self: &Arc<Self>) -> Waker {
        Waker::from(self.clone())
    }

    pub fn was_woken(&self) -> bool {
        self.woken.swap(false, Ordering::Relaxed)
    }
}

impl Wake for TestWaker {
    fn wake(self: Arc<Self>) {
        self.woken.store(true, Ordering::Relaxed);
    }
}

fn ok(res: i32) -> CompletionResult {
    CompletionResult::raw(res, rustix::io_uring::IoringCqeFlags::empty())
}

fn ok_more(res: i32) -> CompletionResult {
    CompletionResult::raw(res, rustix::io_uring::IoringCqeFlags::MORE)
}

pub struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub fn flag() -> (Arc<AtomicBool>, DropFlag) {
    let f = Arc::new(AtomicBool::new(false));
    (f.clone(), DropFlag(f))
}

pub fn dropped(f: &AtomicBool) -> bool {
    f.load(Ordering::Relaxed)
}
