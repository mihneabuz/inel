use crate::cancellation::Cancellation;

pub trait StableBuffer: Into<Cancellation> {
    fn stable_ptr(&self) -> *const u8;
    fn size(&self) -> usize;
}

pub trait StableMutBuffer: StableBuffer {
    fn stable_mut_ptr(&mut self) -> *mut u8;
}

impl<const N: usize> StableBuffer for Box<[u8; N]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.as_ref().len()
    }
}

impl<const N: usize> StableMutBuffer for Box<[u8; N]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for Box<[u8]> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableMutBuffer for Box<[u8]> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}

impl StableBuffer for Vec<u8> {
    fn stable_ptr(&self) -> *const u8 {
        self.as_ptr()
    }

    fn size(&self) -> usize {
        self.len()
    }
}

impl StableMutBuffer for Vec<u8> {
    fn stable_mut_ptr(&mut self) -> *mut u8 {
        self.as_mut_ptr()
    }
}
