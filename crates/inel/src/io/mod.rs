mod buffered;
mod owned;
mod split;

use std::os::fd::{AsRawFd, RawFd};

pub trait ReadSource {
    fn read_source(&self) -> Source;
}

pub trait WriteSource {
    fn write_source(&self) -> Source;
}

pub use buffered::*;
use inel_reactor::source::{AsSource, Source};
pub use owned::{AsyncReadOwned, AsyncWriteOwned};
pub use split::{ReadHandle, Split, WriteHandle};

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

impl ReadSource for Stdin {
    fn read_source(&self) -> Source {
        self.as_raw_fd().as_source()
    }
}

impl WriteSource for Stdout {
    fn write_source(&self) -> Source {
        self.as_raw_fd().as_source()
    }
}
