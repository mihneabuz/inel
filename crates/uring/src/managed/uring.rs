use core::{
    alloc::Layout, num::NonZeroU16, ops::Deref, pin::Pin, ptr::NonNull, slice, task::Waker,
};
use std::alloc;

use rustix::io::Errno;

use crate::{
    managed::{
        cancellation::Cancellation,
        completion::{CompletionEntry, CompletionResult, Completions, Key},
        submission::{AsyncCancel, DetachOp, MultiOp, Op, SingleOp},
    },
    sys::{BufRing, IoUring},
    utils::Slab,
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

    buf_groups: BufGroups,
}

impl Uring {
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
            buf_groups: BufGroups::new(),
        })
    }

    pub const fn is_done(&self) -> bool {
        self.comp.is_empty()
    }

    pub fn submit<O: SingleOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut key = self.comp.insert_single(waker);
        self.push(op, &mut key);
        key
    }

    pub fn submit_multi<O: MultiOp>(&mut self, op: Pin<&mut O>, waker: Waker) -> Key {
        let mut key = self.comp.insert_multi(waker);
        self.push(op, &mut key);
        key
    }

    fn push<O: Op>(&mut self, op: Pin<&mut O>, key: &mut Key) {
        // SAFETY: Op implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep(sqe);
                    sqe.user_data(key.add_group_id(sqe).as_raw());
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR);
        }

        if let Some(group) = key.group_id() {
            self.buf_groups.acquire(group);
        }
    }

    pub fn submit_detached<O: DetachOp>(&mut self, mut op: O) {
        // SAFETY: DetachOp implementation must guarantee safety
        unsafe {
            self.ring
                .push_sqe(|sqe| {
                    op.prep_detached(sqe);
                    sqe.user_data(Key::IGNORE.as_raw());
                    sqe.skip_success_cqe();
                })
                .expect(Self::SUBMISSION_QUEUE_FULL_ERROR)
        }
    }

    pub fn update(&mut self, key: Key, waker: Waker) {
        self.comp.update(key, waker);
    }

    pub fn cancel(&mut self, key: Key, handle: Cancellation, entry: bool) {
        let pending = self.comp.cancel(key, handle, |result| {
            if let Some(bid) = result.buffer_id()
                && let Some(gid) = key.group_id()
            {
                self.buf_groups.recycle(gid, bid);
            }
        });

        if !pending {
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
            let completion = CompletionEntry::from_cqe(cqe);
            if completion.should_ignore() {
                return;
            }
            let key = completion.key();

            let result = completion.result();
            let stored = self.comp.notify(key, result);

            if let Some(group) = key.group_id() {
                if !stored && let Some(bid) = result.buffer_id() {
                    self.buf_groups.recycle(group, bid);
                }

                if !result.has_more() {
                    self.buf_groups.release(group);
                }
            }
        });

        self.buf_groups.free_dead(&self.ring);
    }

    fn create_buf_group(&mut self, count: u16, size: u32) -> Result<BufGroupId, Errno> {
        let (id, buf_ring) = self.buf_groups.insert(count, size)?;
        // SAFETY: the ring is unregistered in `free_dead` before it's dropped
        if let Err(err) = unsafe { self.ring.register_buf_ring(buf_ring, id.raw(), 0) } {
            self.buf_groups.remove(id);
            return Err(err);
        }
        Ok(id)
    }

    fn destroy_buf_group(&mut self, id: BufGroupId) {
        self.buf_groups.kill(id);
        self.buf_groups.free_dead(&self.ring);
    }

    fn take_buf(&mut self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        self.buf_groups.take(id, bid)
    }

    fn return_buf(&mut self, id: BufGroupId, bid: BufId) {
        self.buf_groups.recycle(id, bid);
        self.buf_groups.release(id);
        self.buf_groups.free_dead(&self.ring);
    }

    pub(crate) fn recycle_buf(&mut self, id: BufGroupId, bid: BufId) {
        self.buf_groups.recycle(id, bid);
    }
}

impl Drop for Uring {
    fn drop(&mut self) {
        let _ = self.ring.unregister_buffers();
        let _ = self.ring.unregister_files();
    }
}

/// The kernel's buffer group id. Never 0, so `Option<BufGroupId>` is as small
/// as the id and packs into a `Key` without a separate flag.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct BufGroupId(NonZeroU16);

impl BufGroupId {
    pub const fn new(raw: u16) -> Option<Self> {
        match NonZeroU16::new(raw) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// The id the kernel knows the group by.
    pub const fn raw(self) -> u16 {
        self.0.get()
    }

    /// The group's slot in `BufGroups`, one less than the id.
    const fn slot(self) -> u32 {
        self.raw() as u32 - 1
    }
}

pub type BufId = u16;

/// A group of `count` fixed-size buffers the kernel picks from when a
/// buffer-select op runs. Buffers are handed out as [`GroupBuf`]s and go back
/// into the group when those are dropped.
///
/// The memory is owned by the ring and outlives the handle: it is only freed
/// once the handle is dropped, no op that may still pick a buffer is in
/// flight, and every [`GroupBuf`] is gone.
pub struct BufGroup<U: UringProxy> {
    id: BufGroupId,
    size: u32,
    ring: U,
}

impl<U: UringProxy> BufGroup<U> {
    /// Allocates `count` buffers of `size` bytes each and registers them with
    /// the ring. Use several groups for several buffer sizes.
    pub fn new(count: u16, size: u32, ring: U) -> Result<Self, Errno> {
        let id = ring.with(|ring| ring.create_buf_group(count, size))?;
        Ok(Self { id, size, ring })
    }

    pub const fn buffer_size(&self) -> u32 {
        self.size
    }

    pub(crate) const fn id(&self) -> BufGroupId {
        self.id
    }

    pub(crate) const fn ring(&self) -> &U {
        &self.ring
    }
}

impl<U: UringProxy> Drop for BufGroup<U> {
    fn drop(&mut self) {
        self.ring.with(|ring| ring.destroy_buf_group(self.id));
    }
}

/// A buffer picked by the kernel from a [`BufGroup`], holding the bytes it
/// read. Dropping it returns the buffer to the group.
pub struct GroupBuf<U: UringProxy> {
    ring: U,
    id: BufGroupId,
    bid: BufId,
    ptr: NonNull<u8>,
    len: usize,
}

impl<U: UringProxy> GroupBuf<U> {
    /// # Safety
    /// The kernel must have filled `len` bytes of buffer `bid`, and be done
    /// with it.
    pub(crate) unsafe fn new(ring: U, id: BufGroupId, bid: BufId, len: usize) -> Self {
        let ptr = ring.with(|ring| ring.take_buf(id, bid));
        Self {
            ring,
            id,
            bid,
            ptr,
            len,
        }
    }
}

impl<U: UringProxy> Deref for GroupBuf<U> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        // SAFETY: the buffer is ours until dropped, and the memory stays
        // allocated while we hold it
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<U: UringProxy> Drop for GroupBuf<U> {
    fn drop(&mut self) {
        self.ring.with(|ring| ring.return_buf(self.id, self.bid));
    }
}

/// The ring-side state of every buffer group.
pub struct BufGroups {
    groups: Slab<Group>,
    /// Dead groups without users, to unregister and free.
    dead: Vec<BufGroupId>,
}

struct Group {
    ring: BufRing,
    /// `count * size` bytes. Only ever accessed through raw pointers, since
    /// the kernel and `GroupBuf`s use parts of it concurrently.
    mem: NonNull<u8>,
    layout: Layout,
    size: u32,
    /// In-flight ops that may pick a buffer, plus live `GroupBuf`s.
    users: u32,
    /// False once the `BufGroup` handle is dropped.
    alive: bool,
}

impl Group {
    const fn buf(&self, bid: BufId) -> NonNull<u8> {
        // SAFETY: `bid < count`, so within the allocation
        unsafe { self.mem.add(bid as usize * self.size as usize) }
    }

    fn recycle(&mut self, bid: BufId) {
        let buf = self.buf(bid);
        // SAFETY: the memory lives as long as the group, which stays
        // registered until it is freed
        unsafe { self.ring.add(bid, buf.as_ptr(), self.size) };
        self.ring.advance();
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        // SAFETY: allocated in `BufGroups::insert` with this layout
        unsafe { alloc::dealloc(self.mem.as_ptr(), self.layout) };
    }
}

impl BufGroups {
    pub const fn new() -> Self {
        Self {
            groups: Slab::new(),
            dead: Vec::new(),
        }
    }

    fn get(&mut self, id: BufGroupId) -> &mut Group {
        self.groups.get_mut(id.slot())
    }

    /// Allocates a group with every buffer in its ring. It still has to be
    /// registered with the kernel.
    pub fn insert(&mut self, count: u16, size: u32) -> Result<(BufGroupId, &BufRing), Errno> {
        if count == 0 || size == 0 || count > 1 << 15 {
            return Err(Errno::INVAL);
        }

        let bytes = (count as usize)
            .checked_mul(size as usize)
            .ok_or(Errno::NOMEM)?;

        let layout = Layout::from_size_align(bytes, 64).map_err(|_| Errno::NOMEM)?;

        // SAFETY: `bytes` is non-zero
        let mem = NonNull::new(unsafe { alloc::alloc(layout) })
            .unwrap_or_else(|| alloc::handle_alloc_error(layout));

        let mut group = Group {
            ring: BufRing::new(count),
            mem,
            layout,
            size,
            users: 0,
            alive: true,
        };

        for bid in 0..count {
            let buf = group.buf(bid);
            // SAFETY: the memory lives as long as the group
            unsafe { group.ring.add(bid, buf.as_ptr(), size) };
        }

        group.ring.advance();

        let slot = self.groups.insert(group);
        // the id is the slot plus one, since 0 is never used
        let Some(id) = u16::try_from(slot + 1).ok().and_then(BufGroupId::new) else {
            drop(self.groups.remove(slot));
            return Err(Errno::NOSPC);
        };
        Ok((id, &self.groups.get_mut(slot).ring))
    }

    /// Frees a group that never got registered.
    pub fn remove(&mut self, id: BufGroupId) {
        drop(self.groups.remove(id.slot()));
    }

    /// An op that may pick a buffer from the group was submitted.
    pub fn acquire(&mut self, id: BufGroupId) {
        self.get(id).users += 1;
    }

    /// An op or `GroupBuf` is done with the group.
    pub fn release(&mut self, id: BufGroupId) {
        let group = self.get(id);
        group.users -= 1;
        if !group.alive && group.users == 0 {
            self.dead.push(id);
        }
    }

    /// Hands buffer `bid` out to a `GroupBuf`.
    pub fn take(&mut self, id: BufGroupId, bid: BufId) -> NonNull<u8> {
        let group = self.get(id);
        group.users += 1;
        group.buf(bid)
    }

    /// Puts buffer `bid` back in the ring, unless the group is dead.
    pub fn recycle(&mut self, id: BufGroupId, bid: BufId) {
        let group = self.get(id);
        if group.alive {
            group.recycle(bid);
        }
    }

    /// The handle was dropped; the group is freed once it has no users.
    pub fn kill(&mut self, id: BufGroupId) {
        let group = self.get(id);
        group.alive = false;
        if group.users == 0 {
            self.dead.push(id);
        }
    }

    /// Unregisters and frees dead groups without users.
    pub fn free_dead(&mut self, ring: &IoUring) {
        for id in self.dead.drain(..) {
            // SAFETY: no op that may pick a buffer is in flight, and no
            // buffer is handed out
            let _ = unsafe { ring.unregister_buf_ring(id.raw()) };
            drop(self.groups.remove(id.slot()));
        }
    }
}
