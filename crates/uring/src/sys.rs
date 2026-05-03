use core::{
    alloc::Layout,
    ffi::{c_char, c_void},
    mem, ptr,
    ptr::NonNull,
    result::Result,
    sync::atomic::{self, AtomicU16, AtomicU32, Ordering},
};

use allocator_api2::alloc::{Allocator, Global};

use rustix::{
    fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
    fs::{OFlags, Timespec},
    io::{Errno, ReadWriteFlags},
    io_uring::*,
    param::page_size,
};

pub use crate::utils::Mmap;

const REQUIRED_FEATURES: IoringFeatureFlags = IoringFeatureFlags::SINGLE_MMAP;

pub struct Sqe(io_uring_sqe);

impl Sqe {
    pub(crate) const fn init(&mut self) {
        self.0.flags = IoringSqeFlags::empty();
        self.0.ioprio.ioprio = 0;
        self.0.op_flags.rw_flags = ReadWriteFlags::empty();
        self.0.buf.buf_index = 0;
        self.0.personality = 0;
        self.0.splice_fd_in_or_file_index_or_addr_len.file_index = 0;
        self.0.addr3_or_cmd.addr3.addr3 = 0;
    }

    const fn add_flag(&mut self, flag: IoringSqeFlags) {
        self.0.flags = self.0.flags.union(flag);
    }

    pub const fn user_data(&mut self, user_data: u64) {
        self.0.user_data.u64_ = user_data;
    }

    pub const fn link(&mut self) {
        self.add_flag(IoringSqeFlags::IO_LINK);
    }

    pub const fn hardlink(&mut self) {
        self.add_flag(IoringSqeFlags::IO_HARDLINK);
    }

    pub const fn drain(&mut self) {
        self.add_flag(IoringSqeFlags::IO_DRAIN);
    }

    pub const fn force_async(&mut self) {
        self.add_flag(IoringSqeFlags::ASYNC);
    }

    pub const fn skip_success_cqe(&mut self) {
        self.add_flag(IoringSqeFlags::CQE_SKIP_SUCCESS);
    }

    pub const fn get_buf_index(&self) -> u16 {
        unsafe { self.0.buf.buf_index }
    }

    const fn prep_rw(
        &mut self,
        op: IoringOp,
        fd: RawFd,
        addr: *const c_void,
        len: u32,
        offset: u64,
    ) {
        self.0.opcode = op;
        self.0.fd = fd;
        self.0.addr_or_splice_off_in.addr = io_uring_ptr::new(addr.cast_mut());
        self.0.len.len = len;
        self.0.off_or_addr2.off = offset;
    }

    pub const fn prep_nop(&mut self) {
        self.prep_rw(IoringOp::Nop, -1, ptr::null(), 0, 0);
    }

    pub const fn prep_timeout(
        &mut self,
        ts: *const Timespec,
        count: u32,
        flags: IoringTimeoutFlags,
    ) {
        self.prep_rw(IoringOp::Timeout, -1, ts.cast(), 1, count as u64);
        self.0.op_flags.timeout_flags = flags;
    }

    pub const fn prep_cancel(&mut self, user_data: u64, flags: IoringAsyncCancelFlags) {
        self.prep_rw(IoringOp::AsyncCancel, -1, ptr::null(), 0, 0);
        self.0.addr_or_splice_off_in.splice_off_in = user_data;
        self.0.op_flags.cancel_flags = flags;
    }

    pub const fn prep_openat(&mut self, dfd: RawFd, path: *const c_char, flags: OFlags, mode: u32) {
        self.prep_rw(IoringOp::Openat, dfd, path.cast(), mode, 0);
        self.0.op_flags.open_flags = flags;
    }

    pub const fn prep_read(&mut self, fd: RawFd, buf: *mut u8, nbytes: u32, offset: u64) {
        self.prep_rw(IoringOp::Read, fd, buf.cast(), nbytes, offset);
    }

    pub const fn prep_write(&mut self, fd: RawFd, buf: *const u8, nbytes: u32, offset: u64) {
        self.prep_rw(IoringOp::Write, fd, buf.cast(), nbytes, offset);
    }
}

struct SubmissionQueue {
    pub(crate) head: u32,
    pub(crate) tail: u32,

    pub(crate) khead: NonNull<AtomicU32>,
    pub(crate) ktail: NonNull<AtomicU32>,

    pub(crate) mask: u32,
    pub(crate) entries: u32,

    pub(crate) flags: NonNull<AtomicU32>,

    pub(crate) sqes: NonNull<Sqe>,
}

impl SubmissionQueue {
    pub fn new(ring: ptr::NonNull<u8>, sqes: ptr::NonNull<u8>, p: &io_uring_params) -> Self {
        unsafe {
            let khead = ring.offset(p.sq_off.head as isize).cast::<AtomicU32>();
            assert!(khead.is_aligned(), "misaligned sq khead pointer");

            let ktail = ring.offset(p.sq_off.tail as isize).cast::<AtomicU32>();
            assert!(ktail.is_aligned(), "misaligned sq ktail pointer");

            let mask_ptr = ring.offset(p.sq_off.ring_mask as isize).cast::<u32>();
            assert!(mask_ptr.is_aligned(), "misaligned sq mask pointer");

            let entries_ptr = ring.offset(p.sq_off.ring_entries as isize).cast::<u32>();
            assert!(entries_ptr.is_aligned(), "misaligned sq entries pointer");

            let flags = ring.offset(p.sq_off.flags as isize).cast::<AtomicU32>();
            assert!(flags.is_aligned(), "misaligned sq flags pointer");

            let sqes = sqes.cast::<Sqe>();
            assert!(sqes.is_aligned(), "misaligned sq array pointer");

            let mask = mask_ptr.read();
            let entries = entries_ptr.read();

            Self {
                head: 0,
                tail: 0,
                khead,
                ktail,
                mask,
                entries,
                flags,
                sqes,
            }
        }
    }
}

pub struct Cqe(io_uring_cqe);

impl Cqe {
    pub const fn user_data(&self) -> u64 {
        unsafe { self.0.user_data.u64_ }
    }

    pub const fn result(&self) -> i32 {
        self.0.res
    }

    pub const fn flags(&self) -> IoringCqeFlags {
        self.0.flags
    }

    pub const fn has_more(&self) -> bool {
        self.0.flags.contains(IoringCqeFlags::MORE)
    }

    pub const fn is_notif(&self) -> bool {
        self.0.flags.contains(IoringCqeFlags::NOTIF)
    }

    pub fn buffer_id(&self) -> Option<u16> {
        self.0
            .flags
            .contains(IoringCqeFlags::BUFFER)
            .then_some((self.0.flags.bits() >> IORING_CQE_BUFFER_SHIFT) as u16)
    }

    pub const fn sock_nonempty(&self) -> bool {
        self.0.flags.contains(IoringCqeFlags::SOCK_NONEMPTY)
    }
}

struct CompletionQueue {
    pub(crate) khead: NonNull<AtomicU32>,
    pub(crate) ktail: NonNull<AtomicU32>,

    pub(crate) mask: u32,
    pub(crate) entries: u32,

    pub(crate) cqes: NonNull<Cqe>,
}

impl CompletionQueue {
    pub fn new(ring: ptr::NonNull<u8>, p: &io_uring_params) -> Self {
        unsafe {
            let khead = ring.offset(p.cq_off.head as isize).cast::<AtomicU32>();
            assert!(khead.is_aligned(), "misaligned cq khead pointer");

            let ktail = ring.offset(p.cq_off.tail as isize).cast::<AtomicU32>();
            assert!(ktail.is_aligned(), "misaligned cq ktail pointer");

            let mask_ptr = ring.offset(p.cq_off.ring_mask as isize).cast::<u32>();
            assert!(mask_ptr.is_aligned(), "misaligned cq mask pointer");

            let entries_ptr = ring.offset(p.cq_off.ring_entries as isize).cast::<u32>();
            assert!(entries_ptr.is_aligned(), "misaligned cq entries pointer");

            let cqes = ring.offset(p.cq_off.cqes as isize).cast::<Cqe>();
            assert!(cqes.is_aligned(), "misaligned cq array pointer");

            let mask = mask_ptr.read();
            let entries = entries_ptr.read();

            Self {
                khead,
                ktail,
                mask,
                entries,
                cqes,
            }
        }
    }
}

#[allow(unused)]
struct Backing {
    rings: Mmap,
    sqes: Mmap,
}

pub struct IoUringOptions<'a> {
    sq_entries: u32,
    cq_entries: u32,
    share_wq: Option<BorrowedFd<'a>>,
    flags: IoringSetupFlags,
}

impl Default for IoUringOptions<'_> {
    fn default() -> Self {
        Self {
            sq_entries: 128,
            cq_entries: 128,
            share_wq: None,
            flags: IoringSetupFlags::NO_SQARRAY | IoringSetupFlags::CLAMP,
        }
    }
}

impl<'a> IoUringOptions<'a> {
    #[must_use]
    pub const fn with_sq_entries(mut self, entries: u32) -> Self {
        self.sq_entries = entries;
        self
    }

    #[must_use]
    pub const fn with_cq_entries(mut self, entries: u32) -> Self {
        self.cq_entries = entries;
        self
    }

    #[must_use]
    pub const fn with_single_issuer(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::SINGLE_ISSUER);
        self
    }

    #[must_use]
    pub const fn with_coop_taskrun(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::COOP_TASKRUN);
        self
    }

    #[must_use]
    pub const fn with_defer_taskrun(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::DEFER_TASKRUN);
        self
    }

    #[must_use]
    pub const fn with_submit_all(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::SUBMIT_ALL);
        self
    }

    #[must_use]
    pub const fn with_taskrun_flag(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::TASKRUN_FLAG);
        self
    }

    #[must_use]
    pub const fn with_attach_wq(mut self, other: BorrowedFd<'a>) -> Self {
        self.share_wq = Some(other);
        self
    }

    #[must_use]
    pub const fn with_sqpoll(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::SQPOLL);
        self
    }

    #[must_use]
    pub const fn with_iopoll(mut self) -> Self {
        self.flags = self.flags.union(IoringSetupFlags::IOPOLL);
        self
    }

    pub fn build(self) -> Result<IoUring, Errno> {
        let mut p: io_uring_params = unsafe { mem::zeroed() };

        p.flags = self.flags;

        if self.cq_entries > self.sq_entries {
            p.flags |= IoringSetupFlags::CQSIZE;
            p.cq_entries = self.cq_entries;
        }

        if let Some(other) = self.share_wq {
            p.flags |= IoringSetupFlags::ATTACH_WQ;
            p.wq_fd = other.as_raw_fd();
        }

        let fd = unsafe { system::io_uring_setup(self.sq_entries, &mut p) }?;

        if !p.features.contains(REQUIRED_FEATURES) {
            return Err(Errno::INVAL);
        }

        let maps = unsafe { system::io_uring_map_rings(&fd, &p) }?;

        let sq = SubmissionQueue::new(maps.rings.addr(), maps.sqes.addr(), &p);
        let cq = CompletionQueue::new(maps.rings.addr(), &p);

        Ok(IoUring {
            fd,
            sq,
            cq,
            setup: p.flags,
            memory: maps,
        })
    }
}

pub struct IoUring {
    fd: system::OwnedFd,

    sq: SubmissionQueue,
    cq: CompletionQueue,

    setup: IoringSetupFlags,

    #[allow(unused)]
    memory: Backing,
}

impl AsFd for IoUring {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl IoUring {
    pub fn new() -> Result<Self, Errno> {
        Self::options().build()
    }

    #[must_use]
    pub fn options<'a>() -> IoUringOptions<'a> {
        IoUringOptions::default()
    }

    pub const fn sq_size(&self) -> u32 {
        self.sq.entries
    }

    pub const fn cq_size(&self) -> u32 {
        self.cq.entries
    }

    pub fn submit(&mut self) -> Result<u32, Errno> {
        self.submit_and_wait(0)
    }

    pub fn submit_and_wait(&mut self, want: u32) -> Result<u32, Errno> {
        let pending = self.sq_flush();

        let sq_needs_enter = self.sq_ring_needs_enter(pending);
        let cq_needs_enter = self.cq_ring_needs_enter();

        let submitted = if sq_needs_enter || cq_needs_enter || want > 0 {
            let mut flags = IoringEnterFlags::empty();

            if self.is_sqpoll() && sq_needs_enter {
                flags |= IoringEnterFlags::SQ_WAKEUP;
            }

            if cq_needs_enter || want > 0 {
                flags |= IoringEnterFlags::GETEVENTS;
            }

            unsafe { system::io_uring_enter(&self.fd, pending, want, flags) }?
        } else {
            0
        };

        Ok(submitted)
    }

    pub unsafe fn push_sqe<F>(&mut self, build: F) -> Result<(), Errno>
    where
        F: FnOnce(&mut Sqe),
    {
        self.get_next_sqe().map(build).ok_or(Errno::NOBUFS)
    }

    fn get_next_sqe(&mut self) -> Option<&mut Sqe> {
        let khead = self.sq_load_khead();
        let sq = &mut self.sq;

        if sq.tail.wrapping_sub(khead) >= sq.entries {
            return None;
        }

        let sqe = unsafe { sq.sqes.add((sq.tail & sq.mask) as usize).as_mut() };

        sq.tail = sq.tail.wrapping_add(1);

        sqe.init();

        Some(sqe)
    }

    pub fn for_each_cqe<F>(&mut self, mut reap: F)
    where
        F: FnMut(&Cqe),
    {
        let mut khead = self.cq_load_khead();
        let ktail = self.cq_load_ktail();

        let cq = &mut self.cq;

        let mut count = 0;
        while khead != ktail {
            let cqe = unsafe { cq.cqes.add((khead & cq.mask) as usize).as_ref() };
            reap(cqe);
            khead = khead.wrapping_add(1);
            count += 1;
        }

        self.cq_advance(count);
    }

    fn sq_flush(&mut self) -> u32 {
        if self.sq.head != self.sq.tail {
            self.sq.head = self.sq.tail;
            self.sq_store_ktail();
        }

        self.sq.tail.wrapping_sub(self.sq_load_khead())
    }

    fn sq_load_khead(&self) -> u32 {
        let ordering = if self.is_sqpoll() {
            Ordering::Acquire
        } else {
            Ordering::Relaxed
        };

        unsafe { self.sq.khead.as_ref().load(ordering) }
    }

    fn sq_store_ktail(&self) {
        let ordering = if self.is_sqpoll() {
            Ordering::Release
        } else {
            Ordering::Relaxed
        };

        unsafe { self.sq.ktail.as_ref().store(self.sq.tail, ordering) };
    }

    fn sq_load_kflags(&self) -> IoringSqFlags {
        IoringSqFlags::from_bits_retain(unsafe { self.sq.flags.as_ref().load(Ordering::Relaxed) })
    }

    fn sq_ring_needs_enter(&self, pending: u32) -> bool {
        if self.is_sqpoll() {
            atomic::fence(Ordering::SeqCst);
            pending > 0 && self.sq_thread_needs_wakeup()
        } else {
            pending > 0
        }
    }

    fn sq_thread_needs_wakeup(&self) -> bool {
        self.sq_load_kflags().contains(IoringSqFlags::NEED_WAKEUP)
    }

    fn cq_load_khead(&self) -> u32 {
        unsafe { self.cq.khead.as_ref().load(Ordering::Relaxed) }
    }

    fn cq_load_ktail(&self) -> u32 {
        unsafe { self.cq.ktail.as_ref().load(Ordering::Acquire) }
    }

    fn cq_ring_needs_enter(&self) -> bool {
        self.has_defer_taskrun() || self.cq_ring_needs_flush()
    }

    fn cq_ring_needs_flush(&self) -> bool {
        self.sq_load_kflags()
            .intersects(IoringSqFlags::TASKRUN | IoringSqFlags::CQ_OVERFLOW)
    }

    fn cq_advance(&self, count: u32) {
        unsafe {
            self.cq.khead.as_ref().store(
                self.cq.khead.as_ref().load(Ordering::Relaxed) + count,
                Ordering::Release,
            )
        };
    }

    const fn is_sqpoll(&self) -> bool {
        self.setup.contains(IoringSetupFlags::SQPOLL)
    }

    const fn has_defer_taskrun(&self) -> bool {
        self.setup.contains(IoringSetupFlags::DEFER_TASKRUN)
    }

    unsafe fn do_register(
        &self,
        op: IoringRegisterOp,
        arg: *const c_void,
        nr_args: u32,
    ) -> Result<u32, Errno> {
        unsafe { system::io_uring_register(&self.fd, op, arg, nr_args) }
    }

    /// # Safety
    /// The memory referenced by `iovecs` must remain valid until the buffers
    /// are unregistered (or the ring is closed).
    pub unsafe fn register_buffers(&self, iovecs: &[iovec]) -> Result<u32, Errno> {
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterBuffers,
                iovecs.as_ptr().cast(),
                iovecs.len() as u32,
            )
        }
    }

    pub fn register_buffers_sparse(&self, nr: u32) -> Result<u32, Errno> {
        let mut reg = io_uring_rsrc_register::default();
        reg.nr = nr;
        reg.flags = IoringRsrcFlags::REGISTER_SPARSE;
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterBuffers2,
                ptr::from_ref(&reg).cast(),
                mem::size_of::<io_uring_rsrc_register>() as u32,
            )
        }
    }

    /// # Safety
    /// The memory referenced by `iovecs` must remain valid until the buffers
    /// are unregistered (or the ring is closed).
    pub unsafe fn register_buffers_update(
        &self,
        offset: u32,
        iovecs: &[iovec],
    ) -> Result<u32, Errno> {
        let mut up = io_uring_rsrc_update2::default();
        up.offset = offset;
        up.data = io_uring_ptr::new(iovecs.as_ptr().cast_mut().cast());
        up.nr = iovecs.len() as u32;
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterBuffersUpdate,
                ptr::from_ref(&up).cast(),
                mem::size_of::<io_uring_rsrc_update2>() as u32,
            )
        }
    }

    pub fn unregister_buffers(&self) -> Result<u32, Errno> {
        unsafe { self.do_register(IoringRegisterOp::UnregisterBuffers, ptr::null(), 0) }
    }

    /// # Safety
    /// `fds` must reference open file descriptors. The slice itself only needs
    /// to live for the duration of the call (the kernel copies the values).
    pub unsafe fn register_files(&self, fds: &[RawFd]) -> Result<u32, Errno> {
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFiles,
                fds.as_ptr().cast(),
                fds.len() as u32,
            )
        }
    }

    pub fn register_files_sparse(&self, nr: u32) -> Result<u32, Errno> {
        let mut reg = io_uring_rsrc_register::default();
        reg.nr = nr;
        reg.flags = IoringRsrcFlags::REGISTER_SPARSE;
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFiles2,
                ptr::from_ref(&reg).cast(),
                mem::size_of::<io_uring_rsrc_register>() as u32,
            )
        }
    }

    /// # Safety
    /// `fds` must reference open file descriptors for the duration of the call.
    /// Returns the number of files actually updated.
    pub unsafe fn register_files_update(&self, offset: u32, fds: &[RawFd]) -> Result<u32, Errno> {
        let mut up = io_uring_rsrc_update::default();
        up.offset = offset;
        up.data = io_uring_ptr::new(fds.as_ptr().cast_mut().cast());
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFilesUpdate,
                ptr::from_ref(&up).cast(),
                fds.len() as u32,
            )
        }
    }

    pub fn unregister_files(&self) -> Result<u32, Errno> {
        unsafe { self.do_register(IoringRegisterOp::UnregisterFiles, ptr::null(), 0) }
    }

    /// Registers a `BufRing` to the kernel, allowing the use of opcodes which can
    /// use provided buffers.
    ///
    /// # Safety
    /// If this call succeeds, `ring` must be unregistered via
    /// `unregister_buf_ring(bgid)` before it is dropped — the kernel retains
    /// `ring`'s address until then and will read from it to service ops that
    /// consume provided buffers.
    pub unsafe fn register_buf_ring<A: Allocator>(
        &self,
        ring: &BufRing<A>,
        bgid: u16,
        flags: u16,
    ) -> Result<u32, Errno> {
        let mut reg = io_uring_buf_reg::default();
        reg.ring_addr = io_uring_ptr::new(ring.base_addr().as_ptr().cast());
        reg.ring_entries = ring.capacity() as u32;
        reg.bgid = bgid;
        reg.flags = flags;
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterPbufRing,
                ptr::from_ref(&reg).cast(),
                1,
            )
        }
    }

    /// Unregisters the `BufRing` previously registered with the given `bgid`.
    ///
    /// # Safety
    /// No in-flight operation may hold a buffer obtained from this ring — once
    /// unregistered, any `bid` the kernel handed out under this `bgid` is no
    /// longer associated with the ring's memory, and a later registration on
    /// the same `bgid` could reuse those ids for unrelated buffers.
    pub unsafe fn unregister_buf_ring(&self, bgid: u16) -> Result<u32, Errno> {
        let mut reg = io_uring_buf_reg::default();
        reg.bgid = bgid;
        unsafe {
            self.do_register(
                IoringRegisterOp::UnregisterPbufRing,
                ptr::from_ref(&reg).cast(),
                1,
            )
        }
    }

    pub fn register_file_alloc_range(&self, offset: u32, len: u32) -> Result<u32, Errno> {
        let mut reg = io_uring_file_index_range::default();
        reg.offset = offset;
        reg.len = len;
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFileAllocRange,
                ptr::from_ref(&reg).cast(),
                0,
            )
        }
    }

    pub fn register_sync_cancel(&self, reg: &io_uring_sync_cancel_reg) -> Result<u32, Errno> {
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterSyncCancel,
                ptr::from_ref(reg).cast(),
                1,
            )
        }
    }
}

pub struct BufRing<A: Allocator = Global> {
    ptr: NonNull<u8>,
    mask: u16,
    tail: u16,
    alloc: A,
}

impl BufRing<Global> {
    pub fn new(max_entries: u16) -> Self {
        Self::new_in(max_entries, Global)
    }
}

impl<A: Allocator> BufRing<A> {
    fn layout(capacity: u16) -> Layout {
        let bytes = capacity as usize * size_of::<io_uring_buf>();
        Layout::from_size_align(bytes, page_size()).unwrap()
    }

    pub fn new_in(max_entries: u16, alloc: A) -> Self {
        let capacity = max_entries.next_power_of_two();
        let layout = Self::layout(capacity);
        let ptr = alloc.allocate(layout).expect("failed to allocate memory");

        Self {
            ptr: ptr.cast(),
            mask: capacity - 1,
            tail: 0,
            alloc,
        }
    }

    pub const fn capacity(&self) -> u16 {
        self.mask + 1
    }

    /// Adds a buffer to the back of the queue. In order to make it visible
    /// to the kernel, you must call `BufRing::advance`.
    ///
    /// # Safety
    /// `buf` must live until either the `BufRing` is unregistered or it is
    /// used by the kernel to service an opcode.
    pub const unsafe fn add(&mut self, bid: u16, buf: *mut u8, len: u32) {
        let slot = (self.tail & self.mask) as usize;

        unsafe {
            let entry = self.base_addr().add(slot).as_mut();
            entry.addr = io_uring_ptr::new(buf.cast());
            entry.len = len;
            entry.bid = bid;
            entry.resv = 0;
        }

        self.tail = self.tail.wrapping_add(1);
    }

    /// Advances the tail of the queue to make any previously added buffers
    /// visible to the kernel
    pub fn advance(&mut self) {
        self.tail().store(self.tail, Ordering::Release);
    }

    const fn base_addr(&self) -> NonNull<io_uring_buf> {
        self.ptr.cast()
    }

    const fn tail(&self) -> &AtomicU16 {
        unsafe {
            self.ptr
                .byte_offset(mem::offset_of!(buf_ring_tail_struct, tail) as isize)
                .cast()
                .as_ref()
        }
    }
}

impl<A: Allocator> Drop for BufRing<A> {
    fn drop(&mut self) {
        unsafe {
            self.alloc
                .deallocate(self.ptr, Self::layout(self.capacity()));
        }
    }
}

#[cfg(not(miri))]
mod system {
    use core::{cmp, mem};

    use rustix::{
        fd::AsFd,
        ffi::c_void,
        io::Errno,
        io_uring::{
            IORING_OFF_SQ_RING, IORING_OFF_SQES, IoringEnterFlags, IoringRegisterOp,
            io_uring_params,
        },
    };

    use super::{Backing, Cqe, Mmap, Sqe};

    pub use rustix::fd::OwnedFd;

    pub unsafe fn io_uring_setup(
        entries: u32,
        params: &mut io_uring_params,
    ) -> Result<OwnedFd, Errno> {
        unsafe { rustix::io_uring::io_uring_setup(entries, params) }
    }

    pub unsafe fn io_uring_enter<Fd: AsFd>(
        fd: Fd,
        to_submit: u32,
        min_complete: u32,
        flags: IoringEnterFlags,
    ) -> Result<u32, Errno> {
        unsafe { rustix::io_uring::io_uring_enter(fd, to_submit, min_complete, flags) }
    }

    pub unsafe fn io_uring_register<Fd: AsFd>(
        fd: Fd,
        opcode: IoringRegisterOp,
        arg: *const c_void,
        nr_args: u32,
    ) -> Result<u32, Errno> {
        unsafe { rustix::io_uring::io_uring_register(fd, opcode, arg, nr_args) }
    }

    pub unsafe fn io_uring_map_rings<Fd: AsFd>(
        fd: Fd,
        p: &io_uring_params,
    ) -> Result<Backing, Errno> {
        let sq_ring_size = p.sq_off.array as usize + p.sq_entries as usize * mem::size_of::<u32>();
        let cq_ring_size = p.cq_off.cqes as usize + p.cq_entries as usize * mem::size_of::<Cqe>();
        let rings_size = cmp::max(sq_ring_size, cq_ring_size);
        let rings_map = unsafe { Mmap::shared(&fd, IORING_OFF_SQ_RING, rings_size) }?;

        let sqes_size = p.sq_entries as usize * mem::size_of::<Sqe>();
        let sqes_map = unsafe { Mmap::shared(&fd, IORING_OFF_SQES, sqes_size) }?;

        Ok(Backing {
            rings: rings_map,
            sqes: sqes_map,
        })
    }
}

#[cfg(miri)]
mod system {
    use core::{
        mem, ptr,
        sync::atomic::{AtomicU32, Ordering},
    };

    use std::{collections::HashMap, sync::Mutex};

    use rustix::{
        fd::{AsFd, AsRawFd, BorrowedFd},
        ffi::c_void,
        io::Errno,
        io_uring::{
            IoringCqeFlags, IoringEnterFlags, IoringFeatureFlags, IoringOp, IoringRegisterOp,
            IoringSetupFlags, IoringSqFlags, io_uring_params,
        },
    };

    use crate::{sys::Mmap, utils::Slab};

    use super::{Backing, Cqe, Sqe};

    pub struct OwnedFd(i32);

    impl AsFd for OwnedFd {
        fn as_fd(&self) -> BorrowedFd<'_> {
            unsafe { BorrowedFd::borrow_raw(self.0) }
        }
    }

    impl Drop for OwnedFd {
        fn drop(&mut self) {
            FAKE_RINGS.lock().unwrap().remove(self.0 as u32);
        }
    }

    static FAKE_RINGS: Mutex<Slab<FakeRing>> = Mutex::new(Slab::new());

    struct FakeRing {
        sq_head: *const AtomicU32,
        sq_tail: *const AtomicU32,
        cq_head: *const AtomicU32,
        cq_tail: *const AtomicU32,

        sqes: *const Sqe,
        cqes: *const Cqe,

        pending: Vec<Sqe>,

        files: HashMap<i32, Vec<u8>>,
        next_fd: i32,
        seed: u32,
    }

    unsafe impl Send for FakeRing {}

    impl FakeRing {
        const ENTRIES: u32 = 16;

        fn new(_entries: u32, p: &mut io_uring_params) -> Self {
            if p.flags.contains(IoringSetupFlags::SQPOLL) {
                panic!("Simulated io_uring does not support SQPOLL");
            }

            p.sq_entries = Self::ENTRIES;
            p.cq_entries = Self::ENTRIES;

            p.sq_off.head = 0;
            p.sq_off.tail = 4;
            p.sq_off.ring_mask = 8;
            p.sq_off.ring_entries = 12;
            p.sq_off.flags = 16;
            p.sq_off.array = 0;

            p.cq_off.head = 20;
            p.cq_off.tail = 24;
            p.cq_off.ring_mask = 28;
            p.cq_off.ring_entries = 32;
            p.cq_off.cqes = 64;

            p.features = IoringFeatureFlags::SINGLE_MMAP;

            Self {
                sq_head: ptr::null(),
                sq_tail: ptr::null(),
                cq_head: ptr::null(),
                cq_tail: ptr::null(),
                sqes: ptr::null(),
                cqes: ptr::null(),
                pending: Vec::new(),
                files: HashMap::new(),
                next_fd: 100,
                seed: 17,
            }
        }

        fn map(&mut self, p: &io_uring_params) -> Backing {
            let rings = Mmap::private(4096).unwrap();
            let sqes = Mmap::private(4096).unwrap();

            unsafe {
                self.sq_head = rings.addr().offset(p.sq_off.head as isize).as_ptr().cast();
                self.sq_tail = rings.addr().offset(p.sq_off.tail as isize).as_ptr().cast();
                rings
                    .addr()
                    .offset(p.sq_off.ring_mask as isize)
                    .cast()
                    .write(Self::ENTRIES - 1);
                rings
                    .addr()
                    .offset(p.sq_off.ring_entries as isize)
                    .cast()
                    .write(Self::ENTRIES);
                rings
                    .addr()
                    .offset(p.sq_off.flags as isize)
                    .cast()
                    .write(IoringSqFlags::empty());

                self.cq_head = rings.addr().offset(p.cq_off.head as isize).as_ptr().cast();
                self.cq_tail = rings.addr().offset(p.cq_off.tail as isize).as_ptr().cast();
                rings
                    .addr()
                    .offset(p.cq_off.ring_mask as isize)
                    .cast()
                    .write(Self::ENTRIES - 1);
                rings
                    .addr()
                    .offset(p.cq_off.ring_entries as isize)
                    .cast()
                    .write(Self::ENTRIES);

                self.cqes = rings.addr().offset(p.cq_off.cqes as isize).as_ptr().cast();
                for i in 0..Self::ENTRIES as isize {
                    self.cqes.offset(i).cast_mut().write(mem::zeroed());
                }

                self.sqes = sqes.addr().as_ptr().cast();
                for i in 0..Self::ENTRIES as isize {
                    self.sqes.offset(i).cast_mut().write(mem::zeroed());
                }
            }

            Backing { rings, sqes }
        }

        fn enter(&mut self, to_submit: u32, min_complete: u32, _flags: IoringEnterFlags) -> u32 {
            let mask = Self::ENTRIES - 1;
            let head = unsafe { (*self.sq_head).load(Ordering::Relaxed) };
            let tail = unsafe { (*self.sq_tail).load(Ordering::Relaxed) };
            let available = tail.wrapping_sub(head);
            let count = to_submit.min(available);

            for i in 0..count {
                let idx = (head.wrapping_add(i) & mask) as usize;
                let sqe = unsafe { ptr::read(self.sqes.add(idx)) };
                self.pending.push(sqe);
            }

            unsafe { (*self.sq_head).store(head.wrapping_add(count), Ordering::Relaxed) };

            let cq_head = unsafe { (*self.cq_head).load(Ordering::Acquire) };
            let cq_tail = unsafe { (*self.cq_tail).load(Ordering::Relaxed) };
            let ready = cq_tail.wrapping_sub(cq_head);
            let to_post = min_complete.saturating_sub(ready);
            for _ in 0..to_post {
                self.complete_rand();
            }

            count
        }

        fn post_cqe(&mut self, user_data: u64, res: i32) {
            let mask = Self::ENTRIES - 1;
            let head = unsafe { (*self.cq_head).load(Ordering::Acquire) };
            let tail = unsafe { (*self.cq_tail).load(Ordering::Relaxed) };

            assert!(tail.wrapping_sub(head) < Self::ENTRIES, "CQ overflow");

            let idx = (tail & mask) as usize;
            unsafe {
                let slot = &mut (*self.cqes.add(idx).cast_mut()).0;
                slot.user_data.u64_ = user_data;
                slot.res = res;
                slot.flags = IoringCqeFlags::empty();
            }

            unsafe { (*self.cq_tail).store(tail.wrapping_add(1), Ordering::Release) };
        }

        fn complete_rand(&mut self) {
            if self.pending.is_empty() {
                return;
            }

            let idx = self.rand(self.pending.len() as u32) as usize;
            let sqe = self.pending.swap_remove(idx);

            let user_data = unsafe { sqe.0.user_data.u64_ };
            let res = match sqe.0.opcode {
                IoringOp::Openat => self.simulate_openat(),
                IoringOp::Read => unsafe { self.simulate_read(&sqe) },
                IoringOp::Write => unsafe { self.simulate_write(&sqe) },
                _ => -1,
            };

            self.post_cqe(user_data, res);
        }

        fn simulate_openat(&mut self) -> i32 {
            let fd = self.next_fd;
            self.next_fd += 1;
            self.files.insert(fd, Vec::new());
            fd
        }

        /// # Safety
        /// `sqe` must be a valid Read SQE: `addr`/`len` describe a writable
        /// buffer the caller still owns.
        unsafe fn simulate_read(&mut self, sqe: &Sqe) -> i32 {
            let fd = sqe.0.fd;
            let addr = unsafe { sqe.0.addr_or_splice_off_in.addr.ptr } as *mut u8;
            let len = unsafe { sqe.0.len.len } as usize;
            let offset = unsafe { sqe.0.off_or_addr2.off } as usize;

            let Some(file) = self.files.get(&fd) else {
                return -Errno::BADF.raw_os_error();
            };

            if offset >= file.len() {
                return 0;
            }
            let n = (file.len() - offset).min(len);

            unsafe { ptr::copy_nonoverlapping(file.as_ptr().add(offset), addr, n) };

            n as i32
        }

        /// # Safety
        /// `sqe` must be a valid Write SQE: `addr`/`len` describe a readable
        /// buffer the caller still owns.
        unsafe fn simulate_write(&mut self, sqe: &Sqe) -> i32 {
            let fd = sqe.0.fd;
            let addr = unsafe { sqe.0.addr_or_splice_off_in.addr.ptr } as *const u8;
            let len = unsafe { sqe.0.len.len } as usize;
            let offset = unsafe { sqe.0.off_or_addr2.off } as usize;

            let Some(file) = self.files.get_mut(&fd) else {
                return -Errno::BADF.raw_os_error();
            };

            let end = offset + len;
            if file.len() < end {
                file.resize(end, 0);
            }

            unsafe { ptr::copy_nonoverlapping(addr, file.as_mut_ptr().add(offset), len) };

            len as i32
        }

        fn rand(&mut self, max: u32) -> u32 {
            self.seed = self.seed.wrapping_mul(31).wrapping_add(101);
            self.seed % max
        }
    }

    pub unsafe fn io_uring_setup(
        entries: u32,
        params: &mut io_uring_params,
    ) -> Result<OwnedFd, Errno> {
        let ring = FakeRing::new(entries, params);
        let idx = FAKE_RINGS.lock().unwrap().insert(ring);
        Ok(OwnedFd(idx as i32))
    }

    pub unsafe fn io_uring_map_rings<Fd: AsFd>(
        fd: Fd,
        p: &io_uring_params,
    ) -> Result<Backing, Errno> {
        Ok(FAKE_RINGS
            .lock()
            .unwrap()
            .get_mut(fd.as_fd().as_raw_fd() as u32)
            .map(p))
    }

    pub unsafe fn io_uring_enter<Fd: AsFd>(
        fd: Fd,
        to_submit: u32,
        min_complete: u32,
        flags: IoringEnterFlags,
    ) -> Result<u32, Errno> {
        Ok(FAKE_RINGS
            .lock()
            .unwrap()
            .get_mut(fd.as_fd().as_raw_fd() as u32)
            .enter(to_submit, min_complete, flags))
    }

    pub unsafe fn io_uring_register<Fd: AsFd>(
        _fd: Fd,
        _opcode: IoringRegisterOp,
        _arg: *const c_void,
        _nr_args: u32,
    ) -> Result<u32, Errno> {
        panic!("Simulated io_uring cannot register");
    }
}
