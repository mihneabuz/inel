use crate::buffer::StableBuffer;

pub struct BufferRegister {
    buffers: Vec<libc::iovec>,
    vacant: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct BufferKey(u16);

impl BufferRegister {
    pub fn new() -> Self {
        Self {
            buffers: Vec::with_capacity(16),
            vacant: Vec::new(),
        }
    }

    pub fn insert<B>(&mut self, buffer: &mut B) -> BufferKey
    where
        B: StableBuffer,
    {
        let index = match self.vacant.pop() {
            Some(spot) => {
                self.buffers[spot].iov_base = buffer.stable_mut_ptr() as *mut _;
                self.buffers[spot].iov_len = buffer.capacity();

                spot
            }
            None => {
                let index = self.buffers.len();

                self.buffers.push(libc::iovec {
                    iov_base: buffer.stable_mut_ptr() as *mut _,
                    iov_len: buffer.capacity(),
                });

                index
            }
        };

        BufferKey::from_usize(index)
    }

    pub fn remove<B>(&mut self, buffer: &mut B, key: BufferKey)
    where
        B: StableBuffer,
    {
        let index = key.index() as usize;

        if self.buffers[index].iov_base == buffer.stable_mut_ptr() as *mut _ {
            self.vacant.push(index);
        }
    }

    pub fn iovecs(&self) -> &[libc::iovec] {
        self.buffers.as_slice()
    }
}

impl BufferKey {
    fn from_usize(key: usize) -> Self {
        Self(key as u16)
    }

    pub fn index(&self) -> u16 {
        self.0
    }
}
