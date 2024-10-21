use std::{
    ffi::CStr,
    io::{ErrorKind, Result},
    os::unix::ffi::OsStrExt,
    path::Path,
};

use inel_reactor::op::{self, Op};

use crate::GlobalReactor;

#[derive(Clone, Debug)]
pub struct DirBuilder {
    recursive: bool,
}

impl Default for DirBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DirBuilder {
    pub fn new() -> Self {
        Self { recursive: false }
    }

    pub fn recursive(&mut self, recursive: bool) -> &mut Self {
        self.recursive = recursive;
        self
    }

    pub async fn create<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        if self.recursive {
            return Self::create_recursive(path).await;
        }

        op::MkDirAt::new(path)
            .mode(libc::S_IRWXU)
            .run_on(GlobalReactor)
            .await
    }

    async fn create_recursive<P: AsRef<Path>>(path: P) -> Result<()> {
        let mut buf = Vec::with_capacity(path.as_ref().as_os_str().len());

        for item in path.as_ref().iter() {
            buf.extend_from_slice(item.as_bytes());
            buf.push(b'\0');

            let cpath = unsafe { CStr::from_ptr(buf.as_ptr() as *const _) };
            let res = op::MkDirAt::from_raw(libc::AT_FDCWD, cpath, 0)
                .mode(libc::S_IRWXU)
                .run_on(GlobalReactor)
                .await;

            if res
                .as_ref()
                .is_err_and(|err| err.kind() != ErrorKind::AlreadyExists)
            {
                return res;
            }

            *buf.last_mut().unwrap() = std::path::MAIN_SEPARATOR as u8;
        }

        Ok(())
    }
}
