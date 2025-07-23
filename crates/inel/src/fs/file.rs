use std::{
    fmt::{self, Debug, Formatter},
    io::{Error, Result},
    ops::{BitAnd, BitOr},
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use inel_reactor::{
    op::{self, OpExt},
    source::{AsSource, Source},
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
    permissions: Permissions,
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
            permissions: Permissions::READ | Permissions::WRITE,
        }
    }

    pub const fn readable(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    pub const fn writable(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    pub const fn append(&mut self, append: bool) -> &mut Self {
        self.append = append;
        self
    }

    pub const fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    pub const fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    pub const fn permissions(&mut self, perms: Permissions) -> &mut Self {
        self.permissions = perms;
        self
    }

    pub const fn direct(&mut self, direct: bool) -> &mut Self {
        self.direct = direct;
        self
    }

    const fn raw_opts(&self) -> (libc::c_int, libc::mode_t) {
        let mut flags = 0;

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

        (flags, self.permissions.as_raw())
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
            .auto()
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
    pub const fn is_dir(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFDIR
    }

    pub const fn is_file(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFREG
    }

    pub const fn is_symlink(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFLNK
    }

    pub const fn is_block_device(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFBLK
    }

    pub const fn is_char_device(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFCHR
    }

    pub const fn is_fifo(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFIFO
    }

    pub const fn is_socket(&self) -> bool {
        (self.raw.stx_mode as u32 & libc::S_IFMT) == libc::S_IFSOCK
    }

    pub const fn user_id(&self) -> u16 {
        self.raw.stx_uid as u16
    }

    pub const fn group_id(&self) -> u16 {
        self.raw.stx_gid as u16
    }

    pub fn accessed(&self) -> Result<SystemTime> {
        Self::convert_timestamp(&self.raw.stx_atime)
    }

    pub fn created(&self) -> Result<SystemTime> {
        Self::convert_timestamp(&self.raw.stx_btime)
    }

    pub fn modified(&self) -> Result<SystemTime> {
        Self::convert_timestamp(&self.raw.stx_mtime)
    }

    pub const fn permissions(&self) -> Permissions {
        Permissions {
            raw: self.raw.stx_mode as libc::mode_t,
        }
    }

    pub const fn len(&self) -> u64 {
        self.raw.stx_size
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn convert_timestamp(raw: &libc::statx_timestamp) -> Result<SystemTime> {
        let (sec, nsec) = (raw.tv_sec, raw.tv_nsec);
        if sec >= 0 {
            UNIX_EPOCH
                .checked_add(Duration::new(sec as u64, nsec))
                .ok_or(Error::other("Overflow"))
        } else {
            UNIX_EPOCH
                .checked_sub(Duration::new((-sec) as u64, nsec))
                .ok_or(Error::other("Underflow"))
        }
    }
}

impl Debug for Metadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metadata").finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Permissions {
    raw: libc::mode_t,
}

#[derive(Clone)]
pub enum Permission {
    UserRead,
    UserWrite,
    UserExecute,
    GroupRead,
    GroupWrite,
    GroupExecute,
    OtherRead,
    OtherWrite,
    OtherExecute,
}

impl Permission {
    const fn raw_flag(&self) -> libc::mode_t {
        match self {
            Permission::UserRead => libc::S_IRUSR,
            Permission::UserWrite => libc::S_IWUSR,
            Permission::UserExecute => libc::S_IXUSR,
            Permission::GroupRead => libc::S_IRGRP,
            Permission::GroupWrite => libc::S_IWGRP,
            Permission::GroupExecute => libc::S_IXGRP,
            Permission::OtherRead => libc::S_IROTH,
            Permission::OtherWrite => libc::S_IWOTH,
            Permission::OtherExecute => libc::S_IXOTH,
        }
    }
}

impl Permissions {
    pub const EMPTY: Self = Self { raw: 0 };
    pub const READ: Self = Self { raw: 0o444 };
    pub const WRITE: Self = Self { raw: 0o222 };
    pub const EXECUTE: Self = Self { raw: 0o111 };

    const fn as_raw(&self) -> libc::mode_t {
        self.raw
    }

    pub const fn is_empty(&self) -> bool {
        self.raw == 0
    }

    pub const fn get(&self, which: Permission) -> bool {
        (self.raw & which.raw_flag()) != 0
    }

    pub const fn set(mut self, which: Permission) -> Self {
        self.raw |= which.raw_flag();
        self
    }

    pub const fn unset(mut self, which: Permission) -> Self {
        self.raw &= !which.raw_flag();
        self
    }
}

impl BitAnd for Permissions {
    type Output = Permissions;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self {
            raw: self.raw & rhs.raw,
        }
    }
}

impl BitOr for Permissions {
    type Output = Permissions;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            raw: self.raw | rhs.raw,
        }
    }
}

impl Debug for Permissions {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Permissions").finish()
    }
}

pub enum Advice {
    Normal,
    Sequential,
    Random,
}

impl Advice {
    fn as_raw(&self) -> i32 {
        match self {
            Advice::Normal => libc::POSIX_FADV_NORMAL,
            Advice::Sequential => libc::POSIX_FADV_SEQUENTIAL,
            Advice::Random => libc::POSIX_FADV_RANDOM,
        }
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
        let statx = op::Statx::new(
            self.fd.as_raw(),
            libc::STATX_BASIC_STATS | libc::STATX_BTIME,
        )
        .run_on(GlobalReactor)
        .await?;

        Ok(Metadata { raw: statx })
    }

    pub async fn sync_data(&self) -> Result<()> {
        op::FSync::new(&self.fd).run_on(GlobalReactor).await
    }

    pub async fn sync_all(&self) -> Result<()> {
        op::FSync::new(&self.fd)
            .sync_meta()
            .run_on(GlobalReactor)
            .await
    }

    pub async fn truncate(&self, len: usize) -> Result<()> {
        op::FTruncate::new(&self.fd, len)
            .run_on(GlobalReactor)
            .await
    }

    pub async fn advise(&self, advice: Advice) -> Result<()> {
        op::FAdvise::new(&self.fd, advice.as_raw())
            .run_on(GlobalReactor)
            .await
    }

    pub async fn make_direct(&self) -> Result<DirectFile> {
        let slot = op::RegisterFile::new(self.fd.as_raw())
            .run_on(GlobalReactor)
            .await?;

        Ok(DirectFile::from_direct(OwnedDirect::Auto(slot)))
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
        self.fd.as_source()
    }
}

impl WriteSource for File {
    fn write_source(&self) -> Source {
        self.fd.as_source()
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
        op::FSync::new(&self.direct).run_on(GlobalReactor).await
    }

    pub async fn sync_all(&self) -> Result<()> {
        op::FSync::new(&self.direct)
            .sync_meta()
            .run_on(GlobalReactor)
            .await
    }

    pub async fn truncate(&self, len: usize) -> Result<()> {
        op::FTruncate::new(&self.direct, len)
            .run_on(GlobalReactor)
            .await
    }

    pub async fn advise(&self, advice: Advice) -> Result<()> {
        op::FAdvise::new(&self.direct, advice.as_raw())
            .run_on(GlobalReactor)
            .await
    }

    pub async fn make_regular(&self) -> Result<File> {
        let fd = op::InstallSlot::new(&self.direct)
            .run_on(GlobalReactor)
            .await?;

        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl ReadSource for DirectFile {
    fn read_source(&self) -> Source {
        self.direct.as_source()
    }
}

impl WriteSource for DirectFile {
    fn write_source(&self) -> Source {
        self.direct.as_source()
    }
}
