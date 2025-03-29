use std::{
    fmt::{self, Debug, Formatter},
    io::Result,
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    path::Path,
};

use inel_reactor::{
    op::{self, OpExt},
    AsSource, Source,
};

use crate::{
    io::{ReadSource, WriteSource},
    source::{OwnedDirect, OwnedFd},
    GlobalReactor,
};

#[derive(Clone, Debug)]
pub struct OpenOptions {
    read: bool,
    write: bool,
    append: bool,
    create: bool,
    truncate: bool,
    direct: bool,
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
            direct: false,
        }
    }

    pub fn readable(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    pub fn writable(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    pub fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    pub fn direct(&mut self, direct: bool) -> &mut Self {
        self.direct = direct;
        self
    }

    fn raw_opts(&self) -> (libc::c_int, libc::mode_t) {
        let mut flags: libc::c_int = 0;
        let mode: libc::mode_t = 0o666;

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
        }

        if self.truncate {
            flags |= libc::O_TRUNC;
        }

        if self.direct {
            flags |= libc::O_DIRECT;
        }

        (flags, mode)
    }

    pub async fn open<P: AsRef<Path>>(&self, path: P) -> Result<File> {
        let (flags, mode) = self.raw_opts();

        let fd = op::OpenAt::new(path, flags)
            .mode(mode)
            .run_on(GlobalReactor)
            .await?;

        Ok(unsafe { File::from_raw_fd(fd) })
    }

    pub async fn open_direct<P: AsRef<Path>>(&self, path: P) -> Result<DirectFile> {
        let (flags, mode) = self.raw_opts();

        let slot = op::OpenAt::new(path, flags)
            .mode(mode)
            .direct()
            .run_on(GlobalReactor)
            .await?;

        let direct = OwnedDirect::auto(slot);

        Ok(DirectFile::from_direct(direct))
    }
}

#[derive(Clone)]
pub struct Metadata {
    raw: Box<libc::statx>,
}

impl Metadata {
    // TODO: add more stats
    pub fn is_dir(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFDIR
    }

    pub fn is_file(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFREG
    }

    pub fn is_symlink(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFLNK
    }

    pub fn len(&self) -> u64 {
        self.raw.stx_size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Debug for Metadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metadata").finish()
    }
}

pub struct File {
    fd: OwnedFd,
}

impl Debug for File {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("File").finish()
    }
}

impl File {
    pub fn options() -> OpenOptions {
        OpenOptions::new()
    }

    pub async fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::options().readable(true).open(path).await
    }

    pub async fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::options().create(true).writable(true).open(path).await
    }

    pub async fn open_direct<P: AsRef<Path>>(path: P) -> Result<DirectFile> {
        Self::options().readable(true).open_direct(path).await
    }

    pub async fn create_direct<P: AsRef<Path>>(path: P) -> Result<DirectFile> {
        Self::options()
            .create(true)
            .writable(true)
            .open_direct(path)
            .await
    }

    pub async fn metadata(&self) -> Result<Metadata> {
        let statx = op::Statx::from_fd(self.fd.as_raw())
            .mask(libc::STATX_TYPE | libc::STATX_SIZE)
            .run_on(GlobalReactor)
            .await?;

        Ok(Metadata { raw: statx })
    }

    pub async fn sync_data(&self) -> Result<()> {
        op::Fsync::new(self.fd.as_raw()).run_on(GlobalReactor).await
    }

    pub async fn sync_all(&self) -> Result<()> {
        op::Fsync::new(self.fd.as_raw())
            .sync_meta()
            .run_on(GlobalReactor)
            .await
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            fd: OwnedFd::from_raw(fd),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_raw()
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw()
    }
}

impl ReadSource for File {
    fn read_source(&self) -> Source {
        self.as_raw_fd().as_source()
    }
}

impl WriteSource for File {
    fn write_source(&self) -> Source {
        self.as_raw_fd().as_source()
    }
}

pub struct DirectFile {
    direct: OwnedDirect,
}

impl Debug for DirectFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectFile").finish()
    }
}

impl DirectFile {
    fn from_direct(direct: OwnedDirect) -> Self {
        Self { direct }
    }

    pub async fn sync_data(&self) -> Result<()> {
        op::Fsync::new(self.direct.as_slot())
            .run_on(GlobalReactor)
            .await
    }

    pub async fn sync_all(&self) -> Result<()> {
        op::Fsync::new(self.direct.as_slot())
            .sync_meta()
            .run_on(GlobalReactor)
            .await
    }
}

impl ReadSource for DirectFile {
    fn read_source(&self) -> Source {
        self.direct.as_slot().as_source()
    }
}

impl WriteSource for DirectFile {
    fn write_source(&self) -> Source {
        self.direct.as_slot().as_source()
    }
}
