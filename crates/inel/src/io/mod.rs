mod buffered;
mod owned;

use std::os::fd::AsRawFd;

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

pub use buffered::{BufReader, BufWriter, FixedBufReader, FixedBufWriter};
pub use owned::{AsyncReadOwned, AsyncWriteOwned};
