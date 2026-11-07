//! In-process emulation of the kernel side of io_uring.
//!
//! Miri can't run io_uring (or `mmap` a ring), so under miri (or with
//! `--cfg inel_fake`) the syscalls used by [`super::IoUring`] are served by
//! this module instead. Everything above the syscalls runs unchanged: the
//! ring memory is laid out like the kernel's and shared with the user side the
//! same way, with the same atomic protocol on the head/tail indices.
//!
//! Requests are executed asynchronously by a dedicated kernel thread, so miri
//! sees the "kernel" accessing user memory concurrently with the program and
//! reports use-after-free, aliasing violations and data races on buffers that
//! are still owned by in-flight operations.
//!
//! The emulation is deliberately adversarial where the kernel is allowed to be:
//! - requests are picked from the queue in random order, and every request
//!   (including NOPs) completes asynchronously;
//! - user memory is accessed as late as possible (buffers when the op runs,
//!   not at submission);
//! - consumed SQE slots are filled with garbage, so `prep_*` functions that
//!   rely on stale fields fail with `-EINVAL`;
//! - cancellation may return `-EALREADY` and let the request run to
//!   completion anyway.
//!
//! Randomness comes from `INEL_FAKE_SEED`. Miri hides the host environment,
//! so forward it:
//!
//! ```sh
//! INEL_FAKE_SEED=2 MIRIFLAGS="-Zmiri-env-forward=INEL_FAKE_SEED -Zmiri-seed=2" \
//!     cargo +nightly miri test -p uring
//! ```
//!
//! I/O on stream fds (offset `-1`) is gated on readiness through epoll, like
//! the kernel's internal poll arming, so blocked requests don't hold up the
//! kernel thread and can be cancelled. Positioned I/O (any other offset) runs
//! directly. Under miri, regular files must therefore use explicit offsets
//! (miri can't poll them), and non-seekable fds must use offset `-1` (miri
//! can't `pread` them).

// The kernel state lock is deliberately held across whole sections.
#![allow(clippy::significant_drop_tightening)]

use core::{
    alloc::Layout,
    ffi::{CStr, c_void},
    mem::{self, MaybeUninit},
    ptr::{self, NonNull},
    slice,
    sync::atomic::{AtomicU16, AtomicU32, AtomicU64, Ordering},
    time::Duration,
};
use std::{
    alloc,
    collections::{BTreeMap, BTreeSet, VecDeque},
    env,
    ffi::CString,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::{self, JoinHandle},
    time::Instant,
};

use rustix::{
    buffer::spare_capacity,
    event::{self, EventfdFlags, Timespec as EventTimespec, epoll},
    fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd, RawFd},
    fs::{self, Mode, OFlags},
    io::{self, Errno},
    io_uring::*,
};

use super::TIMEOUT_MULTISHOT;

// Ring layout. Both rings share one allocation (IORING_FEAT_SINGLE_MMAP).
const SQ_HEAD: u32 = 0;
const SQ_TAIL: u32 = 4;
const SQ_MASK: u32 = 8;
const SQ_ENTRIES: u32 = 12;
const SQ_FLAGS: u32 = 16;
const SQ_DROPPED: u32 = 20;
const CQ_HEAD: u32 = 64;
const CQ_TAIL: u32 = 68;
const CQ_MASK: u32 = 72;
const CQ_ENTRIES: u32 = 76;
const CQ_OVERFLOW: u32 = 80;
const CQ_FLAGS: u32 = 84;
const CQ_CQES: u32 = 128;

const MAX_ENTRIES: u32 = 32768;
const POISON: u8 = 0xA5;
const WAKE_TOKEN: u64 = u64::MAX;

/// `IORING_NOP_INJECT_RESULT`
const NOP_INJECT_RESULT: u32 = 1 << 0;

const SUPPORTED_SETUP: IoringSetupFlags = IoringSetupFlags::NO_SQARRAY
    .union(IoringSetupFlags::CQSIZE)
    .union(IoringSetupFlags::SINGLE_ISSUER)
    .union(IoringSetupFlags::DEFER_TASKRUN)
    .union(IoringSetupFlags::COOP_TASKRUN)
    .union(IoringSetupFlags::TASKRUN_FLAG)
    .union(IoringSetupFlags::SUBMIT_ALL)
    .union(IoringSetupFlags::CLAMP)
    // sharing the async backend only matters for performance
    .union(IoringSetupFlags::ATTACH_WQ);

const SUPPORTED_SQE_FLAGS: IoringSqeFlags = IoringSqeFlags::ASYNC
    .union(IoringSqeFlags::CQE_SKIP_SUCCESS)
    .union(IoringSqeFlags::BUFFER_SELECT);

const SUPPORTED_TIMEOUT: IoringTimeoutFlags =
    TIMEOUT_MULTISHOT.union(IoringTimeoutFlags::ETIME_SUCCESS);

/// Handle to a fake ring. Dropping it shuts the kernel down: pending requests
/// are discarded and a running one is waited for.
pub struct RingFd {
    kernel: Arc<Kernel>,
    thread: Option<JoinHandle<()>>,
}

impl AsFd for RingFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // a stand-in fd, so the ring has an identity like a real one
        self.kernel.epoll.as_fd()
    }
}

impl Drop for RingFd {
    fn drop(&mut self) {
        self.kernel.shutdown();
        if let Some(thread) = self.thread.take()
            && let Err(panic) = thread.join()
            && !thread::panicking()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

pub struct Mapping {
    addr: NonNull<u8>,
    _kernel: Arc<Kernel>,
}

impl Mapping {
    #[inline]
    pub const fn addr(&self) -> NonNull<u8> {
        self.addr
    }
}

pub struct Backing {
    pub rings: Mapping,
    pub sqes: Mapping,
}

/// # Safety
///
/// Same contract as the real `io_uring_setup`.
pub unsafe fn io_uring_setup(entries: u32, p: &mut io_uring_params) -> Result<RingFd, Errno> {
    if !SUPPORTED_SETUP.contains(p.flags) {
        return Err(Errno::INVAL);
    }

    // `IoUring` never fills the SQ index array
    if !p.flags.contains(IoringSetupFlags::NO_SQARRAY) {
        return Err(Errno::INVAL);
    }

    if p.flags.contains(IoringSetupFlags::DEFER_TASKRUN)
        && !p.flags.contains(IoringSetupFlags::SINGLE_ISSUER)
    {
        return Err(Errno::INVAL);
    }

    let clamp = |entries: u32, max: u32| match entries {
        0 => Err(Errno::INVAL),
        n if n <= max => Ok(n),
        _ if p.flags.contains(IoringSetupFlags::CLAMP) => Ok(max),
        _ => Err(Errno::INVAL),
    };

    let sq_entries = clamp(entries, MAX_ENTRIES)?.next_power_of_two();
    let cq_entries = if p.flags.contains(IoringSetupFlags::CQSIZE) {
        let cq_entries = clamp(p.cq_entries, 2 * MAX_ENTRIES)?.next_power_of_two();
        if cq_entries < sq_entries {
            return Err(Errno::INVAL);
        }
        cq_entries
    } else {
        2 * sq_entries
    };

    p.sq_entries = sq_entries;
    p.cq_entries = cq_entries;
    p.features = IoringFeatureFlags::SINGLE_MMAP
        | IoringFeatureFlags::NODROP
        | IoringFeatureFlags::SUBMIT_STABLE
        | IoringFeatureFlags::RW_CUR_POS
        | IoringFeatureFlags::CQE_SKIP;

    p.sq_off.head = SQ_HEAD;
    p.sq_off.tail = SQ_TAIL;
    p.sq_off.ring_mask = SQ_MASK;
    p.sq_off.ring_entries = SQ_ENTRIES;
    p.sq_off.flags = SQ_FLAGS;
    p.sq_off.dropped = SQ_DROPPED;
    p.sq_off.array = 0;

    p.cq_off.head = CQ_HEAD;
    p.cq_off.tail = CQ_TAIL;
    p.cq_off.ring_mask = CQ_MASK;
    p.cq_off.ring_entries = CQ_ENTRIES;
    p.cq_off.overflow = CQ_OVERFLOW;
    p.cq_off.flags = CQ_FLAGS;
    p.cq_off.cqes = CQ_CQES;

    let kernel = Arc::new(Kernel::new(sq_entries, cq_entries, p.flags)?);

    let thread = {
        let kernel = kernel.clone();
        thread::Builder::new()
            .name("fake-uring-kernel".into())
            .spawn(move || kernel.run())
            .expect("fake io_uring: failed to spawn the kernel thread")
    };

    Ok(RingFd {
        kernel,
        thread: Some(thread),
    })
}

/// # Safety
///
/// Same contract as the real `mmap` of the rings.
pub unsafe fn io_uring_map_rings(fd: &RingFd, p: &io_uring_params) -> Result<Backing, Errno> {
    let sq_ring_size = p.sq_off.array as usize + p.sq_entries as usize * mem::size_of::<u32>();
    let cq_ring_size =
        p.cq_off.cqes as usize + p.cq_entries as usize * mem::size_of::<io_uring_cqe>();
    let sqes_size = p.sq_entries as usize * mem::size_of::<io_uring_sqe>();

    Ok(Backing {
        rings: map(fd, IORING_OFF_SQ_RING, sq_ring_size.max(cq_ring_size))?,
        sqes: map(fd, IORING_OFF_SQES, sqes_size)?,
    })
}

fn map(fd: &RingFd, offset: u64, len: usize) -> Result<Mapping, Errno> {
    let kernel = &fd.kernel;
    let region = match offset {
        IORING_OFF_SQ_RING | IORING_OFF_CQ_RING => &kernel.rings,
        IORING_OFF_SQES => &kernel.sqes,
        _ => return Err(Errno::INVAL),
    };

    if len == 0 || len > region.layout.size() {
        return Err(Errno::INVAL);
    }

    Ok(Mapping {
        addr: region.ptr,
        _kernel: kernel.clone(),
    })
}

/// # Safety
///
/// Same contract as the real `io_uring_enter`.
pub unsafe fn io_uring_enter(
    fd: &RingFd,
    to_submit: u32,
    min_complete: u32,
    flags: IoringEnterFlags,
) -> Result<u32, Errno> {
    // without SQPOLL, SQ_WAKEUP is ignored by the kernel
    if !(IoringEnterFlags::GETEVENTS | IoringEnterFlags::SQ_WAKEUP).contains(flags) {
        return Err(Errno::INVAL);
    }

    let kernel = &fd.kernel;
    let mut state = kernel.lock();

    let submitted = kernel.submit(&mut state, to_submit);

    if flags.contains(IoringEnterFlags::GETEVENTS) {
        loop {
            kernel.flush(&mut state);

            if kernel.cq_ready() >= min_complete {
                break;
            }

            // The real kernel would block forever here; panicking is friendlier.
            // (If requests are in flight but can never complete, miri reports a
            // deadlock instead.)
            assert!(
                !state.requests.is_empty(),
                "fake io_uring: waiting for {min_complete} completions with no requests in \
                 flight, io_uring_enter would block forever"
            );

            state = kernel
                .completions
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    Ok(submitted)
}

/// # Safety
///
/// Same contract as the real `io_uring_register`.
pub unsafe fn io_uring_register(
    fd: &RingFd,
    opcode: IoringRegisterOp,
    arg: *const c_void,
    nr_args: u32,
) -> Result<u32, Errno> {
    match opcode {
        // nothing can be registered yet, like the kernel when the table is empty
        IoringRegisterOp::UnregisterBuffers | IoringRegisterOp::UnregisterFiles => Err(Errno::NXIO),
        IoringRegisterOp::RegisterPbufRing | IoringRegisterOp::UnregisterPbufRing => {
            if nr_args != 1 {
                return Err(Errno::INVAL);
            }
            if arg.is_null() {
                return Err(Errno::FAULT);
            }
            // SAFETY: the caller passes a valid `io_uring_buf_reg`
            let reg = unsafe { arg.cast::<io_uring_buf_reg>().read() };
            if opcode == IoringRegisterOp::RegisterPbufRing {
                fd.kernel.register_buf_ring(&reg).map(|()| 0)
            } else {
                fd.kernel.unregister_buf_ring(&reg).map(|()| 0)
            }
        }
        // TODO: fixed buffers/files
        _ => Err(Errno::INVAL),
    }
}

fn seed() -> u64 {
    static RINGS: AtomicU64 = AtomicU64::new(0);
    let base = env::var("INEL_FAKE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED);
    let ring = RINGS.fetch_add(1, Ordering::Relaxed);
    base ^ ring.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// Zero-initialized (or poisoned) memory shared with the user side.
struct Region {
    ptr: NonNull<u8>,
    layout: Layout,
}

// SAFETY: shared with the user side, which accesses it only through atomics or
// under the ring protocol
unsafe impl Send for Region {}
unsafe impl Sync for Region {}

impl Region {
    fn new(len: usize, fill: u8) -> Self {
        let layout = Layout::from_size_align(len, 64).expect("fake io_uring: bad region size");
        // SAFETY: `len` is non-zero
        let ptr = unsafe { alloc::alloc(layout) };
        let Some(ptr) = NonNull::new(ptr) else {
            alloc::handle_alloc_error(layout);
        };
        // SAFETY: freshly allocated `len` bytes
        unsafe { ptr.write_bytes(fill, len) };
        Self { ptr, layout }
    }

    const fn at<T>(&self, offset: u32) -> *mut T {
        // SAFETY: offsets are within the region by construction
        unsafe { self.ptr.add(offset as usize).cast().as_ptr() }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: allocated in `new` with the same layout
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

/// A user pointer taken from an SQE. Carries the provenance of the user's
/// pointer, so miri checks the kernel's accesses like any other.
#[derive(Clone, Copy)]
struct UserPtr(*mut u8);

// SAFETY: the kernel accesses user memory from any thread
unsafe impl Send for UserPtr {}

#[derive(Clone, Copy)]
struct RawCqe {
    user_data: u64,
    res: i32,
    flags: IoringCqeFlags,
}

#[derive(Clone)]
enum Op {
    Nop {
        res: i32,
    },
    Timeout {
        interval: Duration,
        multishot: bool,
        /// Multishot only, 0 = infinite.
        shots_left: u64,
        /// Completes successfully once this many CQEs were posted in total.
        target: Option<u64>,
    },
    Openat {
        dfd: RawFd,
        path: CString,
        flags: OFlags,
        mode: Mode,
    },
    Read {
        fd: RawFd,
        buf: UserPtr,
        len: u32,
        off: u64,
        /// Buffer group to pick the buffer from when the read runs.
        select: Option<u16>,
    },
    Write {
        fd: RawFd,
        buf: UserPtr,
        len: u32,
        off: u64,
    },
}

impl Op {
    /// Stream I/O must wait for readiness instead of blocking the kernel
    /// thread.
    const fn readiness(&self) -> Option<(RawFd, epoll::EventFlags)> {
        match *self {
            Self::Read { fd, off, .. } if off == u64::MAX => Some((fd, epoll::EventFlags::IN)),
            Self::Write { fd, off, .. } if off == u64::MAX => Some((fd, epoll::EventFlags::OUT)),
            _ => None,
        }
    }
}

/// A provided buffer ring registered by the user. The user publishes buffers
/// by writing entries and then the tail (release); the head is the kernel's.
struct BufRing {
    base: UserPtr,
    mask: u16,
    head: u16,
}

struct PickedBuf {
    addr: UserPtr,
    len: u32,
    bid: u16,
}

impl BufRing {
    /// The buffer at the head, without consuming it.
    fn peek(&self) -> Option<PickedBuf> {
        let base = self.base.0;
        // SAFETY: the user keeps the ring memory valid while it's registered.
        // The tail overlaps `resv` of the first entry and is only accessed
        // atomically; the entry fields are read individually so it's never
        // read non-atomically.
        unsafe {
            let tail_offset = mem::offset_of!(buf_ring_tail_struct, tail);
            let tail = AtomicU16::from_ptr(base.add(tail_offset).cast()).load(Ordering::Acquire);
            if tail == self.head {
                return None;
            }
            let entry = base
                .cast::<io_uring_buf>()
                .add((self.head & self.mask) as usize);
            Some(PickedBuf {
                addr: UserPtr((&raw const (*entry).addr.ptr).read().cast()),
                len: (&raw const (*entry).len).read(),
                bid: (&raw const (*entry).bid).read(),
            })
        }
    }
}

enum Status {
    /// In the ready queue, waiting to be executed.
    Queued,
    /// The kernel thread is executing it.
    Running,
    /// Waiting for its fd to become ready. Holds the dup registered in epoll.
    Parked(OwnedFd),
    /// Waiting for a deadline.
    Timer(Instant),
}

struct Request {
    user_data: u64,
    skip_success: bool,
    op: Op,
    status: Status,
    /// Readiness was signaled, the next execution should not block.
    ready: bool,
    /// Cancellation returned `-EALREADY`; the request completes normally if it
    /// can make progress, or with `-ECANCELED` if it would have to wait.
    cancel_requested: bool,
}

struct State {
    next_id: u64,
    requests: BTreeMap<u64, Request>,
    ready: Vec<u64>,
    timers: BTreeSet<(Instant, u64)>,
    /// Completions waiting for the task to enter (IORING_SETUP_DEFER_TASKRUN).
    deferred: VecDeque<RawCqe>,
    /// Completions that didn't fit in the CQ (IORING_FEAT_NODROP).
    overflow: VecDeque<RawCqe>,
    /// Total CQEs posted, for timeouts with a completion count.
    posted: u64,
    /// Registered provided buffer rings, by group id.
    buf_rings: BTreeMap<u16, BufRing>,
    rng: Rng,
    shutdown: bool,
}

struct Kernel {
    rings: Region,
    sqes: Region,

    sq_mask: u32,
    sq_entries: u32,
    cq_mask: u32,
    cq_entries: u32,

    setup: IoringSetupFlags,

    state: Mutex<State>,
    /// Signaled when CQEs are posted.
    completions: Condvar,

    /// Readiness of parked requests; the kernel thread waits on it.
    epoll: OwnedFd,
    /// Eventfd registered in `epoll`, used to wake the kernel thread.
    wake: OwnedFd,
}

impl Kernel {
    fn new(sq_entries: u32, cq_entries: u32, setup: IoringSetupFlags) -> Result<Self, Errno> {
        let sq_size = sq_entries as usize * mem::size_of::<u32>();
        let cq_size = CQ_CQES as usize + cq_entries as usize * mem::size_of::<io_uring_cqe>();
        let rings = Region::new(sq_size.max(cq_size), 0);
        let sqes = Region::new(sq_entries as usize * mem::size_of::<io_uring_sqe>(), POISON);

        // SAFETY: not shared yet
        unsafe {
            rings.at::<u32>(SQ_MASK).write(sq_entries - 1);
            rings.at::<u32>(SQ_ENTRIES).write(sq_entries);
            rings.at::<u32>(CQ_MASK).write(cq_entries - 1);
            rings.at::<u32>(CQ_ENTRIES).write(cq_entries);
        }

        let epoll = epoll::create(epoll::CreateFlags::CLOEXEC)?;
        let wake = event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
        epoll::add(
            &epoll,
            &wake,
            epoll::EventData::new_u64(WAKE_TOKEN),
            epoll::EventFlags::IN,
        )?;

        Ok(Self {
            rings,
            sqes,
            sq_mask: sq_entries - 1,
            sq_entries,
            cq_mask: cq_entries - 1,
            cq_entries,
            setup,
            state: Mutex::new(State {
                next_id: 0,
                requests: BTreeMap::new(),
                ready: Vec::new(),
                timers: BTreeSet::new(),
                deferred: VecDeque::new(),
                overflow: VecDeque::new(),
                posted: 0,
                buf_rings: BTreeMap::new(),
                rng: Rng::new(seed()),
                shutdown: false,
            }),
            completions: Condvar::new(),
            epoll,
            wake,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    const fn atomic(&self, offset: u32) -> &AtomicU32 {
        // SAFETY: the ring indices are only ever accessed atomically
        unsafe { AtomicU32::from_ptr(self.rings.at(offset)) }
    }

    fn cq_ready(&self) -> u32 {
        // the kernel is the only writer of the CQ tail
        let tail = self.atomic(CQ_TAIL).load(Ordering::Relaxed);
        let head = self.atomic(CQ_HEAD).load(Ordering::Acquire);
        tail.wrapping_sub(head)
    }

    fn kick(&self) {
        let _ = io::write(&self.wake, &1u64.to_ne_bytes());
    }

    fn shutdown(&self) {
        let mut state = self.lock();
        state.shutdown = true;
        for (_, req) in mem::take(&mut state.requests) {
            if let Status::Parked(dup) = req.status {
                let _ = epoll::delete(&self.epoll, &dup);
            }
        }
        state.ready.clear();
        state.timers.clear();
        drop(state);

        self.kick();
    }

    // ---- provided buffer rings ------------------------------------------

    fn register_buf_ring(&self, reg: &io_uring_buf_reg) -> Result<(), Errno> {
        // mmap'ed rings (IOU_PBUF_RING_MMAP) and incremental consumption
        // aren't supported yet
        if reg.flags != 0 || reg.resv != [0; 3] {
            return Err(Errno::INVAL);
        }
        if !reg.ring_entries.is_power_of_two() || reg.ring_entries > 1 << 15 {
            return Err(Errno::INVAL);
        }
        let base = reg.ring_addr.ptr.cast::<u8>();
        if base.is_null() {
            return Err(Errno::FAULT);
        }
        if base.addr() % rustix::param::page_size() != 0 {
            return Err(Errno::INVAL);
        }

        let mut state = self.lock();
        if state.buf_rings.contains_key(&reg.bgid) {
            return Err(Errno::EXIST);
        }
        state.buf_rings.insert(
            reg.bgid,
            BufRing {
                base: UserPtr(base),
                mask: (reg.ring_entries - 1) as u16,
                head: 0,
            },
        );
        Ok(())
    }

    fn unregister_buf_ring(&self, reg: &io_uring_buf_reg) -> Result<(), Errno> {
        let mut state = self.lock();
        state
            .buf_rings
            .remove(&reg.bgid)
            .map(drop)
            .ok_or(Errno::NOENT)
    }

    // ---- submission ------------------------------------------------------

    fn submit(&self, state: &mut State, to_submit: u32) -> u32 {
        // the kernel is the only writer of the SQ head
        let head = self.atomic(SQ_HEAD).load(Ordering::Relaxed);
        let tail = self.atomic(SQ_TAIL).load(Ordering::Acquire);
        let count = tail.wrapping_sub(head).min(to_submit).min(self.sq_entries);

        for i in 0..count {
            let index = (head.wrapping_add(i) & self.sq_mask) as usize;
            // SAFETY: the slot is between head and tail, so it was published
            // by the user side and won't be touched until the head moves
            unsafe {
                let slot = self.sqes.ptr.cast::<io_uring_sqe>().add(index).as_ptr();
                self.prep(state, slot);
                // the slot's contents are unspecified once consumed
                slot.cast::<u8>()
                    .write_bytes(POISON, mem::size_of::<io_uring_sqe>());
            }
        }

        self.atomic(SQ_HEAD)
            .store(head.wrapping_add(count), Ordering::Release);

        count
    }

    /// # Safety
    ///
    /// `slot` must point to a published SQE.
    unsafe fn prep(&self, state: &mut State, slot: *const io_uring_sqe) {
        // SAFETY: plain fields of a published SQE
        let (opcode, user_data) = unsafe {
            (
                slot.cast::<u8>().read(),
                ptr::addr_of!((*slot).user_data.u64_).read(),
            )
        };

        // `IoringOp` is an enum, so check the raw opcode before reading the
        // whole SQE
        let known = [
            IoringOp::Nop,
            IoringOp::Timeout,
            IoringOp::AsyncCancel,
            IoringOp::Openat,
            IoringOp::Read,
            IoringOp::Write,
        ];
        if !known.iter().any(|op| *op as u8 == opcode) {
            return self.post(state, user_data, -Errno::INVAL.raw_os_error(), false);
        }

        // SAFETY: the opcode is valid, every other field is plain data
        let sqe = unsafe { slot.read() };
        let skip_success = sqe.flags.contains(IoringSqeFlags::CQE_SKIP_SUCCESS);

        match self.prep_op(state, &sqe) {
            Ok(Some(op)) => self.queue(state, user_data, skip_success, op),
            Ok(None) => {}
            Err(err) => self.post(state, user_data, -err.raw_os_error(), false),
        }
    }

    fn prep_op(&self, state: &mut State, sqe: &io_uring_sqe) -> Result<Option<Op>, Errno> {
        if !SUPPORTED_SQE_FLAGS.contains(sqe.flags) || sqe.personality != 0 {
            return Err(Errno::INVAL);
        }
        // like the kernel, before the op's own checks
        if sqe.flags.contains(IoringSqeFlags::BUFFER_SELECT) && sqe.opcode != IoringOp::Read {
            return Err(Errno::OPNOTSUPP);
        }

        // SAFETY: all union variants read below are plain integers or pointers
        unsafe {
            let addr = sqe.addr_or_splice_off_in.addr.ptr.cast::<u8>();
            let len = sqe.len.len;
            let off = sqe.off_or_addr2.off;
            let op_flags = sqe.op_flags.poll32_events;
            let buf_index = sqe.buf.buf_index;
            let buf_group = sqe.buf.buf_group;
            let file_index = sqe.splice_fd_in_or_file_index_or_addr_len.file_index;

            match sqe.opcode {
                IoringOp::Nop => {
                    if op_flags & !NOP_INJECT_RESULT != 0 {
                        return Err(Errno::INVAL);
                    }
                    let res = if op_flags & NOP_INJECT_RESULT != 0 {
                        len as i32
                    } else {
                        0
                    };
                    Ok(Some(Op::Nop { res }))
                }

                IoringOp::Timeout => {
                    let flags = sqe.op_flags.timeout_flags;
                    if buf_index != 0 || len != 1 || file_index != 0 {
                        return Err(Errno::INVAL);
                    }
                    if !SUPPORTED_TIMEOUT.contains(flags) {
                        return Err(Errno::INVAL);
                    }
                    if addr.is_null() {
                        return Err(Errno::FAULT);
                    }

                    // the kernel copies the timespec during submission
                    let ts = addr.cast::<Timespec>().read_unaligned();
                    if ts.tv_sec < 0 || !(0..1_000_000_000).contains(&ts.tv_nsec) {
                        return Err(Errno::INVAL);
                    }
                    let interval = Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32);

                    let multishot = flags.contains(TIMEOUT_MULTISHOT);
                    let target = (!multishot && off > 0).then(|| state.posted + off);
                    Ok(Some(Op::Timeout {
                        interval,
                        multishot,
                        shots_left: if multishot { off } else { 0 },
                        target,
                    }))
                }

                IoringOp::AsyncCancel => {
                    let flags = sqe.op_flags.cancel_flags;
                    if off != 0 || file_index != 0 {
                        return Err(Errno::INVAL);
                    }
                    let supported = IoringAsyncCancelFlags::ALL | IoringAsyncCancelFlags::ANY;
                    if !supported.contains(flags) {
                        return Err(Errno::INVAL);
                    }

                    // like the kernel, cancellation runs inline during submission
                    let target = sqe.addr_or_splice_off_in.splice_off_in;
                    let res = self.cancel(state, target, flags);
                    let skip = sqe.flags.contains(IoringSqeFlags::CQE_SKIP_SUCCESS);
                    self.post(state, sqe.user_data.u64_, res, skip);
                    Ok(None)
                }

                IoringOp::Openat => {
                    if buf_index != 0 || file_index != 0 {
                        return Err(Errno::INVAL);
                    }
                    if addr.is_null() {
                        return Err(Errno::FAULT);
                    }
                    // the kernel copies the path during submission
                    let path = CStr::from_ptr(addr.cast()).to_owned();
                    Ok(Some(Op::Openat {
                        dfd: sqe.fd,
                        path,
                        flags: sqe.op_flags.open_flags,
                        mode: Mode::from_bits_retain(len),
                    }))
                }

                IoringOp::Read | IoringOp::Write => {
                    // no RWF_* flags, priorities or PI attributes supported yet
                    let attr_type_mask = sqe.addr3_or_cmd.addr3.__pad2[0];
                    if op_flags != 0 || sqe.ioprio.ioprio != 0 || attr_type_mask != 0 {
                        return Err(Errno::INVAL);
                    }
                    let (fd, buf) = (sqe.fd, UserPtr(addr));
                    Ok(Some(if sqe.opcode == IoringOp::Read {
                        let select = sqe
                            .flags
                            .contains(IoringSqeFlags::BUFFER_SELECT)
                            .then_some(buf_group);
                        Op::Read {
                            fd,
                            buf,
                            len,
                            off,
                            select,
                        }
                    } else {
                        Op::Write { fd, buf, len, off }
                    }))
                }

                _ => unreachable!(),
            }
        }
    }

    fn queue(&self, state: &mut State, user_data: u64, skip_success: bool, op: Op) {
        let id = state.next_id;
        state.next_id += 1;

        let status = if let Op::Timeout { interval, .. } = op {
            let deadline = Instant::now() + interval;
            state.timers.insert((deadline, id));
            self.kick();
            Status::Timer(deadline)
        } else {
            state.ready.push(id);
            self.kick();
            Status::Queued
        };

        state.requests.insert(
            id,
            Request {
                user_data,
                skip_success,
                op,
                status,
                ready: false,
                cancel_requested: false,
            },
        );
    }

    // ---- cancellation ----------------------------------------------------

    fn cancel(&self, state: &mut State, target: u64, flags: IoringAsyncCancelFlags) -> i32 {
        let any = flags.contains(IoringAsyncCancelFlags::ANY);
        let all = any || flags.contains(IoringAsyncCancelFlags::ALL);

        let matches = state
            .requests
            .iter()
            .filter(|(_, req)| any || req.user_data == target)
            .map(|(id, _)| *id)
            .take(if all { usize::MAX } else { 1 })
            .collect::<Vec<_>>();

        if matches.is_empty() {
            return -Errno::NOENT.raw_os_error();
        }

        let mut cancelled = 0;
        for id in matches {
            if self.try_cancel(state, id) {
                cancelled += 1;
            }
        }

        match (cancelled, all) {
            (0, _) => -Errno::ALREADY.raw_os_error(),
            (n, true) => n,
            (_, false) => 0,
        }
    }

    /// Returns false if the request may still complete normally (`-EALREADY`).
    fn try_cancel(&self, state: &mut State, id: u64) -> bool {
        let req = state.requests.get_mut(&id).unwrap();

        match req.status {
            Status::Running => {
                req.cancel_requested = true;
                return false;
            }
            Status::Queued => {
                if state.rng.coin() {
                    req.cancel_requested = true;
                    return false;
                }
                state.ready.retain(|other| *other != id);
            }
            Status::Parked(ref dup) => {
                let _ = epoll::delete(&self.epoll, dup);
            }
            Status::Timer(deadline) => {
                state.timers.remove(&(deadline, id));
            }
        }

        let req = state.requests.remove(&id).unwrap();
        self.post(state, req.user_data, -Errno::CANCELED.raw_os_error(), false);
        true
    }

    // ---- completion ------------------------------------------------------

    fn post(&self, state: &mut State, user_data: u64, res: i32, skip_success: bool) {
        self.post_flags(state, user_data, res, IoringCqeFlags::empty(), skip_success);
    }

    fn post_flags(
        &self,
        state: &mut State,
        user_data: u64,
        res: i32,
        flags: IoringCqeFlags,
        skip_success: bool,
    ) {
        if skip_success && res >= 0 {
            return;
        }

        let cqe = RawCqe {
            user_data,
            res,
            flags,
        };

        state.posted += 1;

        if self.setup.contains(IoringSetupFlags::DEFER_TASKRUN) {
            state.deferred.push_back(cqe);
            if self.setup.contains(IoringSetupFlags::TASKRUN_FLAG) {
                self.atomic(SQ_FLAGS)
                    .fetch_or(IoringSqFlags::TASKRUN.bits(), Ordering::Relaxed);
            }
        } else {
            state.overflow.push_back(cqe);
            self.flush_overflow(state);
        }

        self.completions.notify_all();
        self.complete_counted_timeouts(state);
    }

    /// Timeouts with a completion count complete once enough CQEs are posted.
    fn complete_counted_timeouts(&self, state: &mut State) {
        let posted = state.posted;
        let done = state.requests.iter().find_map(|(id, req)| match req.op {
            Op::Timeout {
                target: Some(target),
                ..
            } if posted >= target => Some(*id),
            _ => None,
        });

        if let Some(id) = done {
            let req = state.requests.remove(&id).unwrap();
            if let Status::Timer(deadline) = req.status {
                state.timers.remove(&(deadline, id));
            }
            // recurses through `post` until no counted timeout is left
            self.post(state, req.user_data, 0, req.skip_success);
        }
    }

    /// Moves completions into the CQ, as the task does when entering the ring.
    fn flush(&self, state: &mut State) {
        if !state.deferred.is_empty() {
            state.overflow.extend(state.deferred.drain(..));
            self.atomic(SQ_FLAGS)
                .fetch_and(!IoringSqFlags::TASKRUN.bits(), Ordering::Relaxed);
        }
        self.flush_overflow(state);
    }

    fn flush_overflow(&self, state: &mut State) {
        while let Some(cqe) = state.overflow.front() {
            if !self.try_fill(*cqe) {
                self.atomic(SQ_FLAGS)
                    .fetch_or(IoringSqFlags::CQ_OVERFLOW.bits(), Ordering::Relaxed);
                return;
            }
            state.overflow.pop_front();
        }
        self.atomic(SQ_FLAGS)
            .fetch_and(!IoringSqFlags::CQ_OVERFLOW.bits(), Ordering::Relaxed);
    }

    fn try_fill(&self, cqe: RawCqe) -> bool {
        // the kernel is the only writer of the CQ tail
        let tail = self.atomic(CQ_TAIL).load(Ordering::Relaxed);
        let head = self.atomic(CQ_HEAD).load(Ordering::Acquire);
        if tail.wrapping_sub(head) >= self.cq_entries {
            return false;
        }

        let index = (tail & self.cq_mask) as usize;
        // SAFETY: the slot was released by the user side when it advanced the head
        unsafe {
            let slot = self.rings.at::<io_uring_cqe>(CQ_CQES).add(index);
            ptr::addr_of_mut!((*slot).user_data).write(io_uring_user_data {
                u64_: cqe.user_data,
            });
            ptr::addr_of_mut!((*slot).res).write(cqe.res);
            ptr::addr_of_mut!((*slot).flags).write(cqe.flags);
        }

        self.atomic(CQ_TAIL)
            .store(tail.wrapping_add(1), Ordering::Release);

        true
    }

    // ---- execution -------------------------------------------------------

    /// The kernel thread: waits for readiness and timers, and executes ready
    /// requests one at a time, picked in random order.
    fn run(&self) {
        let mut events = Vec::with_capacity(32);
        let mut state = self.lock();
        loop {
            if state.shutdown {
                return;
            }

            // only block when there's nothing to execute, but always look for
            // readiness and timers, so they aren't starved by ready requests
            let timeout = if state.ready.is_empty() {
                state
                    .timers
                    .first()
                    .map(|(deadline, _)| deadline.saturating_duration_since(Instant::now()))
            } else {
                Some(Duration::ZERO)
            };
            drop(state);
            self.poll(&mut events, timeout);

            state = self.lock();
            if state.shutdown {
                return;
            }
            for event in &events {
                match event.data.u64() {
                    WAKE_TOKEN => {
                        let _ = io::read(&self.wake, &mut [0u8; 8]);
                    }
                    id => self.on_ready(&mut state, id),
                }
            }
            self.fire_timers(&mut state);

            if !state.ready.is_empty() {
                state = self.step(state);
            }
        }
    }

    fn poll(&self, events: &mut Vec<epoll::Event>, timeout: Option<Duration>) {
        let timeout = timeout.map(|wait| EventTimespec {
            tv_sec: wait.as_secs() as _,
            tv_nsec: wait.subsec_nanos() as _,
        });
        events.clear();
        match epoll::wait(&self.epoll, spare_capacity(events), timeout.as_ref()) {
            Ok(_) | Err(Errno::INTR) => {}
            Err(err) => panic!("fake io_uring: epoll_wait failed: {err}"),
        }
    }

    /// Executes (or parks) one random ready request. The lock is released
    /// while it runs, so the user side can race with it.
    fn step<'a>(&'a self, mut state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        let len = state.ready.len();
        let pick = state.rng.below(len);
        let id = state.ready.swap_remove(pick);
        let req = state.requests.get_mut(&id).unwrap();

        if let Some((fd, interest)) = req.op.readiness()
            && !req.ready
        {
            if req.cancel_requested {
                let req = state.requests.remove(&id).unwrap();
                self.post(
                    &mut state,
                    req.user_data,
                    -Errno::CANCELED.raw_os_error(),
                    false,
                );
                return state;
            }

            match self.arm(id, fd, interest) {
                Ok(Some(dup)) => {
                    req.status = Status::Parked(dup);
                    return state;
                }
                // not pollable (regular file), will not block
                Ok(None) => {}
                Err(err) => {
                    let req = state.requests.remove(&id).unwrap();
                    self.post(&mut state, req.user_data, -err.raw_os_error(), false);
                    return state;
                }
            }
        }

        let req = state.requests.get_mut(&id).unwrap();
        req.status = Status::Running;
        req.ready = false;
        let mut op = req.op.clone();

        // pick the buffer only now, when the read can make progress. It is
        // only consumed if the read succeeds; this thread is the only one
        // picking buffers, so nothing else can take it in the meantime.
        let mut selected = None;
        if let Op::Read {
            select: Some(bgid),
            ref mut buf,
            ref mut len,
            ..
        } = op
        {
            let Some(picked) = state.buf_rings.get(&bgid).and_then(BufRing::peek) else {
                let req = state.requests.remove(&id).unwrap();
                self.post(
                    &mut state,
                    req.user_data,
                    -Errno::NOBUFS.raw_os_error(),
                    false,
                );
                return state;
            };
            *buf = picked.addr;
            if *len == 0 || *len > picked.len {
                *len = picked.len;
            }
            selected = Some((bgid, picked.bid));
        }
        drop(state);

        // SAFETY: the SQE contract guarantees user memory stays valid
        // until the completion is posted
        let res = unsafe { execute(&op) };

        let mut state = self.lock();
        let Some(req) = state.requests.get_mut(&id) else {
            // discarded by shutdown
            return state;
        };

        match res {
            Err(Errno::AGAIN) if op.readiness().is_some() => {
                req.status = Status::Queued;
                state.ready.push(id);
            }
            Ok(res) => {
                let req = state.requests.remove(&id).unwrap();
                let mut flags = IoringCqeFlags::empty();
                if let Some((bgid, bid)) = selected {
                    if let Some(ring) = state.buf_rings.get_mut(&bgid) {
                        ring.head = ring.head.wrapping_add(1);
                    }
                    flags = IoringCqeFlags::BUFFER
                        | IoringCqeFlags::from_bits_retain(
                            u32::from(bid) << IORING_CQE_BUFFER_SHIFT,
                        );
                }
                self.post_flags(&mut state, req.user_data, res, flags, req.skip_success);
            }
            Err(err) => {
                let req = state.requests.remove(&id).unwrap();
                self.post(
                    &mut state,
                    req.user_data,
                    -err.raw_os_error(),
                    req.skip_success,
                );
            }
        }
        state
    }

    /// Registers a dup of `fd` in epoll, so the same fd can be watched by
    /// several requests. Returns `None` for fds that can't be polled.
    fn arm(
        &self,
        id: u64,
        fd: RawFd,
        interest: epoll::EventFlags,
    ) -> Result<Option<OwnedFd>, Errno> {
        let dup = io::dup(borrow_fd(fd)?)?;
        match epoll::add(&self.epoll, &dup, epoll::EventData::new_u64(id), interest) {
            Ok(()) => Ok(Some(dup)),
            Err(Errno::PERM) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn on_ready(&self, state: &mut State, id: u64) {
        let Some(req) = state.requests.get_mut(&id) else {
            return;
        };

        if let Status::Parked(dup) = mem::replace(&mut req.status, Status::Queued) {
            let _ = epoll::delete(&self.epoll, &dup);
            req.ready = true;
            state.ready.push(id);
        } else {
            unreachable!("fake io_uring: readiness for a request that isn't parked");
        }
    }

    fn fire_timers(&self, state: &mut State) {
        let now = Instant::now();
        while let Some(&(deadline, id)) = state.timers.first()
            && deadline <= now
        {
            state.timers.pop_first();

            let req = state.requests.get_mut(&id).unwrap();
            let Op::Timeout {
                interval,
                multishot,
                ref mut shots_left,
                ..
            } = req.op
            else {
                unreachable!();
            };

            let etime = -Errno::TIME.raw_os_error();
            if multishot && *shots_left != 1 {
                *shots_left = shots_left.saturating_sub(1);
                let deadline = now + interval;
                req.status = Status::Timer(deadline);
                let user_data = req.user_data;
                state.timers.insert((deadline, id));
                self.post_flags(state, user_data, etime, IoringCqeFlags::MORE, false);
            } else {
                let req = state.requests.remove(&id).unwrap();
                self.post(state, req.user_data, etime, false);
            }
        }
    }
}

const fn borrow_fd<'a>(fd: RawFd) -> Result<BorrowedFd<'a>, Errno> {
    if fd < 0 {
        return Err(Errno::BADF);
    }
    // SAFETY: the SQE contract requires the fd to stay open until completion
    Ok(unsafe { BorrowedFd::borrow_raw(fd) })
}

/// # Safety
///
/// Buffers referenced by `op` must be valid for the access.
unsafe fn execute(op: &Op) -> Result<i32, Errno> {
    let res = match *op {
        Op::Nop { res } => return Ok(res),
        Op::Timeout { .. } => unreachable!("timeouts are fired by the kernel thread"),

        Op::Openat {
            dfd,
            ref path,
            flags,
            mode,
        } => {
            let dir = if dfd == fs::CWD.as_raw_fd() {
                fs::CWD
            } else {
                borrow_fd(dfd)?
            };
            return fs::openat(dir, path.as_c_str(), flags, mode).map(|fd| fd.into_raw_fd());
        }

        Op::Read {
            fd, buf, len, off, ..
        } => {
            let fd = borrow_fd(fd)?;
            let buf = unsafe { user_slice_mut::<MaybeUninit<u8>>(buf, len) }?;
            if off == u64::MAX {
                io::read(fd, buf)?.0.len()
            } else {
                io::pread(fd, buf, off)?.0.len()
            }
        }

        Op::Write { fd, buf, len, off } => {
            let fd = borrow_fd(fd)?;
            let buf = unsafe { user_slice(buf, len) }?;
            if off == u64::MAX {
                io::write(fd, buf)?
            } else {
                io::pwrite(fd, buf, off)?
            }
        }
    };

    Ok(res as i32)
}

/// # Safety
///
/// `buf` must be valid for reads of `len` bytes.
const unsafe fn user_slice<'a>(buf: UserPtr, len: u32) -> Result<&'a [u8], Errno> {
    if len == 0 {
        return Ok(&[]);
    }
    if buf.0.is_null() {
        return Err(Errno::FAULT);
    }
    Ok(unsafe { slice::from_raw_parts(buf.0, len as usize) })
}

/// # Safety
///
/// `buf` must be valid for writes of `len` bytes.
const unsafe fn user_slice_mut<'a, T>(buf: UserPtr, len: u32) -> Result<&'a mut [T], Errno> {
    if len == 0 {
        return Ok(&mut []);
    }
    if buf.0.is_null() {
        return Err(Errno::FAULT);
    }
    Ok(unsafe { slice::from_raw_parts_mut(buf.0.cast(), len as usize) })
}

/// xorshift64*
struct Rng(u64);

impl Rng {
    const fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    const fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    const fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    const fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}
