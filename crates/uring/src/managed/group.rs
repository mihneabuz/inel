use core::{mem, slice};
use std::{
    alloc::{Layout, handle_alloc_error},
    cell::{Cell, RefCell},
    num::NonZeroU16,
    ops::Deref,
    ptr::NonNull,
    rc::Rc,
};

use allocator_api2::alloc::{Allocator, Global};
use rustix::io::Errno;

use crate::{
    managed::{Cancellation, CompletionResult, UringProxy, UringProxyExt, completion::Key},
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
/// tasks without an `Rc`. Once every handle (including the ones held by ops
/// that may still pick a buffer) and every [`SelectBuf`] is gone, the ring
/// unregisters and frees the group.
#[derive(Clone)]
pub struct SelectGroup {
    handle: Rc<BufGroupHandle>,
}

impl SelectGroup {
    /// Allocates `count` buffers of `size` bytes each and registers them with
    /// `uring`'s ring as a new group.
    ///
    /// Fails with `EINVAL` if `count` or `size` is 0 or `count` is above
    /// 32768, with `ENOMEM` if the total size overflows, or with whatever the
    /// kernel returns when registering.
    ///
    /// # Panics
    /// If the ring already has `u16::MAX` groups.
    pub fn new<U: UringProxy>(count: u16, size: u32, uring: &U) -> Result<Self, Errno> {
        uring.create_buf_group(count, size)
    }

    /// The size of every buffer in the group, and so the most a single
    /// buffer-select read can return.
    pub fn buffer_size(&self) -> u32 {
        self.handle.size
    }

    pub(crate) fn id(&self) -> BufGroupId {
        self.handle.gid
    }

    /// Takes ownership of buffer `bid`, which holds `len` bytes the kernel
    /// wrote. Dropping the returned buffer puts it back in the ring.
    ///
    /// # Safety
    /// `bid` must come from a CQE of an op that selected from this group, and
    /// be taken only once per CQE. `len` must be at most
    /// [`Self::buffer_size`], and the first `len` bytes must be initialized,
    /// i.e. `len` is the CQE's result (or 0).
    pub(crate) unsafe fn take(self, bid: BufId, len: usize) -> SelectBuf {
        SelectBuf {
            ptr: self.handle.buf(bid),
            handle: self.handle,
            bid,
            len,
        }
    }
}

/// Keeps the group alive until the cancelled op's final CQE, since the kernel
/// may still pick a buffer from it until then.
impl From<SelectGroup> for Cancellation {
    fn from(group: SelectGroup) -> Self {
        group.handle.into()
    }
}

/// A buffer picked by the kernel from a [`SelectGroup`], holding the bytes it
/// read. Dropping it returns the buffer to the group.
pub struct SelectBuf {
    handle: Rc<BufGroupHandle>,
    bid: BufId,
    /// The start of the buffer, kept here so reading it doesn't go through
    /// the `Rc`.
    ptr: NonNull<u8>,
    len: usize,
}

impl SelectBuf {
    /// Returns the buffer to its group, so the kernel can pick it again. Same
    /// as dropping it.
    pub fn release(self) {
        let _ = self;
    }
}

impl Deref for SelectBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: the buffer is ours until dropped, the memory stays allocated
        // while we hold the group, and its first `len` bytes were written
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for SelectBuf {
    fn drop(&mut self) {
        self.handle.group.recycle(self.bid);
    }
}

/// What every user of a group shares: [`SelectGroup`]s, the ops and
/// [`Cancellation`]s holding one, and [`SelectBuf`]s. Once the last of them is
/// gone, no op can pick from the group anymore, so dropping it queues the
/// group to be unregistered and freed by the ring.
///
/// Copies the fields ops need, so building an SQE or taking a buffer doesn't
/// go through to the [`BufGroup`].
struct BufGroupHandle {
    gid: BufGroupId,
    size: u32,
    mem: NonNull<u8>,

    group: Rc<BufGroup>,
    dead: Rc<DeadList>,
}

impl BufGroupHandle {
    /// The start of buffer `bid`. Same as [`BufGroup::buf`].
    const fn buf(&self, bid: BufId) -> NonNull<u8> {
        // SAFETY: see `BufGroup::buf`
        unsafe { self.mem.add(bid.as_raw() as usize * self.size as usize) }
    }
}

impl Drop for BufGroupHandle {
    fn drop(&mut self) {
        self.dead.push(self.gid);
    }
}

/// One group: its buffer ring and the memory of all its buffers.
struct BufGroup<A: Allocator = Global> {
    ring: IoBufRing,

    count: u16,
    size: u32,

    /// `count * size` bytes, buffer `bid` at offset `bid * size`.
    mem: NonNull<u8>,
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

        let group = Self {
            ring: IoBufRing::new(count),
            count,
            size,
            mem,
            alloc,
        };

        for bid in (0..count).map(BufId::from_raw) {
            group.recycle(bid);
        }

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
    fn recycle(&self, bid: BufId) {
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

/// List of gids for groups whose last [`BufGroupHandle`] was dropped.
///
/// Once it is in this list, we know there are no more outside users of this group, so
/// we can unregister it and free the memory.
struct DeadList {
    queue: RefCell<Vec<BufGroupId>>,
    empty: Cell<bool>,
}

impl DeadList {
    const fn new() -> Self {
        Self {
            queue: RefCell::new(Vec::new()),
            empty: Cell::new(true),
        }
    }

    /// Mark the group with `gid` as dead.
    fn push(&self, gid: BufGroupId) {
        self.queue.borrow_mut().push(gid);
        self.empty.set(false);
    }

    /// Calls `f` on every queued group, emptying the queue.
    fn pop_for_each<F>(&self, mut f: F)
    where
        F: FnMut(BufGroupId),
    {
        if self.empty.replace(true) {
            return;
        }

        let mut queue = self.queue.borrow_mut();
        for gid in queue.drain(..) {
            f(gid)
        }
    }
}

/// Registry of active buffer groups.
///
/// The groups slab is keyed by the gid, and the dead list contains the gid of groups
/// whose last [`BufGroupHandle`] was dropped, meaning no outside users exist anymore.
pub struct BufGroups {
    groups: Slab<Rc<BufGroup>>,
    dead: Rc<DeadList>,
}

impl BufGroups {
    pub fn new() -> Self {
        Self {
            groups: Slab::new(),
            dead: Rc::new(DeadList::new()),
        }
    }

    /// Allocates and registers a new group with `count` number of `size` buffers.
    ///
    /// # Panics
    /// If there are already `u16::MAX` groups.
    pub fn create(&mut self, count: u16, size: u32, ring: &IoUring) -> Result<SelectGroup, Errno> {
        let entry = self.groups.vacant_entry();
        let gid = BufGroupId::from_slot(entry.key()).expect("cannot create more buffer groups");

        let group = BufGroup::new(count, size)?;

        // SAFETY: the group is only freed by `Self::cleanup_dead`, which
        // unregisters it first
        unsafe { ring.register_buf_ring(&group.ring, gid.as_raw(), 0)? };

        let group = entry.insert(Rc::new(group)).clone();

        let handle = BufGroupHandle {
            gid,
            size,
            mem: group.mem,
            group,
            dead: self.dead.clone(),
        };

        Ok(SelectGroup {
            handle: Rc::new(handle),
        })
    }

    /// Puts the buffer of a result nobody will read (its op was cancelled)
    /// back in the ring.
    pub fn handle_lost(&mut self, key: &Key, result: &CompletionResult) {
        if let (Some(gid), Some(bid)) = (key.group_id(), result.buffer_id())
            && let Some(group) = self.groups.get_mut(gid.as_slot())
        {
            group.recycle(bid);
        }
    }

    /// Unregisters and frees every group whose last handle was dropped.
    ///
    /// Must not run between a cancelled op's final CQE dropping its handle and
    /// [`Self::handle_lost`] recycling that CQE's buffer.
    pub fn cleanup_dead(&mut self, ring: &IoUring) {
        self.dead.pop_for_each(|gid| {
            let group = self.groups.remove(gid.as_slot());
            debug_assert_eq!(Rc::strong_count(&group), 1);

            // SAFETY: no handle is left, so no op can pick a buffer from it
            // and no `SelectBuf` holds one
            let _ = unsafe { ring.unregister_buf_ring(gid.as_raw()) };
        })
    }

    /// Frees every dead group, and leaks the rest: both the buffers and the
    /// ring the kernel picks them from, since an op still in flight may hold a
    /// handle and pick (and write) one of its buffers.
    pub fn leak_alive(&mut self, ring: &IoUring) {
        self.cleanup_dead(ring);
        for group in self.groups.drain() {
            mem::forget(group);
        }
    }
}
