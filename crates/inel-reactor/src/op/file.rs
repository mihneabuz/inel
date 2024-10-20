use std::{
    io::{Error, Result},
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::Path,
};

use io_uring::{
    opcode::{self, AsyncCancel},
    squeue::Entry,
    types::{Fd, OpenHow},
};

use crate::{cancellation::Cancellation, op::Op};

pub struct OpenAt<P> {
    dir: RawFd,
    path: P,
    flags: libc::c_int,
    mode: libc::mode_t,
}

impl<P: AsRef<Path>> OpenAt<P> {
    pub fn new(path: P, flags: libc::c_int) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative(path: P, flags: libc::c_int) -> Self {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn relative_to(dir: RawFd, path: P, flags: libc::c_int) -> Self {
        Self {
            dir,
            path,
            flags,
            mode: 0,
        }
    }

    pub fn absolute(path: P, flags: libc::c_int) -> Self {
        Self {
            dir: RawFd::from(-1),
            path,
            flags,
            mode: 0,
        }
    }

    pub fn mode(mut self, mode: libc::mode_t) -> Self {
        self.mode = mode;
        self
    }
}

unsafe impl<P: AsRef<Path>> Op for OpenAt<P> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        let path = self.path.as_ref().as_os_str().as_bytes().as_ptr();
        opcode::OpenAt::new(Fd(self.dir), path as *const _)
            .flags(self.flags)
            .mode(self.mode)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(ret))
        } else {
            Ok(ret)
        }
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((AsyncCancel::new(user_data).build(), Cancellation::empty()))
    }
}

pub struct OpenAt2<P> {
    dir: RawFd,
    path: P,
    how: OpenHow,
}

impl<P: AsRef<Path>> OpenAt2<P> {
    pub fn new(path: P, flags: libc::__u64) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path, flags)
        } else {
            Self::relative(path, flags)
        }
    }

    pub fn relative(path: P, flags: libc::__u64) -> Self {
        Self::relative_to(libc::AT_FDCWD, path, flags)
    }

    pub fn relative_to(dir: RawFd, path: P, flags: libc::__u64) -> Self {
        let how = OpenHow::new().flags(flags);
        Self { dir, path, how }
    }

    pub fn absolute(path: P, flags: libc::__u64) -> Self {
        let dir = RawFd::from(-1);
        let how = OpenHow::new().flags(flags);
        Self { dir, path, how }
    }

    pub fn mode(mut self, mode: libc::__u64) -> Self {
        self.how = self.how.mode(mode);
        self
    }

    pub fn resolve(mut self, resolve: libc::__u64) -> Self {
        self.how = self.how.resolve(resolve);
        self
    }

    pub fn from_raw(dir: RawFd, path: P, how: libc::open_how) -> Self {
        let how = OpenHow::new()
            .flags(how.flags)
            .mode(how.mode)
            .resolve(how.resolve);

        Self { dir, path, how }
    }
}

unsafe impl<P: AsRef<Path>> Op for OpenAt2<P> {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        let path = self.path.as_ref().as_os_str().as_bytes().as_ptr();
        opcode::OpenAt2::new(Fd(self.dir), path as *const _, &self.how).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(ret))
        } else {
            Ok(ret)
        }
    }

    fn cancel(&mut self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((AsyncCancel::new(user_data).build(), Cancellation::empty()))
    }
}
