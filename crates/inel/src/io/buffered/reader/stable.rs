use std::io::{Error, Result};

use super::{fixed::FixedBufReader, generic::*};
use crate::io::{owned::ReadOwned, AsyncReadOwned, ReadSource};

const DEFAULT_BUF_SIZE: usize = 4096 * 2;

type BoxBuf = Box<[u8]>;

struct OwnedAdapter;
impl<S: ReadSource> BufReaderAdapter<S, BoxBuf, ReadOwned<BoxBuf>> for OwnedAdapter {
    fn create_future(&self, source: &mut S, buffer: BoxBuf) -> ReadOwned<BoxBuf> {
        source.read_owned(buffer)
    }
}

pub struct BufReader<S>(BufReaderInner<S, BoxBuf, ReadOwned<BoxBuf>, OwnedAdapter>);

impl<S> BufReader<S> {
    pub fn new(source: S) -> Self {
        Self::with_capacity(DEFAULT_BUF_SIZE, source)
    }

    pub fn with_capacity(capacity: usize, source: S) -> Self {
        Self(BufReaderInner::empty(
            vec![0; capacity].into_boxed_slice(),
            source,
            OwnedAdapter,
        ))
    }

    pub fn fix(self) -> Result<FixedBufReader<S>> {
        let (source, buf) = self.0.into_raw_parts();
        let (buf, pos, filled) = buf.ok_or(Error::other("BufReader was in use"))?;
        FixedBufReader::from_raw(source, buf, pos, filled)
    }
}

impl_bufreader!(BufReader);
