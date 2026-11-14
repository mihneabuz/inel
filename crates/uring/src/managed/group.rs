use core::{mem, slice};
use std::{
    alloc::{Layout, handle_alloc_error},
    num::NonZeroU16,
    ops::Deref,
    ptr::NonNull,
};

use allocator_api2::alloc::{Allocator, Global};
use rustix::io::Errno;

use crate::{
    managed::{UringProxy, UringProxyExt},
    sys::{IoBufRing, IoUring},
    utils::Slab,
};

/// The kernel's buffer group id. Never 0, so `Option<BufGroupId>` is as small
/// as the id and packs into a `Key` without a separate flag.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct BufGroupId(NonZeroU16);

impl BufGroupId {
    /// The id the kernel uses, or `None` for 0, which means "no group".
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match NonZeroU16::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// The id as the kernel sees it, in SQEs and when registering the ring.
    pub const fn as_raw(self) -> u16 {
        self.0.get()
    }

    /// The id of the group stored in slab slot `slot`, which is `slot + 1`.
    /// `None` if that does not fit a `u16`.
    pub const fn from_slot(slot: u32) -> Option<Self> {
        if slot + 1 > u16::MAX as u32 {
            return None;
        }

        // SAFETY: slot + 1 is > 0 and in bounds
        Some(unsafe { Self(NonZeroU16::new_unchecked(slot as u16 + 1)) })
    }

    /// The slab slot the group is stored in. Inverse of [`Self::from_slot`].
    pub const fn as_slot(self) -> u32 {
        self.as_raw() as u32 - 1
    }
}

/// The id of a buffer within its group, as the kernel reports it in a CQE.
/// Buffer `bid` starts `bid * size` bytes into the group's memory.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct BufId(u16);

impl BufId {
    /// Wraps a buffer id taken from a CQE or used when filling the ring.
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// The id as the kernel sees it.
    pub const fn as_raw(self) -> u16 {
        self.0
    }
}

/// A group of `count` fixed-size buffers the kernel picks from when a
/// buffer-select op runs. Buffers are handed out as [`SelectBuf`]s and go back
/// into the group when those are dropped.
///
/// Cloning gives another handle to the same group, so it can be shared between
/// tasks without an `Rc`. The memory is owned by the ring and outlives the
/// handles: it is only freed once every handle is dropped, no op that may
/// still pick a buffer is in flight, and every [`SelectBuf`] is gone.
pub struct SelectGroup<U: UringProxy> {
    gid: BufGroupId,
    size: u32,
    uring: U,
}

impl<U: UringProxy> SelectGroup<U> {
    /// Allocates `count` buffers of `size` bytes each and registers them with
    /// the ring as a new group.
    ///
    /// Fails with `EINVAL` if `count` or `size` is 0 or `count` is above
    /// 32768, with `ENOMEM` if the total size overflows, or with whatever the
    /// kernel returns when registering.
    ///
    /// # Panics
    /// If the ring already has `u16::MAX` groups.
    pub fn new(count: u16, size: u32, uring: U) -> Result<Self, Errno> {
        let gid = uring.create_buf_group(count, size)?;
        Ok(Self { gid, size, uring })
    }

    /// The size of every buffer in the group, and so the most a single
    /// buffer-select read can return.
    pub const fn buffer_size(&self) -> u32 {
        self.size
    }

    pub(crate) const fn id(&self) -> BufGroupId {
        self.gid
    }

    /// Takes ownership of buffer `bid`, which holds `len` bytes the kernel
    /// wrote. Dropping the returned buffer puts it back in the ring.
    ///
    /// # Safety
    /// `bid` must come from a CQE of an op that selected from this group, and
    /// be taken only once per CQE. `len` must be at most
    /// [`Self::buffer_size`], and the first `len` bytes must be initialized,
    /// i.e. `len` is the CQE's result (or 0).
    pub(crate) unsafe fn take(&self, bid: BufId, len: usize) -> SelectBuf<U> {
        let ptr = self.uring.take_buf(self.gid, bid);
        SelectBuf {
            uring: self.uring.clone(),
            gid: self.gid,
            bid,
            ptr,
            len,
        }
    }
}

impl<U: UringProxy> Clone for SelectGroup<U> {
    fn clone(&self) -> Self {
        self.uring.clone_buf_group(self.gid);
        Self {
            gid: self.gid,
            size: self.size,
            uring: self.uring.clone(),
        }
    }
}

impl<U: UringProxy> Drop for SelectGroup<U> {
    fn drop(&mut self) {
        self.uring.drop_buf_group(self.gid);
    }
}

/// A buffer picked by the kernel from a [`SelectGroup`], holding the bytes it
/// read. Dropping it returns the buffer to the group.
pub struct SelectBuf<U: UringProxy> {
    uring: U,
    gid: BufGroupId,
    bid: BufId,
    ptr: NonNull<u8>,
    len: usize,
}

impl<U: UringProxy> SelectBuf<U> {
    /// Returns the buffer to its group, so the kernel can pick it again. Same
    /// as dropping it.
    pub fn release(self) {
        let _ = self;
    }
}

impl<U: UringProxy> Deref for SelectBuf<U> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: the buffer is ours until dropped, and the memory stays
        // allocated while we hold it
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<U: UringProxy> Drop for SelectBuf<U> {
    fn drop(&mut self) {
        self.uring.return_buf(self.gid, self.bid);
    }
}

/// The ring-side state of every buffer group.
///
/// A group lives until every [`SelectGroup`] handle is dropped and it has no
/// users left. It then goes on the `dead` list and is unregistered and freed
/// by [`BufGroups::free_dead`].
pub(crate) struct BufGroups {
    groups: Slab<BufGroup>,
    dead: Vec<BufGroupId>,
}

/// One group: its buffer ring and the memory of all its buffers.
struct BufGroup<A: Allocator = Global> {
    ring: IoBufRing,

    count: u16,
    size: u32,

    /// `count * size` bytes, buffer `bid` at offset `bid * size`.
    mem: NonNull<u8>,

    /// In-flight ops that may pick a buffer, plus live `SelectBuf`s. The
    /// memory can't be freed while either exists.
    users: u32,
    /// Live `SelectGroup` handles. Once they are all gone, buffers are no longer
    /// put back in the ring.
    handles: u32,

    alloc: A,
}

impl BufGroup {
    fn new(count: u16, size: u32) -> Result<Self, Errno> {
        Self::new_in(count, size, Global)
    }
}

impl<A: Allocator> BufGroup<A> {
    /// Allocates the buffers and puts all of them in the ring. The ring is not
    /// registered yet.
    fn new_in(count: u16, size: u32, alloc: A) -> Result<Self, Errno> {
        if count == 0 || size == 0 || count > 1 << 15 {
            return Err(Errno::INVAL);
        }

        let layout = Self::layout(count, size).ok_or(Errno::NOMEM)?;

        let mem = alloc
            .allocate(layout)
            .unwrap_or_else(|_| handle_alloc_error(layout))
            .cast::<u8>();

        let mut group = Self {
            ring: IoBufRing::new(count),
            size,
            count,
            mem,
            users: 0,
            handles: 1,
            alloc,
        };

        for bid in (0..count).map(BufId::from_raw) {
            let buf = group.buf(bid);

            // SAFETY: the memory lives as long as the group
            unsafe { group.ring.add(bid.as_raw(), buf.as_ptr(), size) };
        }

        group.ring.advance();

        Ok(group)
    }

    /// The layout of all buffers, cache-line aligned. `None` on overflow.
    fn layout(count: u16, size: u32) -> Option<Layout> {
        let bytes = (count as usize).checked_mul(size as usize)?;
        Layout::from_size_align(bytes, 64).ok()
    }

    /// The start of buffer `bid`. `bid` must be below `count`, which holds for
    /// ids we put in the ring and so for every id the kernel hands back.
    const fn buf(&self, bid: BufId) -> NonNull<u8> {
        // SAFETY: `bid < count`, so within the allocation
        unsafe { self.mem.add(bid.as_raw() as usize * self.size as usize) }
    }

    /// Puts buffer `bid` back in the ring and makes it visible to the kernel.
    fn recycle(&mut self, bid: BufId) {
        let buf = self.buf(bid);
        // SAFETY: the memory lives as long as the group, which stays
        // registered until it is freed
        unsafe { self.ring.add(bid.as_raw(), buf.as_ptr(), self.size) };
        self.ring.advance();
    }
}

impl<A: Allocator> Drop for BufGroup<A> {
    fn drop(&mut self) {
        if let Some(layout) = Self::layout(self.count, self.size) {
            // SAFETY: allocated in `BufGroup::new_in` with this layout
            unsafe { self.alloc.deallocate(self.mem, layout) };
        }
    }
}

impl Drop for BufGroups {
    fn drop(&mut self) {
        for group in self.groups.drain() {
            // an op in flight may still pick (and write) one of its buffers,
            // or a `SelectBuf` still points into it: leak both the buffers
            // and the ring the kernel picks them from
            if group.users > 0 {
                mem::forget(group);
            }
        }
    }
}

impl BufGroups {
    pub fn new() -> Self {
        Self {
            groups: Slab::new(),
            dead: Vec::with_capacity(16),
        }
    }

    fn get(&mut self, id: BufGroupId) -> &mut BufGroup {
        self.groups.get_mut(id.as_slot())
    }

    /// Allocates a group with every buffer in its ring. It still has to be
    /// registered with [`Self::register_or_drop`].
    ///
    /// # Panics
    /// If there are already `u16::MAX` groups.
    pub fn insert(&mut self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        let group = BufGroup::new(count, size)?;

        let id = BufGroupId::from_slot(self.groups.insert(group))
            .expect("cannot create more buffer groups");

        Ok(id)
    }

    /// An op that may pick a buffer from the group was submitted.
    pub fn acquire(&mut self, id: BufGroupId) {
        self.get(id).users += 1;
    }

    /// Hands buffer `bid` out to a `SelectBuf`, which counts as a user until
    /// it is returned.
    pub fn take(&mut self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        let group = self.get(id);
        group.users += 1;
        group.buf(bid)
    }

    /// An op or `SelectBuf` is done with the group.
    pub fn release(&mut self, id: BufGroupId) {
        self.get(id).users -= 1;
        self.mark_if_dead(id);
    }

    /// Puts buffer `bid` back in the ring, unless every handle is gone.
    pub fn recycle(&mut self, id: BufGroupId, bid: BufId) {
        let group = self.get(id);
        if group.handles > 0 {
            group.recycle(bid);
        }
    }

    /// A `SelectGroup` handle was cloned.
    pub fn clone_handle(&mut self, id: BufGroupId) {
        self.get(id).handles += 1;
    }

    /// A `SelectGroup` handle was dropped.
    pub fn drop_handle(&mut self, id: BufGroupId) {
        self.get(id).handles -= 1;
        self.mark_if_dead(id);
    }

    /// Queues the group for [`Self::free_dead`] once it has neither handles
    /// nor users. Each happens exactly once, since nothing can bring either
    /// count back up after that.
    fn mark_if_dead(&mut self, id: BufGroupId) {
        let group = self.get(id);
        if group.handles == 0 && group.users == 0 {
            self.dead.push(id);
        }
    }

    /// Registers the ring of a freshly inserted group with the kernel. If that
    /// fails, the group is removed and freed right away; nothing can be using
    /// it yet.
    pub fn register_or_drop(&mut self, id: BufGroupId, ring: &IoUring) -> Result<(), Errno> {
        let buf_ring = &self.get(id).ring;

        if let Err(err) = unsafe { ring.register_buf_ring(buf_ring, id.as_raw(), 0) } {
            drop(self.groups.remove(id.as_slot()));
            Err(err)
        } else {
            Ok(())
        }
    }

    /// Unregisters and frees every dead group. Unregistering first means the
    /// kernel can't pick from a ring whose memory is gone.
    pub fn free_dead(&mut self, ring: &IoUring) {
        for gid in self.dead.drain(..) {
            let _ = unsafe { ring.unregister_buf_ring(gid.as_raw()) };
            drop(self.groups.remove(gid.as_slot()))
        }
    }
}
