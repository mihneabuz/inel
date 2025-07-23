use std::{
    ffi::CString,
    io::Result,
    os::{fd::RawFd, unix::ffi::OsStrExt},
    path::Path,
};

use io_uring::{opcode, squeue::Entry, types::Fd};

use crate::{
    cancellation::Cancellation,
    op::{util, Op},
    ring::RingResult,
};

pub struct LinkAt {
    from_fd: RawFd,
    from_path: CString,
    to_fd: RawFd,
    to_path: CString,
}

impl LinkAt {
    pub fn absolute<P, T>(from: P, to: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        Self::from_raw(RawFd::from(-1), from, RawFd::from(-1), to)
    }

    pub fn relative<P, T>(from: P, to: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        Self::from_raw(libc::AT_FDCWD, from, libc::AT_FDCWD, to)
    }

    fn from_raw<P, T>(from_fd: RawFd, from_path: P, to_fd: RawFd, to_path: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
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

unsafe impl Op for LinkAt {
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

    fn cancel(self) -> Cancellation {
        Cancellation::combine(vec![self.from_path.into(), self.to_path.into()])
    }
}

pub struct SymlinkAt {
    dir: RawFd,
    path: CString,
    target: CString,
}

impl SymlinkAt {
    pub fn new<P, T>(path: P, target: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        if path.as_ref().is_absolute() {
            Self::absolute(path, target)
        } else {
            Self::relative(path, target)
        }
    }

    pub fn absolute<P, T>(path: P, target: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        Self::from_raw(RawFd::from(-1), path, target)
    }

    pub fn relative<P, T>(path: P, target: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        Self::from_raw(libc::AT_FDCWD, path, target)
    }

    fn from_raw<P, T>(dir: RawFd, path: P, target: T) -> Self
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).unwrap();
        let target = CString::new(target.as_ref().as_os_str().as_bytes()).unwrap();
        Self { dir, path, target }
    }
}

unsafe impl Op for SymlinkAt {
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

    fn cancel(self) -> Cancellation {
        Cancellation::combine(vec![self.path.into(), self.target.into()])
    }
}

pub struct UnlinkAt {
    dir: RawFd,
    path: CString,
}

impl UnlinkAt {
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

unsafe impl Op for UnlinkAt {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::UnlinkAt::new(Fd(self.dir), self.path.as_ref().as_ptr()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn cancel(self) -> Cancellation {
        self.path.into()
    }
}
