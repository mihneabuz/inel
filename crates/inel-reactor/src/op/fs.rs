use std::{
    ffi::{CStr, CString},
    io::{Error, Result},
    mem::MaybeUninit,
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::Path,
};

use io_uring::{
    opcode::{self, AsyncCancel},
    squeue::Entry,
    types::{Fd, FsyncFlags, OpenHow},
};

use crate::{op::Op, Cancellation, FileSlotKey, IntoTarget, Target};

pub struct OpenAt<S> {
    dir: RawFd,
    path: S,
    flags: libc::c_int,
    mode: libc::mode_t,
}

impl OpenAt<CString> {
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
}

impl<S: AsRef<CStr>> OpenAt<S> {
    pub fn from_raw(dir: RawFd, path: S, flags: libc::c_int, mode: libc::mode_t) -> Self {
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

    pub fn fixed(self, slot: FileSlotKey) -> OpenAtFixed<S> {
        OpenAtFixed::from_raw(self, slot)
    }

    fn raw_entry(&self) -> opcode::OpenAt {
        opcode::OpenAt::new(Fd(self.dir), self.path.as_ref().as_ptr())
            .flags(self.flags)
            .mode(self.mode)
    }
}

unsafe impl<S: AsRef<CStr>> Op for OpenAt<S> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        self.raw_entry().build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            1.. => Ok(ret),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            0 => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}

pub struct OpenAtFixed<S> {
    inner: OpenAt<S>,
    slot: FileSlotKey,
}

impl<S: AsRef<CStr>> OpenAtFixed<S> {
    pub fn from_raw(inner: OpenAt<S>, slot: FileSlotKey) -> Self {
        Self { inner, slot }
    }
}

unsafe impl<S: AsRef<CStr>> Op for OpenAtFixed<S> {
    type Output = Result<FileSlotKey>;

    fn entry(&mut self) -> Entry {
        self.inner
            .raw_entry()
            .file_index(Some(self.slot.as_destination_slot()))
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            0 => Ok(self.slot),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            _ => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        self.inner.cancel(user_data)
    }
}

pub struct OpenAt2<S> {
    dir: RawFd,
    path: S,
    how: OpenHow,
}

impl OpenAt2<CString> {
    pub fn new<P: AsRef<Path>>(path: P, flags: u64) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative<P: AsRef<Path>>(path: P, flags: u64) -> Self {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn absolute<P: AsRef<Path>>(path: P, flags: u64) -> Self {
        Self::relative_to(RawFd::from(-1), path, flags)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P, flags: u64) -> Self {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        Self::from_raw(dir, path, flags, 0, 0)
    }
}

impl<S: AsRef<CStr>> OpenAt2<S> {
    pub fn from_raw(dir: RawFd, path: S, flags: u64, mode: u64, resolve: u64) -> Self {
        let how = OpenHow::new().flags(flags).mode(mode).resolve(resolve);

        Self { dir, path, how }
    }

    pub fn mode(mut self, mode: u64) -> Self {
        self.how = self.how.mode(mode);
        self
    }

    pub fn resolve(mut self, resolve: u64) -> Self {
        self.how = self.how.resolve(resolve);
        self
    }

    pub fn fixed(self, slot: FileSlotKey) -> OpenAt2Fixed<S> {
        OpenAt2Fixed::from_raw(self, slot)
    }

    fn raw_entry(&self) -> opcode::OpenAt2 {
        opcode::OpenAt2::new(Fd(self.dir), self.path.as_ref().as_ptr(), &self.how)
    }
}

unsafe impl<S: AsRef<CStr>> Op for OpenAt2<S> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        self.raw_entry().build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            1.. => Ok(ret),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            0 => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}

pub struct OpenAt2Fixed<S> {
    inner: OpenAt2<S>,
    slot: FileSlotKey,
}

impl<S: AsRef<CStr>> OpenAt2Fixed<S> {
    pub fn from_raw(inner: OpenAt2<S>, slot: FileSlotKey) -> Self {
        Self { inner, slot }
    }
}

unsafe impl<S: AsRef<CStr>> Op for OpenAt2Fixed<S> {
    type Output = Result<FileSlotKey>;

    fn entry(&mut self) -> Entry {
        self.inner
            .raw_entry()
            .file_index(Some(self.slot.as_destination_slot()))
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            0 => Ok(self.slot),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            _ => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        self.inner.cancel(user_data)
    }
}

pub struct Close {
    target: Target,
}

impl Close {
    pub fn new(target: impl IntoTarget) -> Self {
        Self {
            target: target.into_target(),
        }
    }
}

unsafe impl Op for Close {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Close::new(self.target.as_raw()).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(())
        }
    }
}

pub struct Fsync {
    target: Target,
    meta: bool,
}

impl Fsync {
    pub fn new(target: impl IntoTarget) -> Self {
        Self {
            target: target.into_target(),
            meta: false,
        }
    }

    pub fn sync_meta(mut self) -> Self {
        self.meta = true;
        self
    }
}

unsafe impl Op for Fsync {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let flag = if self.meta {
            FsyncFlags::all()
        } else {
            FsyncFlags::DATASYNC
        };

        opcode::Fsync::new(self.target.as_raw()).flags(flag).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            0 => Ok(()),
            ..=-1 => Err(Error::from_raw_os_error(-ret)),
            1.. => unreachable!(),
        }
    }
}

pub struct Statx<P> {
    dir: RawFd,
    path: P,
    flags: libc::c_int,
    mask: libc::c_uint,
    stats: Option<Box<MaybeUninit<libc::statx>>>,
}

impl Statx<&CStr> {
    pub fn from_fd(fd: RawFd) -> Self {
        Self {
            dir: fd,
            path: unsafe { CStr::from_ptr("\0".as_ptr() as *const _) },
            flags: libc::AT_EMPTY_PATH,
            mask: 0,
            stats: Some(Box::new(MaybeUninit::uninit())),
        }
    }
}

impl Statx<CString> {
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

        Self {
            dir,
            path,
            flags: libc::AT_EMPTY_PATH,
            mask: 0,
            stats: Some(Box::new(MaybeUninit::uninit())),
        }
    }
}

impl<P: AsRef<CStr>> Statx<P> {
    pub fn mask(mut self, mask: libc::c_uint) -> Self {
        self.mask = mask;
        self
    }
}

unsafe impl<P: AsRef<CStr>> Op for Statx<P> {
    type Output = Result<Box<MaybeUninit<libc::statx>>>;

    fn entry(&mut self) -> Entry {
        let output = self.stats.as_mut().unwrap().as_mut_ptr();
        opcode::Statx::new(Fd(self.dir), self.path.as_ref().as_ptr(), output as *mut _)
            .flags(self.flags)
            .mask(self.mask)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            0 => Ok(self.stats.unwrap()),
            ..=-1 => Err(Error::from_raw_os_error(-ret)),
            1.. => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            self.stats.unwrap().into(),
        )
    }
}

pub struct MkDirAt<S> {
    dir: RawFd,
    path: S,
    mode: libc::mode_t,
}

impl MkDirAt<CString> {
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
}

impl<S: AsRef<CStr>> MkDirAt<S> {
    pub fn from_raw(dir: RawFd, path: S, mode: libc::mode_t) -> Self {
        Self { dir, path, mode }
    }

    pub fn mode(mut self, mode: libc::mode_t) -> Self {
        self.mode = mode;
        self
    }
}

unsafe impl<S: AsRef<CStr>> Op for MkDirAt<S> {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::MkDirAt::new(Fd(self.dir), self.path.as_ref().as_ptr())
            .mode(self.mode)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        match ret {
            0 => Ok(()),
            ..=-1 => Err(Error::from_raw_os_error(-ret)),
            1.. => unreachable!(),
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}
