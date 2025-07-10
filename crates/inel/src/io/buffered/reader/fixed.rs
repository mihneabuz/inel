use std::io::Result;

use super::generic::*;
use crate::{
    buffer::Fixed,
    io::{owned::ReadFixed, AsyncReadOwned, ReadSource},
};

type FixedRead = ReadFixed<Fixed>;

struct FixedAdapter;
impl<S: ReadSource> BufReaderAdapter<S, Fixed, FixedRead> for FixedAdapter {
    fn create_future(&self, source: &mut S, buffer: Fixed) -> FixedRead {
        source.read_fixed(buffer)
    }
}

pub struct FixedBufReader<S>(BufReaderInner<S, Fixed, FixedRead, FixedAdapter>);

impl<S> FixedBufReader<S> {
    pub(crate) fn from_raw(
        source: S,
        buffer: Box<[u8]>,
        pos: usize,
        filled: usize,
    ) -> Result<Self> {
        Ok(Self(BufReaderInner::new(
            Fixed::register(buffer)?,
            source,
            pos,
            filled,
            FixedAdapter,
        )))
    }
}

impl_bufreader!(FixedBufReader);
