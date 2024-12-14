mod buffered;
mod owned;
mod split;

use std::os::fd::AsRawFd;

pub(crate) trait ReadSource: AsRawFd {}
pub(crate) trait WriteSource: AsRawFd {}

pub use buffered::{BufReader, BufWriter, FixedBufReader, FixedBufWriter};
pub use owned::{AsyncReadOwned, AsyncWriteOwned};
pub use split::{ReadHandle, WriteHandle};

pub(crate) use split::{split, split_buffered};
