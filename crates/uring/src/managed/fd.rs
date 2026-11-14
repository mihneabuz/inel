use rustix::{fd::{AsRawFd, OwnedFd, RawFd}, io_uring::IoringSqeFlags};

use crate::{
    managed::{
        UringProxy,
        cancellation::Cancellation,
        submission::{DetachOp, Op},
    },
    sys::Sqe,
};

/// Something ops can do I/O on: a regular fd or a [`DirectFd`]. Ops borrow
/// it for as long as they live.
pub trait IoSource {
    fn raw_fd(&self) -> RawFd;
    fn flags(&self) -> IoringSqeFlags;
}

impl IoSource for OwnedFd {
    fn raw_fd(&self) -> RawFd {
        self.as_raw_fd()
    }

    fn flags(&self) -> IoringSqeFlags {
        IoringSqeFlags::empty()
    }
}

/// A direct descriptor: a file in the ring's own table.
///
/// Unlike a regular fd, it isn't in the process' fd table, which saves the
/// kernel the fd lookup and reference counting on every op. Only meaningful to
/// the ring that created it.
///
/// Closed (asynchronously, through the ring) on drop. Ops still in flight on
/// it aren't affected: the kernel resolves their file when they are issued,
/// which is before the close, since SQEs are issued in order.
pub struct DirectFd<U: UringProxy> {
    slot: u32,
    ring: U,
}

impl<U: UringProxy> DirectFd<U> {
    /// # Safety
    /// `slot` must hold a direct descriptor of `ring`'s [`Uring`](super::Uring)
    /// that nothing else owns.
    pub(crate) const unsafe fn from_raw(slot: u32, ring: U) -> Self {
        Self { slot, ring }
    }

    /// The descriptor's slot in the table.
    pub const fn index(&self) -> u32 {
        self.slot
    }
}

impl<U: UringProxy> Drop for DirectFd<U> {
    fn drop(&mut self) {
        let slot = self.slot;
        self.ring.with(|uring| uring.submit_detached(CloseDirect(slot)));
    }
}

impl<U: UringProxy> IoSource for DirectFd<U> {
    fn raw_fd(&self) -> RawFd {
        self.index() as RawFd
    }

    fn flags(&self) -> IoringSqeFlags {
        IoringSqeFlags::FIXED_FILE
    }
}

/// Closes a direct descriptor.
struct CloseDirect(u32);

// SAFETY: the SQE references no memory
unsafe impl Op for CloseDirect {
    fn prep(&mut self, sqe: &mut Sqe) {
        sqe.prep_close_direct(self.0);
    }

    fn cancel(self) -> (Cancellation, bool) {
        unreachable!();
    }
}

// SAFETY: see `Op`, and it selects no buffer
unsafe impl DetachOp for CloseDirect {}
