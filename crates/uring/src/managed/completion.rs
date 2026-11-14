use core::task::Waker;

use rustix::io_uring::*;

use crate::{
    managed::{cancellation::Cancellation, group::BufGroupId, group::BufId},
    sys::{Cqe, Sqe},
    utils::{Deque, UnsafeSlab},
};

/// Identifies a submitted op. It is the op's `user_data`, so it comes back
/// with every CQE and cancellation can match it exactly.
///
/// Packed so it has no padding: 6 bytes instead of 8, which keeps the
/// `SubmissionState` holding it at 7 bytes instead of 12.
#[repr(Rust, packed)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    /// The op's slot in `Completions`.
    slot: u32,
    /// The buffer group the op picks a buffer from, if any.
    gid: Option<BufGroupId>,
}

impl Key {
    /// The key of ops whose completions are ignored. Never a valid key, since
    /// slots never reach `u32::MAX`.
    pub const IGNORE: Self = Self::new(u32::MAX);

    const fn new(slot: u32) -> Self {
        Self { slot, gid: None }
    }

    /// Records the buffer group a prepared SQE selects from, so completions
    /// and cancellation know which group a returned buffer belongs to.
    pub fn add_group_id(&mut self, sqe: &Sqe) -> &mut Self {
        self.gid = sqe.buf_group().and_then(BufGroupId::from_raw);
        self
    }

    /// The buffer group the op picks a buffer from, if any.
    pub const fn group_id(&self) -> Option<BufGroupId> {
        self.gid
    }

    /// Encodes the key as `user_data`: slot in bits 0..32, group in 32..48
    /// (0 for none).
    pub const fn as_raw(&self) -> u64 {
        let gid = match self.gid {
            Some(gid) => gid.as_raw(),
            None => 0,
        };
        self.slot as u64 | (gid as u64) << 32
    }

    /// Decodes a CQE's `user_data`. Inverse of [`Self::as_raw`].
    pub const fn from_raw(raw: u64) -> Self {
        Self {
            slot: raw as u32,
            gid: BufGroupId::from_raw((raw >> 32) as u16),
        }
    }
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

    pub fn buffer_id(&self) -> Option<BufId> {
        self.flags
            .contains(IoringCqeFlags::BUFFER)
            .then_some((self.flags.bits() >> IORING_CQE_BUFFER_SHIFT) as u16)
            .map(BufId::from_raw)
    }

    #[cfg(test)]
    pub const fn raw(res: i32, flags: IoringCqeFlags) -> Self {
        Self { res, flags }
    }
}

pub struct CompletionEntry {
    key: Key,
    result: CompletionResult,
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

    pub fn insert_single(&mut self, waker: Waker) -> Key {
        Key::new(self.slab.insert(CompletionHandler::new_single(waker)))
    }

    pub fn insert_multi(&mut self, waker: Waker) -> Key {
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

    pub fn update(&mut self, key: Key, waker: &Waker) {
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

    /// Replaces the stored waker, unless it already wakes the same task.
    fn update_waker(&mut self, new_waker: &Waker) {
        match self {
            Self::Pending { waker } | Self::Multi { waker, .. } => {
                if !waker.will_wake(new_waker) {
                    *waker = new_waker.clone();
                }
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
