use core::{
    alloc::Layout,
    mem::{self, ManuallyDrop},
    ptr::NonNull,
};
use std::alloc::handle_alloc_error;

use allocator_api2::{
    alloc::{Allocator, Global},
    vec::Vec,
};

union UnsafeEntry<T> {
    value: ManuallyDrop<T>,
    next_free: u32,
}

pub struct UnsafeSlab<T, A: Allocator = Global> {
    ptr: NonNull<UnsafeEntry<T>>,
    len: u32,
    capacity: u32,
    next_free: u32,
    alloc: A,
}

impl<T> UnsafeSlab<T, Global> {
    pub fn with_capacity(capacity: u32) -> Self {
        Self::with_capacity_in(capacity, Global)
    }
}

impl<T, A: Allocator> UnsafeSlab<T, A> {
    pub fn with_capacity_in(capacity: u32, alloc: A) -> Self {
        let layout = Self::layout(capacity);
        let ptr = alloc
            .allocate(layout)
            .unwrap_or_else(|_| handle_alloc_error(layout));

        let mut slab = Self {
            ptr: ptr.cast(),
            len: 0,
            capacity,
            next_free: 0,
            alloc,
        };

        for i in 0..capacity {
            unsafe {
                *slab.entry(i) = UnsafeEntry { next_free: i + 1 };
            }
        }

        slab
    }

    pub const fn len(&self) -> u32 {
        self.len
    }

    fn layout(capacity: u32) -> Layout {
        Layout::array::<UnsafeEntry<T>>(capacity as usize).expect("slab too large")
    }

    /// # Safety
    /// `key` must refer to an occupied slot returned by a prior `insert`
    /// that has not been `remove`d.
    const unsafe fn entry(&mut self, key: u32) -> &mut UnsafeEntry<T> {
        unsafe { self.ptr.add(key as usize).as_mut() }
    }

    pub fn insert(&mut self, value: T) -> u32 {
        if self.next_free == self.capacity {
            self.grow();
        }

        let key = self.next_free;

        unsafe {
            let entry = self.entry(key);
            let next_free = entry.next_free;
            *entry = UnsafeEntry {
                value: ManuallyDrop::new(value),
            };
            self.next_free = next_free;
            self.len += 1;
        }
        key
    }

    /// Doubles the capacity and links the new slots into the free list, which
    /// is empty when this is called. Keys are indices, so they stay valid.
    #[cold]
    fn grow(&mut self) {
        assert!(self.capacity < u32::MAX, "slab is full");

        let old_capacity = self.capacity;
        let new_capacity = old_capacity.saturating_mul(2).max(4);
        let (old, new) = (Self::layout(old_capacity), Self::layout(new_capacity));

        // SAFETY: allocated by `alloc` with `old`, and `new` is not smaller
        let ptr = unsafe { self.alloc.grow(self.ptr.cast(), old, new) }
            .unwrap_or_else(|_| handle_alloc_error(new));
        self.ptr = ptr.cast();
        self.capacity = new_capacity;

        for i in old_capacity..new_capacity {
            // SAFETY: in bounds of the new allocation. `i + 1` can't overflow
            // since `i < new_capacity <= u32::MAX`
            unsafe { *self.entry(i) = UnsafeEntry { next_free: i + 1 } };
        }
        self.next_free = old_capacity;
    }

    /// # Safety
    /// `key` must refer to an occupied slot returned by a prior `insert`
    /// that has not been `remove`d.
    #[inline]
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
    #[inline]
    pub unsafe fn remove(&mut self, key: u32) {
        let next_free = self.next_free;
        unsafe {
            let entry = self.entry(key);
            ManuallyDrop::drop(&mut entry.value);
            *entry = UnsafeEntry { next_free };
        }
        self.next_free = key;
        self.len -= 1;
    }
}

impl<T, A: Allocator> Drop for UnsafeSlab<T, A> {
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

        unsafe {
            self.alloc
                .deallocate(self.ptr.cast(), Self::layout(self.capacity))
        };
    }
}

enum Entry<T> {
    Occupied(T),
    Vacant(u32),
}

pub struct Slab<T, A: Allocator = Global> {
    slots: Vec<Entry<T>, A>,
    free: u32,
}

impl<T> Slab<T, Global> {
    pub const fn new() -> Self {
        Self::new_in(Global)
    }
}

impl<T, A: Allocator> Slab<T, A> {
    pub const fn new_in(alloc: A) -> Self {
        Self {
            slots: Vec::new_in(alloc),
            free: 0,
        }
    }

    pub const fn vacant_entry(&mut self) -> VacantEntry<'_, T, A> {
        VacantEntry {
            key: self.free,
            slab: self,
        }
    }

    pub fn remove(&mut self, key: u32) -> T {
        match core::mem::replace(&mut self.slots[key as usize], Entry::Vacant(self.free)) {
            Entry::Occupied(value) => {
                self.free = key;
                value
            }
            Entry::Vacant(_) => panic!("tried to access vacant entry"),
        }
    }

    pub fn get_mut(&mut self, key: u32) -> Option<&mut T> {
        match &mut self.slots[key as usize] {
            Entry::Occupied(value) => Some(value),
            Entry::Vacant(_) => None,
        }
    }

    pub fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
        self.free = 0;
        self.slots.drain(..).filter_map(|entry| match entry {
            Entry::Occupied(value) => Some(value),
            Entry::Vacant(_) => None,
        })
    }
}

pub struct VacantEntry<'a, T, A: Allocator = Global> {
    slab: &'a mut Slab<T, A>,
    key: u32,
}

impl<'a, T, A: Allocator> VacantEntry<'a, T, A> {
    pub const fn key(&self) -> u32 {
        self.key
    }

    pub fn insert(self, value: T) -> &'a mut T {
        let Self { slab, key } = self;

        if key as usize == slab.slots.len() {
            slab.slots.push(Entry::Occupied(value));
            slab.free = key + 1;
        } else {
            slab.free = match slab.slots[key as usize] {
                Entry::Vacant(next) => next,
                Entry::Occupied(_) => unreachable!(),
            };
            slab.slots[key as usize] = Entry::Occupied(value);
        }

        match &mut slab.slots[key as usize] {
            Entry::Occupied(value) => value,
            Entry::Vacant(_) => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[test]
    fn insert_and_get() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(4);

        let k0 = slab.insert(10);
        let k1 = slab.insert(20);
        let k2 = slab.insert(30);

        assert_eq!(unsafe { *slab.get_mut(k0) }, 10);
        assert_eq!(unsafe { *slab.get_mut(k1) }, 20);
        assert_eq!(unsafe { *slab.get_mut(k2) }, 30);
    }

    #[test]
    fn sequential_keys() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(4);

        assert_eq!(slab.insert(0), 0);
        assert_eq!(slab.insert(0), 1);
        assert_eq!(slab.insert(0), 2);
        assert_eq!(slab.insert(0), 3);
    }

    #[test]
    fn remove_and_reuse() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(4);

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
        let mut slab = UnsafeSlab::<u64>::with_capacity(4);

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
    fn grows_when_full() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(2);

        let keys: Vec<u32> = (0..100).map(|i| slab.insert(i)).collect();
        assert_eq!(keys, (0..100).collect::<Vec<_>>());

        for (i, &k) in keys.iter().enumerate() {
            assert_eq!(unsafe { *slab.get_mut(k) }, i as u64);
        }
    }

    #[test]
    fn grows_from_empty() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(0);

        let k = slab.insert(7);
        assert_eq!(unsafe { *slab.get_mut(k) }, 7);
    }

    #[test]
    fn reuses_slots_after_growing() {
        let mut slab = UnsafeSlab::<u64>::with_capacity(2);

        let keys: Vec<u32> = (0..5).map(|i| slab.insert(i)).collect();
        unsafe { slab.remove(keys[1]) };
        unsafe { slab.remove(keys[4]) };

        // freed slots come back before the slab grows again
        assert_eq!(slab.insert(40), keys[4]);
        assert_eq!(slab.insert(10), keys[1]);
        assert_eq!(unsafe { *slab.get_mut(keys[0]) }, 0);
        assert_eq!(unsafe { *slab.get_mut(keys[3]) }, 3);
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

        let mut slab = UnsafeSlab::<Tracked>::with_capacity(4);

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
            // grows twice, so the free list spans several growths
            let mut slab = UnsafeSlab::<Tracked>::with_capacity(2);

            for i in 0..5 {
                slab.insert(Tracked(i));
            }

            // Remove one, leave four occupied
            unsafe { slab.remove(1) };
            assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1);
        }

        // Slab dropped — remaining 4 entries should have been dropped
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 5);
    }
}
