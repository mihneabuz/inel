use core::{ptr::NonNull, task::Waker};

use rustix::io::Errno;

use crate::{
    managed::{
        Uring,
        cancellation::Cancellation,
        completion::{CompletionResult, Key},
        group::{BufGroupId, BufId},
        submission::{MultiOp, SingleOp},
    },
    sys::Sqe,
};

/// Access to a [`Uring`], wrapped in whatever the consumer likes (a
/// `ThreadLocal<RefCell<_>>`, an `Rc<RefCell<_>>`, ...). The managed types
/// only reach the ring through this trait.
///
/// # Safety
/// Every call to [`Self::with`], on this proxy or any of its clones, must pass
/// `f` the same [`Uring`]. In particular, it must not be replaced by another
/// one while anything created through the proxy (ops, buffer groups,
/// buffers) is alive: keys and buffer ids are only meaningful to the ring
/// that handed them out.
pub unsafe trait UringProxy: Clone {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T;

    fn wait(&self) {
        self.with(|uring| uring.wait());
    }

    fn is_done(&self) -> bool {
        self.with(|uring| uring.is_done())
    }

    fn shutdown(&self) {
        self.with(|uring| uring.shutdown());
    }
}

// SAFETY: forwards to `P`, so it reaches the same ring
unsafe impl<P: UringProxy> UringProxy for &P {
    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Uring) -> T,
    {
        (*self).with(f)
    }
}

/// The ring protocol used by the managed types. Crate-private and implemented
/// for every proxy, so it can neither be called nor overridden from outside:
/// its methods trust their keys and buffer ids.
pub trait UringProxyExt: UringProxy {
    fn submit<O: SingleOp<Self>>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit::<Self, _>(op, waker))
    }

    fn submit_multi<O: MultiOp<Self>>(&self, op: &mut O, waker: Waker) -> Key {
        self.with(|uring| uring.submit_multi::<Self, _>(op, waker))
    }

    /// # Safety
    /// See [`Uring::prep_detached`].
    unsafe fn prep_detached<F: FnOnce(&mut Sqe)>(&self, prep: F) {
        // SAFETY: forwarded to the caller
        self.with(|uring| unsafe { uring.prep_detached(prep) })
    }

    fn cancel(&self, key: &Key, handle: Cancellation, entry: bool) {
        self.with(|uring| uring.cancel(key, handle, entry))
    }

    fn update(&self, key: &Key, waker: &Waker) {
        self.with(|uring| uring.update(key, waker))
    }

    fn result(&self, key: &Key) -> Option<CompletionResult> {
        self.with(|uring| uring.result(key))
    }

    /// Allocates and registers a group of `count` buffers of `size` bytes.
    fn create_buf_group(&self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        self.with(|uring| uring.create_buf_group(count, size))
    }

    /// A handle to the group was cloned.
    fn clone_buf_group(&self, id: BufGroupId) {
        self.with(|uring| uring.clone_buf_group(id))
    }

    /// A handle to the group was dropped. The group is freed once no handle,
    /// op or buffer uses it anymore.
    fn drop_buf_group(&self, id: BufGroupId) {
        self.with(|uring| uring.drop_buf_group(id))
    }

    /// Hands out buffer `bid` of group `id`, keeping the group alive until it
    /// is returned with [`Self::return_buf`].
    fn take_buf(&self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        self.with(|uring| uring.take_buf(id, bid))
    }

    /// Puts a buffer from [`Self::take_buf`] back in its group's ring.
    fn return_buf(&self, id: BufGroupId, bid: BufId) {
        self.with(|uring| uring.return_buf(id, bid))
    }
}

impl<P: UringProxy> UringProxyExt for P {}
