use std::{
    io::{Error, Result},
    ops::RangeTo,
};

use super::{generic::*, FixedBufWriter};
use crate::{
    buffer::{Fixed, View},
    io::{owned::WriteOwned, AsyncWriteOwned, WriteSource},
};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type BoxBuf = Box<[u8]>;
type OwnedWrite = WriteOwned<View<BoxBuf, RangeTo<usize>>>;

struct OwnedAdapter;
impl<S: WriteSource> BufWriterAdapter<S, BoxBuf, OwnedWrite> for OwnedAdapter {
    fn create_future(&self, sink: &mut S, buffer: View<BoxBuf, RangeTo<usize>>) -> OwnedWrite {
        sink.write_owned(buffer)
    }
}

pub struct BufWriter<S>(BufWriterInner<S, BoxBuf, OwnedWrite, OwnedAdapter>);

impl<S> BufWriter<S> {
    pub fn new(sink: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, sink)
    }

    pub fn with_capacity(capacity: usize, sink: S) -> Self {
        Self(BufWriterInner::empty(
            vec![0; capacity].into_boxed_slice(),
            sink,
            OwnedAdapter,
        ))
    }

    pub fn fix(self) -> Result<FixedBufWriter<S>> {
        let (source, buf) = self.0.into_raw_parts();

        let (buf, len) = buf.ok_or(Error::other("BufWriter was in use"))?;
        let fixed = Fixed::register(buf)?;

        Ok(FixedBufWriter::from_raw(source, fixed, len))
    }
}

impl_bufwriter!(BufWriter);
