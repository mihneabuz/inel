use std::marker::PhantomData;

use rustix::{
    fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    io_uring::IoringSqeFlags,
};

use crate::managed::{UringProxy, UringProxyExt};

/// A source descriptor for an op.
///
/// Can either be a process wide fd or a direct slot of a ring.
/// The lifetime ensures that the descriptor cannot be dropped and closed while
/// we hold a reference to it.
///
/// The direct slot variant does not carry the owning ring, meaning it is the
/// responsability of the user to ensure that such a variant is only used for
/// ops on the same ring.
#[derive(Debug, Clone)]
pub enum RawSrc<'a> {
    Global(BorrowedFd<'a>),
    Direct(u32, PhantomData<&'a ()>),
}

impl RawSrc<'_> {
    pub(crate) fn raw_fd(&self) -> RawFd {
        match self {
            RawSrc::Global(fd) => fd.as_raw_fd(),
            RawSrc::Direct(slot, _) => *slot as RawFd,
        }
    }

    pub(crate) const fn flags(&self) -> IoringSqeFlags {
        match self {
            RawSrc::Global(..) => IoringSqeFlags::empty(),
            RawSrc::Direct(..) => IoringSqeFlags::FIXED_FILE,
        }
    }
}

/// Something that has a source descriptor for an op to perform I/O on.
///
/// That descriptor can either be a process wide fd or a direct slot of a ring.
pub trait IoSrc {
    fn raw_src(&self) -> RawSrc<'_>;
}

impl<'a> IoSrc for RawSrc<'a> {
    fn raw_src(&self) -> RawSrc<'_> {
        self.clone()
    }
}

impl<T: AsFd> IoSrc for T {
    fn raw_src(&self) -> RawSrc<'_> {
        RawSrc::Global(self.as_fd())
    }
}

/// A regular fd, owned by a ring.
///
/// Usable like any fd, but closed (asynchronously, through the ring) on drop.
/// Ops still in flight on it aren't affected: the kernel resolves their file
/// when they are issued, which is before the close, since SQEs are issued in
/// order.
pub struct UringFd<U: UringProxy> {
    raw: RawFd,
    uring: U,
}

impl<U: UringProxy> AsRawFd for UringFd<U> {
    fn as_raw_fd(&self) -> RawFd {
        self.raw
    }
}

impl<U: UringProxy> AsFd for UringFd<U> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: the fd stays open until we are dropped
        unsafe { BorrowedFd::borrow_raw(self.raw) }
    }
}

impl<U: UringProxy> UringFd<U> {
    /// # Safety
    /// `fd` must be an open fd that nothing else owns.
    pub(crate) const unsafe fn from_raw(fd: RawFd, uring: U) -> Self {
        Self { raw: fd, uring }
    }
}

impl<U: UringProxy> Drop for UringFd<U> {
    fn drop(&mut self) {
        // SAFETY: the close references no memory
        unsafe {
            self.uring.prep_detached(|sqe| {
                sqe.prep_close(self.raw);
            });
        }
    }
}

/// A direct descriptor in the ring's own table.
///
/// Unlike a regular fd, it isn't in the process' fd table, which saves the
/// kernel the fd lookup and reference counting on every op. Only meaningful to
/// the ring that created it. Using it in an op on another ring is not memory
/// unsafe, but will have unexpected results, based on what happens to be in
/// that slot in the other ring.
///
/// Closed (asynchronously, through the ring) on drop. Ops still in flight on
/// it aren't affected: the kernel resolves their file when they are issued,
/// which is before the close, since SQEs are issued in order.
pub struct DirectFd<U: UringProxy> {
    slot: u32,
    uring: U,
}

impl<U: UringProxy> DirectFd<U> {
    /// # Safety
    /// `slot` must hold a direct descriptor of `uring`'s [`Uring`](super::Uring)
    /// that nothing else owns.
    pub(crate) const unsafe fn from_raw(slot: u32, uring: U) -> Self {
        Self { slot, uring }
    }

    /// The descriptor's slot in the table.
    pub const fn index(&self) -> u32 {
        self.slot
    }
}

impl<U: UringProxy> Drop for DirectFd<U> {
    fn drop(&mut self) {
        // SAFETY: the close references no memory
        unsafe {
            self.uring.prep_detached(|sqe| {
                sqe.prep_close_direct(self.slot);
            });
        }
    }
}

impl<U: UringProxy> IoSrc for DirectFd<U> {
    fn raw_src(&self) -> RawSrc<'_> {
        RawSrc::Direct(self.index(), PhantomData)
    }
}
