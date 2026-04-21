use core::{
    cmp,
    ffi::c_void,
    mem, ptr,
    result::Result,
    sync::atomic::{self, AtomicU32, Ordering},
};

use rustix::{
    fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd},
    io::Errno,
    io_uring::*,
};

use crate::utils::Mmap;

pub struct IoUring {
    fd: OwnedFd,

    sq: SubmissionQueue,
    cq: CompletionQueue,

    setup: IoringSetupFlags,

    #[allow(unused)]
    memory: Backing,
}

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

    pub(crate) khead: *const AtomicU32,
    pub(crate) ktail: *const AtomicU32,

    pub(crate) mask: u32,
    pub(crate) entries: u32,

    pub(crate) flags: *const AtomicU32,

    pub(crate) sqes: *mut Sqe,
}

impl SubmissionQueue {
    pub fn new(ring: ptr::NonNull<u8>, sqes: ptr::NonNull<u8>, p: &io_uring_params) -> Self {
        unsafe {
            let ptr = ring.as_ptr();

            let khead = ptr.offset(p.sq_off.head as isize).cast::<AtomicU32>();
            assert!(khead.is_aligned(), "misaligned sq khead pointer");

            let ktail = ptr.offset(p.sq_off.tail as isize).cast::<AtomicU32>();
            assert!(ktail.is_aligned(), "misaligned sq ktail pointer");

            let mask_ptr = ptr.offset(p.sq_off.ring_mask as isize).cast::<u32>();
            assert!(mask_ptr.is_aligned(), "misaligned sq mask pointer");

            let entries_ptr = ptr.offset(p.sq_off.ring_entries as isize).cast::<u32>();
            assert!(entries_ptr.is_aligned(), "misaligned sq entries pointer");

            let flags = ptr.offset(p.sq_off.flags as isize).cast::<AtomicU32>();
            assert!(flags.is_aligned(), "misaligned sq flags pointer");

            let sqes = sqes.as_ptr().cast::<Sqe>();
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
    pub(crate) khead: *const AtomicU32,
    pub(crate) ktail: *const AtomicU32,

    pub(crate) mask: u32,
    pub(crate) entries: u32,

    pub(crate) cqes: *const Cqe,
}

impl CompletionQueue {
    pub fn new(ring: ptr::NonNull<u8>, p: &io_uring_params) -> Self {
        unsafe {
            let ptr = ring.as_ptr();

            let khead = ptr.offset(p.cq_off.head as isize).cast::<AtomicU32>();
            assert!(khead.is_aligned(), "misaligned cq khead pointer");

            let ktail = ptr.offset(p.cq_off.tail as isize).cast::<AtomicU32>();
            assert!(ktail.is_aligned(), "misaligned cq ktail pointer");

            let mask_ptr = ptr.offset(p.cq_off.ring_mask as isize).cast::<u32>();
            assert!(mask_ptr.is_aligned(), "misaligned cq mask pointer");

            let entries_ptr = ptr.offset(p.cq_off.ring_entries as isize).cast::<u32>();
            assert!(entries_ptr.is_aligned(), "misaligned cq entries pointer");

            let cqes = ptr.offset(p.cq_off.cqes as isize).cast::<Cqe>();
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
            flags: IoringSetupFlags::NO_SQARRAY,
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

        let fd = unsafe { io_uring_setup(self.sq_entries, &mut p) }?;

        if !p.features.contains(REQUIRED_FEATURES) {
            return Err(Errno::INVAL);
        }

        let sq_ring_size = p.sq_off.array as usize + p.sq_entries as usize * mem::size_of::<u32>();
        let cq_ring_size = p.cq_off.cqes as usize + p.cq_entries as usize * mem::size_of::<Cqe>();
        let rings_size = cmp::max(sq_ring_size, cq_ring_size);
        let rings_map = unsafe { Mmap::shared(&fd, IORING_OFF_SQ_RING, rings_size) }?;

        let sqes_size = p.sq_entries as usize * mem::size_of::<Sqe>();
        let sqes_map = unsafe { Mmap::shared(&fd, IORING_OFF_SQES, sqes_size) }?;

        let sq = SubmissionQueue::new(rings_map.addr(), sqes_map.addr(), &p);
        let cq = CompletionQueue::new(rings_map.addr(), &p);

        Ok(IoUring {
            fd,
            sq,
            cq,
            setup: p.flags,
            memory: Backing {
                rings: rings_map,
                sqes: sqes_map,
            },
        })
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

    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    pub unsafe fn push_sqe<F>(&mut self, build: F) -> Result<(), Errno>
    where
        F: FnOnce(&mut Sqe),
    {
        self.sq_get_next().map(build).ok_or(Errno::NOBUFS)
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

            unsafe { io_uring_enter(&self.fd, pending, want, flags) }?
        } else {
            0
        };

        Ok(submitted)
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
            let cqe = unsafe { &*cq.cqes.add((khead & cq.mask) as usize) };
            reap(cqe);
            khead = khead.wrapping_add(1);
            count += 1;
        }

        self.cq_advance(count);
    }

    fn sq_get_next(&mut self) -> Option<&mut Sqe> {
        let khead = self.sq_load_khead();
        let sq = &mut self.sq;

        if sq.tail.wrapping_sub(khead) >= sq.entries {
            return None;
        }

        let sqe = unsafe { &mut *sq.sqes.add((sq.tail & sq.mask) as usize) };

        sq.tail = sq.tail.wrapping_add(1);

        sqe.init();

        Some(sqe)
    }

    fn sq_flush(&mut self) -> u32 {
        let sq = &mut self.sq;

        if sq.head != sq.tail {
            sq.head = sq.tail;
            self.sq_store_ktail();
        }

        self.sq.tail.wrapping_sub(self.sq_load_khead())
    }

    fn sq_load_khead(&self) -> u32 {
        if self.is_sqpoll() {
            unsafe { (*self.sq.khead).load(Ordering::Acquire) }
        } else {
            unsafe { (*self.sq.khead).load(Ordering::Relaxed) }
        }
    }

    fn sq_store_ktail(&self) {
        if self.is_sqpoll() {
            unsafe { (*self.sq.ktail).store(self.sq.tail, Ordering::Release) };
        } else {
            unsafe { (*self.sq.ktail).store(self.sq.tail, Ordering::Relaxed) };
        }
    }

    fn sq_load_kflags(&self) -> IoringSqFlags {
        IoringSqFlags::from_bits_retain(unsafe { (*self.sq.flags).load(Ordering::Relaxed) })
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
        unsafe { (*self.cq.khead).load(Ordering::Relaxed) }
    }

    fn cq_load_ktail(&self) -> u32 {
        unsafe { (*self.cq.ktail).load(Ordering::Acquire) }
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
            (*self.cq.khead).store(
                (*self.cq.khead).load(Ordering::Relaxed) + count,
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
        unsafe { io_uring_register(&self.fd, op, arg, nr_args) }
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

    /// # Safety
    /// `ring_addr` must point to a buffer ring of `ring_entries` entries that
    /// remains valid until the buffer ring is unregistered.
    pub unsafe fn register_buf_ring(
        &self,
        ring_addr: *mut c_void,
        ring_entries: u32,
        bgid: u16,
        flags: u16,
    ) -> Result<u32, Errno> {
        let mut reg = io_uring_buf_reg::default();
        reg.ring_addr = io_uring_ptr::new(ring_addr);
        reg.ring_entries = ring_entries;
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

    pub fn unregister_buf_ring(&self, bgid: u16) -> Result<u32, Errno> {
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
        #[repr(C)]
        struct FileIndexRange(u32, u32, u64);
        let range = FileIndexRange(offset, len, 0);
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFileAllocRange,
                ptr::from_ref(&range).cast(),
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
