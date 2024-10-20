use std::{
    fmt::{self, Display, Formatter},
    io::Result,
    mem::MaybeUninit,
    os::fd::{FromRawFd, IntoRawFd, RawFd},
    path::Path,
};

use inel_reactor::op::{self, Op};

use crate::GlobalReactor;

#[derive(Clone, Debug)]
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

    async fn open<P: AsRef<Path>>(&self, path: P) -> Result<File> {
        let (flags, mode) = self.libc_opts();

        let fd = op::OpenAt::new(path, flags)
            .mode(mode)
            .run_on(GlobalReactor)
            .await?;

        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[derive(Clone, Debug)]
pub struct Metadata {
    raw: Box<MaybeUninit<libc::statx>>,
}

impl Metadata {
    // TODO: add more stats
    fn assume_init(&self) -> &libc::statx {
        unsafe { self.raw.assume_init_ref() }
    }

    pub fn is_dir(&self) -> bool {
        (self.assume_init().stx_mode as u32 & libc::S_IFMT) == libc::S_IFDIR
    }

    pub fn is_file(&self) -> bool {
        (self.assume_init().stx_mode as u32 & libc::S_IFMT) == libc::S_IFREG
    }

    pub fn is_symlink(&self) -> bool {
        (self.assume_init().stx_mode as u32 & libc::S_IFMT) == libc::S_IFLNK
    }

    pub fn len(&self) -> u64 {
        self.assume_init().stx_size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Display for Metadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metadata").finish()
    }
}

#[derive(Clone)]
pub struct File {
    fd: RawFd,
}

impl Display for File {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("File").finish()
    }
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

    pub async fn metadata(&self) -> Result<Metadata> {
        let statx = op::Statx::new(self.fd)
            .mask(libc::STATX_TYPE | libc::STATX_SIZE)
            .run_on(GlobalReactor)
            .await?;

        Ok(Metadata { raw: statx })
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let fd = self.fd;
        if fd > 0 {
            crate::spawn(async move {
                op::Close::new(fd).run_on(GlobalReactor).await;
            });
        }
    }
}
