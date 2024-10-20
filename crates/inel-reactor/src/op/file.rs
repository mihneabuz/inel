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
    types::{Fd, OpenHow},
};

use crate::{cancellation::Cancellation, op::Op};

pub struct OpenAt<S> {
    dir: RawFd,
    path: S,
    flags: libc::c_int,
    mode: libc::mode_t,
}

impl OpenAt<CString> {
    pub fn new<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Option<Self> {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Option<Self> {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn absolute<P: AsRef<Path>>(path: P, flags: libc::c_int) -> Option<Self> {
        Self::relative_to(RawFd::from(-1), path, flags)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P, flags: libc::c_int) -> Option<Self> {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).ok()?;

        Some(Self {
            dir,
            path,
            flags,
            mode: 0,
        })
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
}

unsafe impl<S: AsRef<CStr>> Op for OpenAt<S> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::OpenAt::new(Fd(self.dir), self.path.as_ref().as_ptr())
            .flags(self.flags)
            .mode(self.mode)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret)
        }
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((AsyncCancel::new(user_data).build(), Cancellation::empty()))
    }
}

pub struct OpenAt2<S> {
    dir: RawFd,
    path: S,
    how: OpenHow,
}

impl OpenAt2<CString> {
    pub fn new<P: AsRef<Path>>(path: P, flags: libc::__u64) -> Option<Self> {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative<P: AsRef<Path>>(path: P, flags: libc::__u64) -> Option<Self> {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn absolute<P: AsRef<Path>>(path: P, flags: libc::__u64) -> Option<Self> {
        Self::relative_to(RawFd::from(-1), path, flags)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P, flags: libc::__u64) -> Option<Self> {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).ok()?;
        let how = OpenHow::new().flags(flags);
        Some(Self { dir, path, how })
    }
}

impl<S: AsRef<CStr>> OpenAt2<S> {
    pub fn from_raw(dir: RawFd, path: S, how: libc::open_how) -> Option<Self> {
        let how = OpenHow::new()
            .flags(how.flags)
            .mode(how.mode)
            .resolve(how.resolve);

        Some(Self { dir, path, how })
    }

    pub fn mode(mut self, mode: libc::__u64) -> Self {
        self.how = self.how.mode(mode);
        self
    }

    pub fn resolve(mut self, resolve: libc::__u64) -> Self {
        self.how = self.how.resolve(resolve);
        self
    }
}

unsafe impl<S: AsRef<CStr>> Op for OpenAt2<S> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::OpenAt2::new(Fd(self.dir), self.path.as_ref().as_ptr(), &self.how).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret)
        }
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((AsyncCancel::new(user_data).build(), Cancellation::empty()))
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
    pub fn new(fd: RawFd) -> Self {
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
    pub fn relative<P: AsRef<Path>>(path: P) -> Option<Self> {
        Self::relative_to(libc::AT_FDCWD, path)
    }

    pub fn relative_to<P: AsRef<Path>>(dir: RawFd, path: P) -> Option<Self> {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).ok()?;

        Some(Self {
            dir,
            path,
            flags: libc::AT_EMPTY_PATH,
            mask: 0,
            stats: Some(Box::new(MaybeUninit::uninit())),
        })
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
            -1.. => Err(Error::from_raw_os_error(-ret)),
            _ => unreachable!(),
        }
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((
            AsyncCancel::new(user_data).build(),
            self.stats.take().unwrap().into(),
        ))
    }
}
