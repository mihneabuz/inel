use std::io::Result;

use io_uring::{opcode, squeue::Entry};

use crate::{Cancellation, Ring};

use super::{util, Op, OpExt};

pub struct BufferGroup<R> {
    buffers: Vec<Option<Box<[u8]>>>,
    group: u16,
    reactor: R,
}

impl<R> BufferGroup<R>
where
    R: inel_interface::Reactor<Handle = Ring> + Clone,
{
    pub async fn new(count: usize, size: usize, reactor: R) -> Self {
        let mut buffers = Vec::with_capacity(count);
        let group = 1;

        for i in 0..count {
            let buf = ProvideBuffer::new(Box::from(vec![0; size]), i as u16, group)
                .run_on(reactor.clone())
                .await
                .unwrap();

            buffers.push(Some(buf));
        }

        Self {
            buffers,
            group,
            reactor,
        }
    }

    pub fn take(&mut self, id: u16) -> TempBuffer {
        let buffer = self.buffers[id as usize].take().unwrap();
        TempBuffer { inner: buffer, id }
    }

    pub async fn put(&mut self, buffer: TempBuffer) {
        let TempBuffer { id, inner } = buffer;

        let buffer = ProvideBuffer::new(inner, id, self.group)
            .run_on(self.reactor.clone())
            .await
            .unwrap();

        self.buffers[id as usize] = Some(buffer);
    }
}

pub struct TempBuffer {
    inner: Box<[u8]>,
    id: u16,
}

pub(crate) struct ProvideBuffer {
    buffer: Box<[u8]>,
    group: u16,
    id: u16,
}

impl ProvideBuffer {
    pub fn new(buffer: Box<[u8]>, id: u16, group: u16) -> Self {
        Self { buffer, group, id }
    }
}

unsafe impl Op for ProvideBuffer {
    type Output = Result<Box<[u8]>>;

    fn entry(&mut self) -> Entry {
        opcode::ProvideBuffers::new(
            self.buffer.as_mut_ptr(),
            self.buffer.len() as i32,
            1,
            self.group,
            self.id,
        )
        .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_zero(ret).map(|_| self.buffer)
    }

    fn cancel(self) -> Cancellation {
        self.buffer.into()
    }
}
