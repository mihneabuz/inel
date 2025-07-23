use std::{
    ffi::{CStr, CString},
    io::Result,
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::Path,
};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{
    op::{util, Op},
    ring::RingResult,
};

pub struct LinkAt<S> {
    from_fd: RawFd,
    from_path: S,
    to_fd: RawFd,
    to_path: S,
}

impl LinkAt<CString> {
    pub fn absolute<P: AsRef<Path>>(from: P, to: P) -> Self {
        Self::from_raw(RawFd::from(-1), from, RawFd::from(-1), to)
    }

    pub fn relative<P: AsRef<Path>>(from: P, to: P) -> Self {
        Self::from_raw(libc::AT_FDCWD, from, libc::AT_FDCWD, to)
    }

    fn from_raw<P: AsRef<Path>>(from_fd: RawFd, from_path: P, to_fd: RawFd, to_path: P) -> Self {
        let from_path = CString::new(from_path.as_ref().as_os_str().as_bytes()).unwrap();
        let to_path = CString::new(to_path.as_ref().as_os_str().as_bytes()).unwrap();
        Self {
            from_fd,
            from_path,
            to_fd,
            to_path,
        }
    }
}

unsafe impl<S: AsRef<CStr>> Op for LinkAt<S> {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::LinkAt::new(
            Fd(self.from_fd),
            self.from_path.as_ref().as_ptr(),
            Fd(self.to_fd),
            self.to_path.as_ref().as_ptr(),
        )
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}

pub struct SymlinkAt<S> {
    dir: RawFd,
    path: S,
    target: S,
}

impl SymlinkAt<CString> {
    pub fn new<P: AsRef<Path>>(path: P, target: P) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path, target)
        } else {
            Self::relative(path, target)
        }
    }

    pub fn absolute<P: AsRef<Path>>(path: P, target: P) -> Self {
        Self::from_raw(RawFd::from(-1), path, target)
    }

    pub fn relative<P: AsRef<Path>>(path: P, target: P) -> Self {
        Self::from_raw(libc::AT_FDCWD, path, target)
    }

    fn from_raw<P: AsRef<Path>>(dir: RawFd, path: P, target: P) -> Self {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        let target = CString::new(target.as_ref().as_os_str().as_bytes()).unwrap();
        Self { dir, path, target }
    }
}

unsafe impl<S: AsRef<CStr>> Op for SymlinkAt<S> {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::SymlinkAt::new(
            Fd(self.dir),
            self.target.as_ref().as_ptr(),
            self.path.as_ref().as_ptr(),
        )
        .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}

pub struct UnlinkAt<S> {
    dir: RawFd,
    path: S,
}

impl UnlinkAt<CString> {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        if path.as_ref().is_absolute() {
            Self::absolute(path)
        } else {
            Self::relative(path)
        }
    }

    pub fn absolute<P: AsRef<Path>>(path: P) -> Self {
        Self::from_raw(RawFd::from(-1), path)
    }

    pub fn relative<P: AsRef<Path>>(path: P) -> Self {
        Self::from_raw(libc::AT_FDCWD, path)
    }

    fn from_raw<P: AsRef<Path>>(dir: RawFd, path: P) -> Self {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        Self { dir, path }
    }
}

unsafe impl<S: AsRef<CStr>> Op for UnlinkAt<S> {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::UnlinkAt::new(Fd(self.dir), self.path.as_ref().as_ptr()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}
