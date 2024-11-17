mod buffered;
mod owned;

use std::os::fd::AsRawFd;

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

pub use buffered::{BufReader, FixedBufReader};
pub use owned::{AsyncReadFixed, AsyncReadOwned, AsyncWriteFixed, AsyncWriteOwned};
