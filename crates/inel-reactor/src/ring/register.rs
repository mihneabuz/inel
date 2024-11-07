use crate::buffer::StableMutBuffer;

pub struct BufferRegister {}
pub struct BufferKey {}

impl BufferRegister {
    pub fn new() -> Self {
        Self {}
    }

    pub fn insert<B>(&mut self, _buffer: &mut B) -> BufferKey
    where
        B: StableMutBuffer,
    {
        todo!();
    }

    pub fn remove<B>(&mut self, _buffer: &mut B)
    where
        B: StableMutBuffer,
    {
        todo!();
    }

    pub fn iovecs(&self) -> Vec<libc::iovec> {
        todo!()
    }
}

impl BufferKey {
    pub fn index(&self) -> u16 {
        todo!()
    }
}
