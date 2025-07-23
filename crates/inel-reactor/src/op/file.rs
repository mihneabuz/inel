use std::{
    ffi::CString,
    io::Result,
    mem::MaybeUninit,
    ops::{RangeBounds, RangeFull},
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::Path,
};

use io_uring::{
    opcode,
    squeue::Entry,
    types::{DestinationSlot, Fd, FsyncFlags},
};

use crate::{
    cancellation::Cancellation,
    op::{util, DetachOp, Op},
    ring::{DirectSlot, RingResult},
    source::{AsDirectSlot, AsSource, DirectAutoFd, Source},
};

pub struct OpenAt {
    dir: RawFd,
    path: CString,
    flags: libc::c_int,
    mode: libc::mode_t,
}

impl OpenAt {
    pub fn new<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Self {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn absolute<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Self {
        Self::relative_to(RawFd::from(-1), path, flags)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P, flags: libc::c_int) -> Self {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        Self::from_raw(dir, path, flags, 0)
    }

    pub fn from_raw(dir: RawFd, path: CString, flags: libc::c_int, mode: libc::mode_t) -> Self {
        Self {
            dir,
            path,
            flags,
            mode,
        }
    }

    pub fn mode(mut self, mode: libc::mode_t) -> Self {
        self.mode = mode;
        self
    }

    pub fn direct(self, direct: &impl AsDirectSlot) -> OpenAtDirect {
        OpenAtDirect::from_raw(self, direct.as_slot())
    }

    pub fn auto(self) -> OpenAtAuto {
        OpenAtAuto::from_raw(self)
    }

    fn raw_entry(&self) -> opcode::OpenAt {
        opcode::OpenAt::new(Fd(self.dir), self.path.as_ref().as_ptr())
            .flags(self.flags)
            .mode(self.mode)
    }
}

unsafe impl Op for OpenAt {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        self.raw_entry().build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_fd(&res)
    }

    fn cancel(self) -> Cancellation {
        self.path.into()
    }
}

pub struct OpenAtDirect {
    inner: OpenAt,
    slot: DestinationSlot,
}

impl OpenAtDirect {
    fn from_raw(inner: OpenAt, slot: &DirectSlot) -> Self {
        Self {
            inner,
            slot: slot.as_destination_slot(),
        }
    }
}

unsafe impl Op for OpenAtDirect {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        self.inner.raw_entry().file_index(Some(self.slot)).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn cancel(self) -> Cancellation {
        self.inner.cancel()
    }
}

pub struct OpenAtAuto {
    inner: OpenAt,
}

impl OpenAtAuto {
    pub fn from_raw(inner: OpenAt) -> Self {
        Self { inner }
    }
}

unsafe impl Op for OpenAtAuto {
    type Output = Result<DirectAutoFd>;

    fn entry(&mut self) -> Entry {
        self.inner
            .raw_entry()
            .file_index(Some(DestinationSlot::auto_target()))
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_direct(&res)
    }

    fn cancel(self) -> Cancellation {
        self.inner.cancel()
    }
}

pub struct Close {
    src: Source,
}

impl Close {
    pub fn new(source: &impl AsSource) -> Self {
        Self {
            src: source.as_source(),
        }
    }
}

unsafe impl Op for Close {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Close::new(self.src.as_raw()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

impl DetachOp for Close {}

pub struct FSync {
    src: Source,
    meta: bool,
}

impl FSync {
    pub fn new(source: &impl AsSource) -> Self {
        Self {
            src: source.as_source(),
            meta: false,
        }
    }

    pub fn sync_meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

unsafe impl Op for FSync {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let flag = if self.meta {
            FsyncFlags::all()
        } else {
            FsyncFlags::DATASYNC
        };

        opcode::Fsync::new(self.src.as_raw()).flags(flag).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}

pub struct Statx {
    dir: RawFd,
    mask: libc::c_uint,
    stats: Box<MaybeUninit<libc::statx>>,
}

impl Statx {
    pub fn new(fd: RawFd, mask: libc::c_uint) -> Self {
        Self {
            dir: fd,
            mask,
            stats: Box::new_uninit(),
        }
    }
}

unsafe impl Op for Statx {
    type Output = Result<Box<libc::statx>>;

    fn entry(&mut self) -> Entry {
        let output = self.stats.as_mut().as_mut_ptr();
        opcode::Statx::new(Fd(self.dir), c"".as_ptr(), output as *mut _)
            .flags(libc::AT_EMPTY_PATH)
            .mask(self.mask)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res).map(|_| unsafe { self.stats.assume_init() })
    }

    fn cancel(self) -> Cancellation {
        self.stats.into()
    }
}

pub struct MkDirAt {
    dir: RawFd,
    path: CString,
    mode: libc::mode_t,
}

impl MkDirAt {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path)
        } else {
            Self::relative(path)
        }
    }

    pub fn relative<P: AsRef<Path>>(path: P) -> Self {
        Self::relative_to(libc::AT_FDCWD, path)
    }

    pub fn absolute<P: AsRef<Path>>(path: P) -> Self {
        Self::relative_to(RawFd::from(-1), path)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P) -> Self {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        Self { dir, path, mode: 0 }
    }

    pub fn from_raw(dir: RawFd, path: CString, mode: libc::mode_t) -> Self {
        Self { dir, path, mode }
    }

    pub fn mode(mut self, mode: libc::mode_t) -> Self {
        self.mode = mode;
        self
    }
}

unsafe impl Op for MkDirAt {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::MkDirAt::new(Fd(self.dir), self.path.as_ref().as_ptr())
            .mode(self.mode)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn cancel(self) -> Cancellation {
        self.path.into()
    }
}

pub struct FTruncate {
    src: Source,
    len: usize,
}

impl FTruncate {
    pub fn new(src: &impl AsSource, len: usize) -> Self {
        Self {
            src: src.as_source(),
            len,
        }
    }
}

unsafe impl Op for FTruncate {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Ftruncate::new(self.src.as_raw(), self.len as u64).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}

pub struct FAdvise<R> {
    src: Source,
    advice: i32,
    range: R,
}

impl FAdvise<RangeFull> {
    pub fn new(src: &impl AsSource, advice: i32) -> Self {
        Self {
            src: src.as_source(),
            advice,
            range: (..),
        }
    }

    pub fn range<R>(self, range: R) -> FAdvise<R> {
        let Self { src, advice, .. } = self;
        FAdvise { src, advice, range }
    }
}

unsafe impl<R> Op for FAdvise<R>
where
    R: RangeBounds<usize>,
{
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let offset = match self.range.start_bound() {
            std::ops::Bound::Included(i) => *i,
            std::ops::Bound::Excluded(i) => *i + 1,
            std::ops::Bound::Unbounded => 0,
        };

        let len = match self.range.end_bound() {
            std::ops::Bound::Included(i) => *i + 1,
            std::ops::Bound::Excluded(i) => *i,
            std::ops::Bound::Unbounded => 0,
        };

        opcode::Fadvise::new(self.src.as_raw(), len as i64, self.advice)
            .offset(offset as u64)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct FAllocate {
    src: Source,
    size: usize,
    offset: usize,
}

impl FAllocate {
    pub fn new(src: &impl AsSource, size: usize) -> Self {
        Self {
            src: src.as_source(),
            size,
            offset: 0,
        }
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}

unsafe impl Op for FAllocate {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Fallocate::new(self.src.as_raw(), self.size as u64)
            .offset(self.offset as u64)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}
