use core::task::Waker;
use std::rc::Rc;

use rustix::io::Errno;

use crate::{
    managed::{
        SelectGroup, Uring,
        cancellation::Cancellation,
        completion::{CompletionResult, Key},
        submission::{MultiOp, SingleOp},
    },
    sys::Sqe,
};

/// Provides access to a [`Uring`].
///
/// This trait makes it possible to define types which are generic over the way they
/// obtain access to a [`Uring`], allowing for it to be stored in a thread local or
/// an [`std::rc::Rc`], or just using it by reference.
///
/// # Safety
///
/// Every call to [`Self::with`], on this proxy or any of its clones, must pass `f`
/// the same [`Uring`].
pub unsafe trait UringProxy: Clone {
    /// Runs `f` on the proxied [`Uring`].
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Uring) -> T;

    /// Submits any pending ops and reaps completions.
    fn wait(&self) {
        self.with(|uring| uring.wait());
    }

    /// Returns true if there are no more in-flight ops.
    fn is_done(&self) -> bool {
        self.with(|uring| uring.is_done())
    }

    /// Attempts to cancel all in-flight ops and shut down the ring.
    fn shutdown(&self) {
        self.with(|uring| uring.shutdown());
    }
}

// SAFETY: copies of the reference all point to the same `Uring`
unsafe impl UringProxy for &Uring {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Uring) -> T,
    {
        f(self)
    }
}

// SAFETY: every clone shares the same `Uring` through the `Rc`
unsafe impl UringProxy for Rc<Uring> {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Uring) -> T,
    {
        f(self)
    }
}

// SAFETY: forwards to `U`, so it reaches the same ring
unsafe impl<U: UringProxy> UringProxy for &U {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Uring) -> T,
    {
        (*self).with(f)
    }
}

/// Extension trait containing crate-private functions used by managed types.
pub(crate) trait UringProxyExt: UringProxy {
    /// Submits a single-shot `op`, returning a `Key` for the completion entry.
    fn submit<O: SingleOp<Self>>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit::<Self, _>(op, waker))
    }

    /// Submits a multi-shot `op`, returning a `Key` for the completion entry.
    fn submit_multi<O: MultiOp<Self>>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit_multi::<Self, _>(op, waker))
    }

    /// # Safety
    /// See [`Uring::prep_detached`].
    unsafe fn prep_detached<F: FnOnce(&mut Sqe)>(&self, prep: F) {
        // SAFETY: forwarded to the caller
        self.with(|uring| unsafe { uring.prep_detached(prep) })
    }

    /// Cancels an `op` identified by `key`.
    fn cancel(&self, key: &Key, handle: Cancellation, entry: bool) {
        self.with(|uring| uring.cancel(key, handle, entry))
    }

    /// Updates the waker for an `op` identified by `key`.
    fn update(&self, key: &Key, waker: &Waker) {
        self.with(|uring| uring.update(key, waker))
    }

    /// Pops the next result identified by `key`.
    fn result(&self, key: &Key) -> Option<CompletionResult> {
        self.with(|uring| uring.result(key))
    }

    /// Allocates and registers a group of `count` buffers of `size` bytes.
    fn create_buf_group(&self, count: u16, size: u32) -> Result<SelectGroup, Errno> {
        self.with(|uring| uring.create_buf_group(count, size))
    }
}

impl<U: UringProxy> UringProxyExt for U {}
