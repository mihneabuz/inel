mod buffer;
mod cancellation;
mod completion;
mod direct;
mod open;
mod read;
mod select;
mod shutdown;
mod submission;
mod write;

use std::{
    cell::RefCell,
    marker::PhantomData,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use rustix::io_uring::{IoringTimeoutFlags, Timespec};

use crate::{
    managed::{IoSrc, RawSrc, Uring, UringProxy, submission::Op},
    sys::Sqe,
};

use super::{
    cancellation::Cancellation,
    completion::CompletionResult,
    submission::{MultiOp, SingleOp},
};

/// A real ring (fake under miri) behind the proxy interface.
#[derive(Clone)]
pub struct TestRing {
    ring: Rc<RefCell<Uring>>,
}

// SAFETY: every clone shares the same `Uring` through the `Rc`
unsafe impl UringProxy for TestRing {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T,
    {
        f(&mut self.ring.borrow_mut())
    }
}

impl TestRing {
    pub fn new() -> Self {
        Self {
            ring: Rc::new(RefCell::new(Uring::new().unwrap())),
        }
    }

    pub fn with_direct_files(direct_files: u32) -> Self {
        Self {
            ring: Rc::new(RefCell::new(
                Uring::with_direct_files(direct_files).unwrap(),
            )),
        }
    }

    /// Waits until every op has completed and had its result consumed (or was
    /// cancelled).
    pub fn drain(&self) {
        while !self.is_done() {
            self.wait();
        }
    }
}

pub fn block_on<F: Future>(ring: &TestRing, mut fut: Pin<&mut F>) -> F::Output {
    let tw = TestWaker::new();
    let waker = tw.waker();
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        ring.wait();
    }
}

/// An fd that is never valid, for error paths: a direct descriptor slot far
/// past the end of any table (`-1` can't be a `BorrowedFd`).
pub struct BadFd;

impl IoSrc for BadFd {
    fn raw_src(&self) -> RawSrc<'_> {
        RawSrc::Direct(i32::MAX as u32, PhantomData)
    }
}

pub struct Nop;

// SAFETY: test op, see its docs
unsafe impl Op for Nop {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_nop();
    }
}

// SAFETY: never multishot
unsafe impl<U: UringProxy> SingleOp<U> for Nop {
    type Output = i32;
    fn result(self, cqe: CompletionResult, _: &U) -> i32 {
        cqe.result()
    }
}

impl<U: UringProxy> MultiOp<U> for Nop {
    type Output = i32;
    fn result(&mut self, cqe: CompletionResult, _: &U) -> i32 {
        cqe.result()
    }
}

/// Op that holds a resource moved into Cancellation on cancel.
pub struct OwnedOp {
    resource: Box<DropFlag>,
}

impl OwnedOp {
    pub fn new(df: DropFlag) -> Self {
        Self {
            resource: Box::new(df),
        }
    }
}

// SAFETY: test op, see its docs
unsafe impl Op for OwnedOp {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_nop();
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.resource.into(), true)
    }
}

// SAFETY: never multishot
unsafe impl<U: UringProxy> SingleOp<U> for OwnedOp {
    type Output = ();
    fn result(self, _: CompletionResult, _: &U) {}
}

impl<U: UringProxy> MultiOp<U> for OwnedOp {
    type Output = ();
    fn result(&mut self, _: CompletionResult, _: &U) {}
}

/// Timeout; the kernel copies the timespec during submission.
pub struct Timeout {
    ts: Box<Timespec>,
    count: u32,
    flags: IoringTimeoutFlags,
}

impl Timeout {
    pub fn new(ms: u64, count: u32, flags: IoringTimeoutFlags) -> Self {
        Self {
            ts: Box::new(Timespec {
                tv_sec: (ms / 1000) as _,
                tv_nsec: ((ms % 1000) * 1_000_000) as _,
            }),
            count,
            flags,
        }
    }
}

// SAFETY: test op, see its docs
unsafe impl Op for Timeout {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_timeout(&*self.ts, self.count, self.flags);
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.ts.into(), true)
    }
}

// SAFETY: never multishot
unsafe impl<U: UringProxy> SingleOp<U> for Timeout {
    type Output = i32;
    fn result(self, cqe: CompletionResult, _: &U) -> i32 {
        cqe.result()
    }
}

impl<U: UringProxy> MultiOp<U> for Timeout {
    type Output = i32;
    fn result(&mut self, cqe: CompletionResult, _: &U) -> i32 {
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

pub fn ok(res: i32) -> CompletionResult {
    CompletionResult::raw(res, rustix::io_uring::IoringCqeFlags::empty())
}

pub fn ok_more(res: i32) -> CompletionResult {
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
