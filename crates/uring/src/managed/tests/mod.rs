mod buffer;
mod cancellation;
mod completion;
mod read;
mod submission;
mod write;

use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use rustix::{
    fd::{AsRawFd, RawFd},
    io_uring::{IoringAsyncCancelFlags, IoringTimeoutFlags, Timespec},
};

use crate::{
    managed::{Uring, UringProxy, submission::Op},
    sys::Sqe,
};

use super::{
    cancellation::Cancellation,
    completion::CompletionResult,
    submission::{DetachOp, MultiOp, SingleOp},
};

/// A real ring (fake under miri) behind the proxy interface.
#[derive(Clone)]
pub struct TestRing {
    ring: Rc<RefCell<Uring>>,
}

impl UringProxy for TestRing {
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

/// An fd that is never valid, for error paths.
pub struct BadFd;

impl AsRawFd for BadFd {
    fn as_raw_fd(&self) -> RawFd {
        -1
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

impl Op for OwnedOp {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_nop();
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.resource.into(), true)
    }
}

impl SingleOp for OwnedOp {
    type Output = ();
    fn result(self, _: CompletionResult) {}
}

impl MultiOp for OwnedOp {
    type Output = ();
    fn result(self: Pin<&mut Self>, _: CompletionResult) {}
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

impl Op for Timeout {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_timeout(&*self.ts, self.count, self.flags);
    }

    fn cancel(self) -> (Cancellation, bool) {
        (self.ts.into(), true)
    }
}

impl SingleOp for Timeout {
    type Output = i32;
    fn result(self, cqe: CompletionResult) -> i32 {
        cqe.result()
    }
}

impl MultiOp for Timeout {
    type Output = i32;
    fn result(self: Pin<&mut Self>, cqe: CompletionResult) -> i32 {
        cqe.result()
    }
}

/// Cancels every in-flight op.
pub struct CancelAll;

impl Op for CancelAll {
    unsafe fn prep(self: Pin<&mut Self>, sqe: &mut Sqe) {
        sqe.prep_cancel(0, IoringAsyncCancelFlags::ANY);
    }
}

impl DetachOp for CancelAll {}

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
