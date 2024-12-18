mod buffered;
mod owned;
mod split;

use std::os::fd::{AsRawFd, RawFd};

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

pub use buffered::{BufReader, BufWriter, FixedBufReader, FixedBufWriter};
pub use owned::{AsyncReadOwned, AsyncWriteOwned};
pub use split::{ReadHandle, WriteHandle};

pub(crate) use split::{split, split_buffered};

pub fn stdin() -> Stdin {
    Stdin(())
}

pub fn stdout() -> Stdout {
    Stdout(())
}

pub struct Stdin(());
pub struct Stdout(());

impl AsRawFd for Stdin {
    fn as_raw_fd(&self) -> RawFd {
        libc::STDIN_FILENO
    }
}

impl AsRawFd for Stdout {
    fn as_raw_fd(&self) -> RawFd {
        libc::STDOUT_FILENO
    }
}

impl ReadSource for Stdin {}
impl WriteSource for Stdout {}
