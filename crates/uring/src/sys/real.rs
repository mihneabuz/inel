//! Real io_uring backend: thin wrappers over the raw syscalls.

use core::{cmp, mem, ptr};

use rustix::{
    fd::AsFd,
    ffi::c_void,
    io::Errno,
    io_uring::{
        IORING_OFF_SQ_RING, IORING_OFF_SQES, IoringEnterFlags, IoringRegisterOp, io_uring_params,
    },
    mm::*,
};

use super::{Cqe, Sqe};

pub use rustix::fd::OwnedFd as RingFd;

pub struct Backing {
    pub rings: Mmap,
    pub sqes: Mmap,
}

pub unsafe fn io_uring_setup(entries: u32, params: &mut io_uring_params) -> Result<RingFd, Errno> {
    unsafe { rustix::io_uring::io_uring_setup(entries, params) }
}

pub unsafe fn io_uring_enter(
    fd: &RingFd,
    to_submit: u32,
    min_complete: u32,
    flags: IoringEnterFlags,
) -> Result<u32, Errno> {
    unsafe { rustix::io_uring::io_uring_enter(fd, to_submit, min_complete, flags) }
}

pub unsafe fn io_uring_register(
    fd: &RingFd,
    opcode: IoringRegisterOp,
    arg: *const c_void,
    nr_args: u32,
) -> Result<u32, Errno> {
    unsafe { rustix::io_uring::io_uring_register(fd, opcode, arg, nr_args) }
}

pub unsafe fn io_uring_map_rings(fd: &RingFd, p: &io_uring_params) -> Result<Backing, Errno> {
    let sq_ring_size = p.sq_off.array as usize + p.sq_entries as usize * mem::size_of::<u32>();
    let cq_ring_size = p.cq_off.cqes as usize + p.cq_entries as usize * mem::size_of::<Cqe>();
    let rings_size = cmp::max(sq_ring_size, cq_ring_size);
    let rings_map = unsafe { Mmap::shared(fd, IORING_OFF_SQ_RING, rings_size) }?;

    let sqes_size = p.sq_entries as usize * mem::size_of::<Sqe>();
    let sqes_map = unsafe { Mmap::shared(fd, IORING_OFF_SQES, sqes_size) }?;

    Ok(Backing {
        rings: rings_map,
        sqes: sqes_map,
    })
}

// Shared mmap for mapping the io_uring sq and cq in userspace.
// Dropping calls munmap automatically.
pub struct Mmap {
    addr: ptr::NonNull<u8>,
    len: usize,
}

impl Mmap {
    /// # Safety
    /// The underlying file must outlive this mapping. `len` must be non-zero.
    pub unsafe fn shared<Fd: AsFd>(fd: Fd, offset: u64, len: usize) -> Result<Self, Errno> {
        unsafe {
            let ptr = mmap(
                ptr::null_mut(),
                len,
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::SHARED | MapFlags::POPULATE,
                fd,
                offset,
            )?;

            madvise(ptr, len, Advice::LinuxDontFork)?;

            Ok(Self {
                addr: ptr::NonNull::new_unchecked(ptr.cast()),
                len,
            })
        }
    }

    #[inline]
    pub const fn addr(&self) -> ptr::NonNull<u8> {
        self.addr
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        unsafe { munmap(self.addr.as_ptr().cast(), self.len).unwrap() };
    }
}
