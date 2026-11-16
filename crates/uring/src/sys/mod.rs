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
    io::Errno,
    io_uring::*,
    param::page_size,
};

// Miri can't run io_uring, so under miri (or with `--cfg inel_fake`) the
// syscalls are served by an in-process emulation of the kernel side.
#[cfg(any(miri, inel_fake))]
#[path = "fake.rs"]
mod system;
#[cfg(not(any(miri, inel_fake)))]
#[path = "real.rs"]
mod system;

#[cfg(test)]
mod tests;

const REQUIRED_FEATURES: IoringFeatureFlags = IoringFeatureFlags::SINGLE_MMAP;

/// `IORING_FILE_INDEX_ALLOC`: as an op's file index, asks the kernel to pick a
/// free direct descriptor slot (within the alloc range) and return it as the
/// result.
pub const FILE_INDEX_ALLOC: u32 = u32::MAX;

/// `IORING_TIMEOUT_MULTISHOT`, not yet exposed by rustix.
pub const TIMEOUT_MULTISHOT: IoringTimeoutFlags = IoringTimeoutFlags::from_bits_retain(1 << 6);

#[repr(transparent)]
pub struct Sqe(io_uring_sqe);

impl Sqe {
    /// An all-zeroes SQE, for tests that need one outside a ring.
    #[cfg(test)]
    pub(crate) const fn zeroed() -> Self {
        // SAFETY: all-zeroes is a valid `io_uring_sqe`
        Self(unsafe { mem::zeroed() })
    }

    pub(crate) const fn init(&mut self) {
        // SAFETY: all-zeroes is a valid `io_uring_sqe`
        self.0 = unsafe { mem::zeroed() };
    }

    pub const fn add_flag(&mut self, flag: IoringSqeFlags) {
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

    /// For ops that create a descriptor (open, accept, socket), installs it as
    /// a direct descriptor: in slot `index - 1`, or in a slot the kernel picks
    /// with [`FILE_INDEX_ALLOC`]. For close, the slot to close, plus one.
    pub const fn set_file_index(&mut self, index: u32) {
        self.0.splice_fd_in_or_file_index_or_addr_len.file_index = index;
    }

    /// Lets the kernel pick the buffer from the provided buffer group `bgid`
    /// when the op runs, instead of using `addr`.
    pub const fn buffer_select(&mut self, bgid: u16) {
        self.add_flag(IoringSqeFlags::BUFFER_SELECT);
        self.0.buf.buf_group = bgid;
    }

    /// The buffer group the op selects from, if any.
    pub const fn buf_group(&self) -> Option<u16> {
        if self.0.flags.contains(IoringSqeFlags::BUFFER_SELECT) {
            // SAFETY: with BUFFER_SELECT, the union holds the group
            Some(unsafe { self.0.buf.buf_group })
        } else {
            None
        }
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

    /// Closes the regular fd `fd`.
    pub const fn prep_close(&mut self, fd: RawFd) {
        self.prep_rw(IoringOp::Close, fd, ptr::null(), 0, 0);
    }

    /// Closes the direct descriptor in `slot`.
    pub const fn prep_close_direct(&mut self, slot: u32) {
        self.prep_rw(IoringOp::Close, 0, ptr::null(), 0, 0);
        self.set_file_index(slot + 1);
    }

    pub const fn prep_read(&mut self, fd: RawFd, buf: *mut u8, nbytes: u32, offset: u64) {
        self.prep_rw(IoringOp::Read, fd, buf.cast(), nbytes, offset);
    }

    pub const fn prep_write(&mut self, fd: RawFd, buf: *const u8, nbytes: u32, offset: u64) {
        self.prep_rw(IoringOp::Write, fd, buf.cast(), nbytes, offset);
    }
}

struct SubmissionQueue {
    head: u32,
    tail: u32,

    khead: NonNull<AtomicU32>,
    ktail: NonNull<AtomicU32>,

    mask: u32,
    entries: u32,

    flags: NonNull<AtomicU32>,

    sqes: NonNull<Sqe>,
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

#[repr(transparent)]
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
    khead: NonNull<AtomicU32>,
    ktail: NonNull<AtomicU32>,

    mask: u32,
    entries: u32,

    cqes: NonNull<Cqe>,
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
    fd: system::RingFd,

    sq: SubmissionQueue,
    cq: CompletionQueue,

    setup: IoringSetupFlags,

    #[allow(unused)]
    memory: system::Backing,
}

impl AsFd for IoUring {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl IoUring {
    /// Creates a ring with the default [`IoUringOptions`].
    pub fn new() -> Result<Self, Errno> {
        Self::options().build()
    }

    /// Starts configuring a ring; finish with [`IoUringOptions::build`].
    #[must_use]
    pub fn options<'a>() -> IoUringOptions<'a> {
        IoUringOptions::default()
    }

    /// Number of entries in the submission queue.
    pub const fn sq_size(&self) -> u32 {
        self.sq.entries
    }

    /// Number of entries in the completion queue.
    pub const fn cq_size(&self) -> u32 {
        self.cq.entries
    }

    /// Hands the pushed SQEs to the kernel without waiting for completions.
    /// See [`submit_and_wait`](Self::submit_and_wait).
    pub fn submit(&mut self) -> Result<u32, Errno> {
        self.submit_and_wait(0)
    }

    /// Hands the pushed SQEs to the kernel and blocks until at least `want`
    /// CQEs are available. Only enters the kernel when needed (pending SQEs
    /// without SQPOLL, a sleeping SQPOLL thread, deferred task work, CQ
    /// overflow, or `want > 0`). Returns the number of SQEs the kernel
    /// consumed during this call, which is 0 if it wasn't entered.
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

    /// Builds the next SQE in place with `build`, returning what it returns.
    /// It is picked up by the kernel on the next [`submit`](Self::submit).
    /// Fails with `NOBUFS` if the submission queue is full, without calling
    /// `build`.
    ///
    /// # Safety
    /// Everything the SQE references (buffers, iovecs, paths, timespecs, ...)
    /// must stay valid, and must not be moved or accessed in a way that
    /// conflicts with the kernel, until the op's final CQE is reaped. Data the
    /// kernel only reads while preparing the op (e.g. a timeout's timespec)
    /// only needs to outlive the kernel consuming the SQE: after `submit`
    /// returns, or at an unknown point with SQPOLL.
    pub unsafe fn push_sqe<F, T>(&mut self, build: F) -> Result<T, Errno>
    where
        F: FnOnce(&mut Sqe) -> T,
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

    /// Calls `reap` on every CQE currently in the completion queue, then
    /// releases them back to the kernel. Doesn't wait for new ones.
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

    /// Registers `iovecs` as fixed buffers, for the `*_FIXED` opcodes.
    ///
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

    /// Registers a table of `nr` empty fixed buffer slots, to be filled with
    /// [`register_buffers_update`](Self::register_buffers_update).
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

    /// Replaces the fixed buffers starting at slot `offset` with `iovecs`.
    ///
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

    /// Unregisters all fixed buffers.
    pub fn unregister_buffers(&self) -> Result<u32, Errno> {
        unsafe { self.do_register(IoringRegisterOp::UnregisterBuffers, ptr::null(), 0) }
    }

    /// Registers `fds` as fixed files, usable with `IOSQE_FIXED_FILE`.
    ///
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

    /// Registers a table of `nr` empty fixed file slots, to be filled with
    /// [`register_files_update`](Self::register_files_update) or by direct
    /// descriptor ops.
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

    /// Replaces the fixed files starting at slot `offset` with `fds` (`-1`
    /// clears a slot). Returns the number of files actually updated.
    ///
    /// # Safety
    /// `fds` must reference open file descriptors for the duration of the call.
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

    /// Unregisters all fixed files.
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
        ring: &IoBufRing<A>,
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

    /// Restricts the fixed file slots the kernel picks from when an op asks
    /// it to allocate one (`IORING_FILE_INDEX_ALLOC`) to `offset..offset + len`.
    pub fn register_file_alloc_range(&self, offset: u32, len: u32) -> Result<u32, Errno> {
        // `struct io_uring_file_index_range`, not exposed by rustix
        #[repr(C)]
        struct FileIndexRange {
            offset: u32,
            len: u32,
            resv: u64,
        }

        let reg = FileIndexRange {
            offset,
            len,
            resv: 0,
        };
        unsafe {
            self.do_register(
                IoringRegisterOp::RegisterFileAllocRange,
                ptr::from_ref(&reg).cast(),
                0,
            )
        }
    }

    /// Cancels the requests matching `reg` and blocks until they have
    /// completed (or `reg`'s timeout expires).
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

pub struct IoBufRing<A: Allocator = Global> {
    ptr: NonNull<u8>,
    mask: u16,
    tail: u16,
    alloc: A,
}

impl IoBufRing<Global> {
    pub fn new(max_entries: u16) -> Self {
        Self::new_in(max_entries, Global)
    }
}

impl<A: Allocator> IoBufRing<A> {
    fn layout(capacity: u16) -> Layout {
        let bytes = capacity as usize * size_of::<io_uring_buf>();
        Layout::from_size_align(bytes, page_size()).unwrap()
    }

    pub fn new_in(max_entries: u16, alloc: A) -> Self {
        let capacity = max_entries.next_power_of_two();
        let layout = Self::layout(capacity);

        // zeroed, so the kernel sees an empty ring (tail 0) until `advance`
        let ptr = alloc
            .allocate_zeroed(layout)
            .expect("failed to allocate memory");

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

        // Write the fields through raw pointers: `resv` of the first entry is
        // the tail, which the kernel reads concurrently, so neither a
        // reference to the entry nor a write to `resv` is allowed.
        unsafe {
            let entry = self.base_addr().add(slot).as_ptr();
            (&raw mut (*entry).addr).write(io_uring_ptr::new(buf.cast()));
            (&raw mut (*entry).len).write(len);
            (&raw mut (*entry).bid).write(bid);
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

impl<A: Allocator> Drop for IoBufRing<A> {
    fn drop(&mut self) {
        unsafe {
            self.alloc
                .deallocate(self.ptr, Self::layout(self.capacity()));
        }
    }
}
