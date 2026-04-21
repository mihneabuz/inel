use core::{
    alloc::Layout,
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr::NonNull,
};

use allocator_api2::alloc::{Allocator, Global};

union Entry<T> {
    value: ManuallyDrop<T>,
    next_free: u32,
}

pub struct Slab<T, A: Allocator = Global> {
    ptr: NonNull<Entry<T>>,
    capacity: u32,
    next_free: u32,
    alloc: A,
    _pd: PhantomData<T>,
}

impl<T> Slab<T, Global> {
    pub fn new(capacity: u32) -> Self {
        Self::new_in(capacity, Global)
    }
}

impl<T, A: Allocator> Slab<T, A> {
    pub fn new_in(capacity: u32, alloc: A) -> Self {
        let layout = Layout::array::<Entry<T>>(capacity as usize).unwrap();
        let ptr = alloc.allocate(layout).unwrap();

        let mut slab = Self {
            ptr: ptr.cast(),
            capacity,
            next_free: 0,
            alloc,
            _pd: PhantomData,
        };

        for i in 0..capacity {
            unsafe {
                *slab.entry(i) = Entry { next_free: i + 1 };
            }
        }

        slab
    }

    /// # Safety
    /// `key` must refer to an occupied slot returned by a prior `insert`
    /// that has not been `remove`d.
    const unsafe fn entry(&mut self, key: u32) -> &mut Entry<T> {
        unsafe { self.ptr.add(key as usize).as_mut() }
    }

    pub const fn insert(&mut self, value: T) -> u32 {
        let key = self.next_free;
        assert!(key < self.capacity, "slab is full");

        unsafe {
            let entry = self.entry(key);
            let next_free = entry.next_free;
            *entry = Entry {
                value: ManuallyDrop::new(value),
            };
            self.next_free = next_free;
        }
        key
    }

    /// # Safety
    /// `key` must refer to an occupied slot returned by a prior `insert`
    /// that has not been `remove`d.
    pub unsafe fn get_mut(&mut self, key: u32) -> &mut T {
        unsafe {
            let entry = self.entry(key);
            &mut entry.value
        }
    }

    /// # Safety
    /// `key` must refer to an occupied slot returned by a prior `insert`
    /// that has not been `remove`d. Must not be called twice with the same
    /// key without a new `insert` in between.
    pub unsafe fn remove(&mut self, key: u32) {
        let next_free = self.next_free;
        unsafe {
            let entry = self.entry(key);
            ManuallyDrop::drop(&mut entry.value);
            *entry = Entry { next_free };
        }
        self.next_free = key;
    }
}

impl<T, A: Allocator> Drop for Slab<T, A> {
    fn drop(&mut self) {
        if mem::needs_drop::<T>() {
            let mut is_free = vec![false; self.capacity as usize];
            let mut idx = self.next_free;
            while idx < self.capacity {
                is_free[idx as usize] = true;
                idx = unsafe { self.entry(idx).next_free }
            }

            for i in 0..self.capacity {
                if !is_free[i as usize] {
                    unsafe { ManuallyDrop::drop(&mut self.entry(i).value) };
                }
            }
        }

        let layout = Layout::array::<Entry<T>>(self.capacity as usize).unwrap();
        unsafe { self.alloc.deallocate(self.ptr.cast(), layout) };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[test]
    fn insert_and_get() {
        let mut slab = Slab::<u64>::new(4);

        let k0 = slab.insert(10);
        let k1 = slab.insert(20);
        let k2 = slab.insert(30);

        assert_eq!(unsafe { *slab.get_mut(k0) }, 10);
        assert_eq!(unsafe { *slab.get_mut(k1) }, 20);
        assert_eq!(unsafe { *slab.get_mut(k2) }, 30);
    }

    #[test]
    fn sequential_keys() {
        let mut slab = Slab::<u64>::new(4);

        assert_eq!(slab.insert(0), 0);
        assert_eq!(slab.insert(0), 1);
        assert_eq!(slab.insert(0), 2);
        assert_eq!(slab.insert(0), 3);
    }

    #[test]
    fn remove_and_reuse() {
        let mut slab = Slab::<u64>::new(4);

        let k0 = slab.insert(10);
        let k1 = slab.insert(20);

        unsafe { slab.remove(k0) };

        // Freed slot should be reused
        let k2 = slab.insert(30);
        assert_eq!(k2, k0);
        assert_eq!(unsafe { *slab.get_mut(k2) }, 30);

        // Other slot untouched
        assert_eq!(unsafe { *slab.get_mut(k1) }, 20);
    }

    #[test]
    fn fill_and_drain() {
        let mut slab = Slab::<u64>::new(4);

        let keys: Vec<u32> = (0..4).map(|i| slab.insert(i as u64)).collect();

        for &k in keys.iter().rev() {
            unsafe { slab.remove(k) };
        }

        // Should be able to fill again
        for i in 0..4u64 {
            slab.insert(i);
        }
    }

    #[test]
    #[should_panic]
    fn insert_when_full_panics() {
        let mut slab = Slab::<u64>::new(2);

        slab.insert(1);
        slab.insert(2);
        slab.insert(3); // should panic
    }

    #[test]
    fn drop_runs_on_remove() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

        #[allow(dead_code)]
        struct Tracked(u32);
        impl Drop for Tracked {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);

        let mut slab = Slab::<Tracked>::new(4);

        let k0 = slab.insert(Tracked(10));
        let k1 = slab.insert(Tracked(20));

        unsafe { slab.remove(k0) };
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1);

        unsafe { slab.remove(k1) };
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn drop_slab_drops_remaining() {
        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

        #[allow(dead_code)]
        struct Tracked(u32);
        impl Drop for Tracked {
            fn drop(&mut self) {
                DROP_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }

        DROP_COUNT.store(0, Ordering::Relaxed);

        {
            let mut slab = Slab::<Tracked>::new(4);

            slab.insert(Tracked(10));
            slab.insert(Tracked(20));
            slab.insert(Tracked(30));

            // Remove one, leave two occupied
            unsafe { slab.remove(1) };
            assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1);
        }

        // Slab dropped — remaining 2 entries should have been dropped
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 3);
    }
}
