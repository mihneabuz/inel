use core::task::Waker;

use rustix::io_uring::*;

use crate::{
    managed::{cancellation::Cancellation, uring::BufGroupId},
    sys::{Cqe, Sqe},
    utils::{Deque, UnsafeSlab},
};

/// Identifies a submitted op. It is the op's `user_data`, so it comes back
/// with every CQE and cancellation can match it exactly.
#[repr(packed)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    slot: u32,
    group: Option<BufGroupId>,
}

impl Key {
    /// The key of ops whose completions are ignored. Never a valid key, since
    /// slots never reach `u32::MAX`.
    pub const IGNORE: Self = Self::new(u32::MAX);

    const fn new(slot: u32) -> Self {
        Self { slot, group: None }
    }

    pub fn add_group_id(&mut self, sqe: &Sqe) -> &mut Self {
        self.group = sqe.buf_group().and_then(BufGroupId::new);
        self
    }

    pub const fn group_id(&self) -> Option<BufGroupId> {
        self.group
    }

    /// Layout: slot in bits 0..32, group in 32..48 (0 for none).
    pub const fn as_raw(&self) -> u64 {
        let group = match self.group {
            Some(group) => group.raw(),
            None => 0,
        };
        self.slot as u64 | (group as u64) << 32
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self {
            slot: raw as u32,
            group: BufGroupId::new((raw >> 32) as u16),
        }
    }
}

pub struct CompletionEntry {
    key: Key,
    result: CompletionResult,
}

#[derive(Clone, Copy)]
pub struct CompletionResult {
    res: i32,
    flags: IoringCqeFlags,
}

impl CompletionResult {
    pub const fn result(&self) -> i32 {
        self.res
    }

    pub const fn has_more(&self) -> bool {
        self.flags.contains(IoringCqeFlags::MORE)
    }

    pub fn buffer_id(&self) -> Option<u16> {
        self.flags
            .contains(IoringCqeFlags::BUFFER)
            .then_some((self.flags.bits() >> IORING_CQE_BUFFER_SHIFT) as u16)
    }

    #[cfg(test)]
    pub fn raw(res: i32, flags: IoringCqeFlags) -> Self {
        Self { res, flags }
    }
}

impl CompletionEntry {
    pub const fn from_cqe(cqe: &Cqe) -> Self {
        Self {
            key: Key::from_raw(cqe.user_data()),
            result: CompletionResult {
                res: cqe.result(),
                flags: cqe.flags(),
            },
        }
    }

    pub const fn key(&self) -> Key {
        self.key
    }

    pub fn should_ignore(&self) -> bool {
        self.key == Key::IGNORE
    }

    pub const fn result(&self) -> CompletionResult {
        self.result
    }
}

pub struct Completions {
    slab: UnsafeSlab<CompletionHandler>,
}

impl Completions {
    pub fn new(capacity: u32) -> Self {
        Self {
            slab: UnsafeSlab::new(capacity),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.slab.len() == 0
    }

    pub const fn insert_single(&mut self, waker: Waker) -> Key {
        Key::new(self.slab.insert(CompletionHandler::new_single(waker)))
    }

    pub const fn insert_multi(&mut self, waker: Waker) -> Key {
        Key::new(self.slab.insert(CompletionHandler::new_multi(waker)))
    }

    fn with_completion<T, F>(&mut self, key: Key, fun: F) -> T
    where
        F: FnOnce(&mut CompletionHandler) -> T,
    {
        let completion = unsafe { self.slab.get_mut(key.slot) };
        let value = fun(completion);
        if completion.is_finished() {
            unsafe { self.slab.remove(key.slot) };
        }
        value
    }

    /// Returns whether the result was stored for the op.
    pub fn notify(&mut self, key: Key, result: CompletionResult) -> bool {
        self.with_completion(key, move |comp| comp.try_notify(result))
    }

    /// Results that were posted but never read are passed to `unread`.
    pub fn cancel<F>(&mut self, key: Key, handle: Cancellation, unread: F) -> bool
    where
        F: FnMut(CompletionResult),
    {
        self.with_completion(key, move |comp| comp.try_cancel(handle, unread))
    }

    pub fn update(&mut self, key: Key, waker: Waker) {
        self.with_completion(key, move |comp| comp.update_waker(waker));
    }

    pub fn result(&mut self, key: Key) -> Option<CompletionResult> {
        self.with_completion(key, move |comp| comp.take_result())
    }
}

enum CompletionHandler {
    Pending {
        waker: Waker,
    },
    Single {
        result: CompletionResult,
    },
    Multi {
        waker: Waker,
        queue: Deque<CompletionResult>,
    },
    Cancelled {
        handle: Cancellation,
    },
    Finished,
}

impl CompletionHandler {
    const fn new_single(waker: Waker) -> Self {
        Self::Pending { waker }
    }

    const fn new_multi(waker: Waker) -> Self {
        Self::Multi {
            waker,
            queue: Deque::new(),
        }
    }

    fn update_waker(&mut self, new_waker: Waker) {
        match self {
            Self::Pending { waker } => {
                *waker = new_waker;
            }
            Self::Multi { waker, .. } => {
                *waker = new_waker;
            }
            _ => unreachable!("Cannot update waker in this state"),
        }
    }

    fn try_notify(&mut self, result: CompletionResult) -> bool {
        match self {
            Self::Pending { waker } => {
                waker.wake_by_ref();
                *self = Self::Single { result };
                true
            }
            Self::Multi { waker, queue } => {
                waker.wake_by_ref();
                queue.push(result);
                true
            }
            Self::Cancelled { handle } => {
                // a multishot op can keep posting until its final CQE
                if !result.has_more() {
                    unsafe { handle.drop_by_ref() };
                    *self = Self::Finished;
                }
                false
            }
            _ => unreachable!("Cannot post result in this state"),
        }
    }

    fn try_cancel<F>(&mut self, handle: Cancellation, mut unread: F) -> bool
    where
        F: FnMut(CompletionResult),
    {
        match self {
            Self::Pending { .. } => {
                *self = Self::Cancelled { handle };
                true
            }
            Self::Single { result } => {
                unread(*result);
                handle.drop_raw();
                *self = Self::Finished;
                false
            }
            Self::Multi { queue, .. } => {
                let mut done = false;

                while let Some(result) = queue.pop() {
                    done |= !result.has_more();
                    unread(result);
                }

                if done {
                    handle.drop_raw();
                    *self = Self::Finished;
                    false
                } else {
                    *self = Self::Cancelled { handle };
                    true
                }
            }
            _ => unreachable!("Cannot cancel in this state"),
        }
    }

    fn take_result(&mut self) -> Option<CompletionResult> {
        match self {
            Self::Pending { .. } => None,
            Self::Single { result } => {
                let res = Some(*result);
                *self = Self::Finished;
                res
            }
            Self::Multi { queue, .. } => {
                let res = queue.pop();
                if res.is_some_and(|res| !res.has_more()) {
                    *self = Self::Finished;
                }
                res
            }
            _ => unreachable!("Cannot get result from this state"),
        }
    }

    const fn is_finished(&self) -> bool {
        matches!(self, Self::Finished)
    }
}
