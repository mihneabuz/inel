use core::{ptr, result::Result};

use rustix::{fd::OwnedFd, io::Errno, mm::*};

// Shared mmap for mapping the io_uring sq and cq in userspace.
// Dropping calls munmap automatically.
pub struct Mmap {
    addr: ptr::NonNull<u8>,
    len: usize,
}

impl Mmap {
    /// # Safety
    /// The underlying file must outlive this mapping. `len` must be non-zero.
    pub unsafe fn shared(fd: &OwnedFd, offset: u64, len: usize) -> Result<Self, Errno> {
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
