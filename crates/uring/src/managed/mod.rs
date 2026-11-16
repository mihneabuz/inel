mod buffer;
mod cancellation;
mod completion;
mod fd;
mod group;
mod ops;
mod proxy;
mod submission;

mod uring;

pub use buffer::{Accumulating, IoBuf, IoBufMut, View};
pub use cancellation::Cancellation;
pub use completion::CompletionResult;
pub use fd::{DirectFd, IoSrc, RawSrc, UringFd};
pub use group::{SelectBuf, SelectGroup};
pub use ops::{At, Current, Offset, Open, OpenDirect, Read, ReadSelect, Write};
pub use proxy::UringProxy;
pub(crate) use proxy::UringProxyExt;
pub use submission::{MultiOp, Op, SingleOp, Submission};
pub use uring::Uring;

#[cfg(test)]
mod tests;
