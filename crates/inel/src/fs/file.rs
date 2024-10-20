use std::{
    io::Result,
    os::fd::{FromRawFd, IntoRawFd, RawFd},
    path::Path,
};

use inel_reactor::op::{self, Op};

use crate::GlobalReactor;

pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    create: bool,
    truncate: bool,
}

impl OpenOptions {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            read: false,
            write: false,
            append: false,
            create: false,
            truncate: false,
        }
    }

    pub fn readable(mut self, read: bool) -> Self {
        self.read = read;
        self
    }

    pub fn writable(mut self, write: bool) -> Self {
        self.write = write;
        self
    }

    pub fn append(mut self, append: bool) -> Self {
        self.append = append;
        self
    }

    pub fn create(mut self, create: bool) -> Self {
        self.create = create;
        self
    }

    pub fn truncate(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    fn libc_opts(&self) -> (libc::c_int, libc::mode_t) {
        let mut flags: libc::c_int = 0;
        // TODO: handle mode flags properly :)
        let mut mode: libc::mode_t = 0;

        flags |= match (self.read, self.write) {
            (true, true) => libc::O_RDWR,
            (true, false) => libc::O_RDONLY,
            (false, true) => libc::O_WRONLY,
            (false, false) => 0,
        };

        if self.append {
            flags |= libc::O_APPEND;
        }

        if self.create {
            flags |= libc::O_CREAT;
            mode |= libc::S_IWUSR;
        }

        if self.truncate {
            flags |= libc::O_TRUNC;
        }

        (flags, mode)
    }

    async fn open<P: AsRef<Path>>(self, path: P) -> Result<File> {
        let (flags, mode) = self.libc_opts();

        let fd = op::OpenAt::new(path, flags)
            .mode(mode)
            .run_on(GlobalReactor)
            .await?;

        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

pub struct File {
    fd: RawFd,
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self { fd }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.fd
    }
}

impl File {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        OpenOptions::new().readable(true).open(path).await
    }

    pub async fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        OpenOptions::new()
            .create(true)
            .writable(true)
            .open(path)
            .await
    }

    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    pub async fn metadata(&self) -> Result<Self> {
        todo!();
    }
}
