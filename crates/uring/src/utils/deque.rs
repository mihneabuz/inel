use core::{
    alloc::Layout,
    cmp::max,
    marker::PhantomData,
    mem::{align_of, needs_drop, size_of},
    ptr::NonNull,
};

use allocator_api2::alloc::{Allocator, Global};

#[repr(C)]
struct Header {
    len: u32,
    capacity: u32,
    head: u32,
}

static EMPTY_HEADER: Header = Header {
    len: 0,
    capacity: 0,
    head: 0,
};

const INITIAL_CAPACITY: u32 = 16;
const MAX_CAPACITY: u32 = 1 << 31;

pub struct Deque<T, A: Allocator = Global> {
    ptr: NonNull<Header>,
    alloc: A,
    _pd: PhantomData<T>,
}

impl<T> Deque<T, Global> {
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<T, A: Allocator> Deque<T, A> {
    pub const fn new_in(alloc: A) -> Self {
        Self {
            ptr: unsafe { NonNull::new_unchecked(&EMPTY_HEADER as *const _ as *mut _) },
            alloc,
            _pd: PhantomData,
        }
    }

    pub const fn len(&self) -> usize {
        self.header().len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn capacity(&self) -> usize {
        self.header().capacity as usize
    }

    #[inline]
    pub fn push(&mut self, value: T) {
        let header = self.header();
        if header.len == header.capacity {
            self.grow();
        }
        unsafe {
            let data = self.data_ptr();
            let header = self.header_mut();
            let mask = header.capacity - 1;
            let idx = (header.head + header.len) & mask;
            data.add(idx as usize).write(value);
            header.len += 1;
        }
    }

    #[inline]
    pub const fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }
        unsafe {
            let data = self.data_ptr();
            let header = self.header_mut();
            let idx = header.head;
            let value = data.add(idx as usize).read();
            header.head = (header.head + 1) & (header.capacity - 1);
            header.len -= 1;
            Some(value)
        }
    }

    const fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    /// # Safety
    /// The deque must not be the empty singleton. Callers must ensure an
    /// allocation has been made (capacity > 0) before invoking this.
    const unsafe fn header_mut(&mut self) -> &mut Header {
        unsafe { self.ptr.as_mut() }
    }

    const fn data_ptr(&self) -> NonNull<T> {
        if self.capacity() == 0 {
            NonNull::<T>::dangling()
        } else {
            unsafe { self.ptr.cast::<u8>().add(data_offset::<T>()).cast() }
        }
    }

    #[cold]
    fn grow(&mut self) {
        let (old_cap, len, old_head) = {
            let h = self.header();
            (h.capacity, h.len, h.head)
        };

        let new_cap = if old_cap == 0 {
            INITIAL_CAPACITY
        } else {
            old_cap
                .checked_mul(2)
                .filter(|&c| c <= MAX_CAPACITY)
                .expect("deque capacity overflow")
        };

        unsafe {
            let new_layout = layout::<T>(new_cap as usize);
            let raw = self
                .alloc
                .allocate(new_layout)
                .expect("failed to allocate memory");
            let new_header: NonNull<Header> = raw.cast();
            let new_data: NonNull<T> = new_header.cast::<u8>().add(data_offset::<T>()).cast();

            if len > 0 {
                let src = self.data_ptr();
                let head = old_head as usize;
                let first = (old_cap as usize - head).min(len as usize);
                new_data.copy_from_nonoverlapping(src.add(head), first);
                if first < len as usize {
                    new_data
                        .add(first)
                        .copy_from_nonoverlapping(src, len as usize - first);
                }
            }

            new_header.write(Header {
                len,
                capacity: new_cap,
                head: 0,
            });

            if old_cap > 0 {
                let old_layout = layout::<T>(old_cap as usize);
                self.alloc.deallocate(self.ptr.cast(), old_layout);
            }

            self.ptr = new_header;
        }
    }
}

impl<T, A: Allocator> Drop for Deque<T, A> {
    fn drop(&mut self) {
        let capacity = self.header().capacity;
        if capacity == 0 {
            return;
        }
        unsafe {
            if needs_drop::<T>() {
                let len = self.header().len as usize;
                let head = self.header().head as usize;
                let mask = capacity as usize - 1;
                let data = self.data_ptr();
                for i in 0..len {
                    data.add((head + i) & mask).drop_in_place();
                }
            }
            self.alloc
                .deallocate(self.ptr.cast(), layout::<T>(capacity as usize));
        }
    }
}

fn alloc_align<T>() -> usize {
    max(align_of::<T>(), align_of::<Header>())
}

const fn data_offset<T>() -> usize {
    let header_size = size_of::<Header>();
    let align = align_of::<T>();
    (header_size + align - 1) & !(align - 1)
}

fn layout<T>(cap: usize) -> Layout {
    let size = data_offset::<T>()
        .checked_add(
            size_of::<T>()
                .checked_mul(cap)
                .expect("deque size overflow"),
        )
        .expect("deque size overflow");
    Layout::from_size_align(size, alloc_align::<T>()).expect("deque layout overflow")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[test]
    fn new_is_empty_and_unallocated() {
        let d: Deque<u32> = Deque::new();
        assert_eq!(d.len(), 0);
        assert_eq!(d.capacity(), 0);
        assert!(d.is_empty());
    }

    #[test]
    fn pop_empty_returns_none() {
        let mut d: Deque<u32> = Deque::new();
        assert!(d.pop().is_none());
    }

    #[test]
    fn push_pop_single() {
        let mut d = Deque::new();
        d.push(42u32);
        assert_eq!(d.len(), 1);
        assert_eq!(d.pop(), Some(42));
        assert!(d.is_empty());
    }

    #[test]
    fn fifo_order() {
        let mut d = Deque::new();
        for i in 0..8u32 {
            d.push(i);
        }
        for i in 0..8u32 {
            assert_eq!(d.pop(), Some(i));
        }
        assert!(d.pop().is_none());
    }

    #[test]
    fn grows_past_initial_capacity() {
        let mut d = Deque::new();
        for i in 0..100u32 {
            d.push(i);
        }
        assert_eq!(d.len(), 100);
        assert!(d.capacity() >= 100);
        for i in 0..100u32 {
            assert_eq!(d.pop(), Some(i));
        }
    }

    #[test]
    fn grow_preserves_wrapped_layout() {
        let mut d = Deque::new();
        for i in 0..INITIAL_CAPACITY {
            d.push(i);
        }
        assert_eq!(d.pop(), Some(0));
        assert_eq!(d.pop(), Some(1));
        d.push(30);
        d.push(31);
        d.push(32);
        assert_eq!(d.pop(), Some(2));
        assert_eq!(d.pop(), Some(3));
        assert_eq!(d.pop(), Some(4));
        assert_eq!(d.pop(), Some(5));
        assert_eq!(d.pop(), Some(6));
    }

    #[test]
    fn interleaved_push_pop_wraps() {
        let mut d = Deque::new();
        for i in 0..1000u32 {
            d.push(i);
            if i >= 3 {
                assert_eq!(d.pop(), Some(i - 3));
            }
        }
        for i in 997..1000u32 {
            assert_eq!(d.pop(), Some(i));
        }
        assert!(d.is_empty());
    }

    #[test]
    fn drop_runs_on_pop() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

        struct Tracked;
        impl Drop for Tracked {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);
        let mut d = Deque::new();
        d.push(Tracked);
        d.push(Tracked);
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 0);
        drop(d.pop());
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1);
        drop(d.pop());
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn drop_runs_on_deque_drop() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

        struct Tracked;
        impl Drop for Tracked {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);
        {
            let mut d = Deque::new();
            for _ in 0..10 {
                d.push(Tracked);
            }
            for _ in 0..3 {
                d.pop();
            }
        }
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 10);

        DROP_COUNT.store(0, Ordering::Relaxed);
        {
            let mut d = Deque::new();
            for _ in 0..4 {
                d.push(Tracked);
            }
            d.pop();
            d.pop();
            d.push(Tracked);
            d.push(Tracked);
        }
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn high_alignment_type() {
        #[repr(align(64))]
        #[derive(Debug, PartialEq, Eq)]
        struct Aligned(u64);

        let mut d = Deque::new();
        for i in 0..20u64 {
            d.push(Aligned(i));
        }
        for i in 0..20u64 {
            let v = d.pop().unwrap();
            assert_eq!(v, Aligned(i));
            assert_eq!((&v as *const _ as usize) % 64, 0);
        }
    }

    #[test]
    fn zero_sized_type_pointer_is_aligned_for_data() {
        // Sanity: data_offset for align-1 types is the header size exactly.
        assert_eq!(data_offset::<u8>(), size_of::<Header>());
        // For align-8 types, offset rounds up.
        assert_eq!(data_offset::<u64>(), 16);
        // For align-64 types.
        #[repr(align(64))]
        struct A;
        assert_eq!(data_offset::<A>(), 64);
    }
}
