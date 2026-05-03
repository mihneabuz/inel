use core::task::Waker;

use rustix::io_uring::*;

use crate::{
    managed::{buf_rings::BufGroupId, cancellation::Cancellation},
    sys::{Cqe, Sqe},
    utils::{Deque, UnsafeSlab},
};

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Key(pub(crate) u32);

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct UserData(u64);

impl UserData {
    pub const IGNORE: u64 = u64::MAX;

    pub const fn build(sqe: &Sqe, key: Key) -> Self {
        Self(key.0 as u64 | (sqe.get_buf_index() as u64) << 32)
    }

    pub const fn as_raw(&self) -> u64 {
        self.0
    }
}

pub struct CompletionEntry {
    user_data: UserData,
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
            user_data: UserData(cqe.user_data()),
            result: CompletionResult {
                res: cqe.result(),
                flags: cqe.flags(),
            },
        }
    }

    pub const fn key(&self) -> Key {
        Key(self.user_data.0 as u32)
    }

    pub const fn bgid(&self) -> BufGroupId {
        BufGroupId((self.user_data.0 >> 32) as u16)
    }

    pub const fn should_ignore(&self) -> bool {
        self.user_data.0 == UserData::IGNORE
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
        Key(self.slab.insert(CompletionHandler::new_single(waker)))
    }

    pub const fn insert_multi(&mut self, waker: Waker) -> Key {
        Key(self.slab.insert(CompletionHandler::new_multi(waker)))
    }

    fn with_completion<T, F>(&mut self, key: Key, fun: F) -> T
    where
        F: FnOnce(&mut CompletionHandler) -> T,
    {
        let completion = unsafe { self.slab.get_mut(key.0) };
        let value = fun(completion);
        if completion.is_finished() {
            unsafe { self.slab.remove(key.0) };
        }
        value
    }

    pub fn notify(&mut self, key: Key, result: CompletionResult) -> bool {
        self.with_completion(key, move |comp| comp.try_notify(result))
    }

    pub fn cancel(&mut self, key: Key, handle: Cancellation) -> bool {
        self.with_completion(key, move |comp| comp.try_cancel(handle))
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
                unsafe { handle.drop_by_ref() };
                *self = Self::Finished;
                false
            }
            _ => unreachable!("Cannot post result in this state"),
        }
    }

    fn try_cancel(&mut self, handle: Cancellation) -> bool {
        match self {
            Self::Pending { .. } => {
                *self = Self::Cancelled { handle };
                true
            }
            Self::Single { .. } => {
                handle.drop_raw();
                *self = Self::Finished;
                false
            }
            Self::Multi { .. } => {
                *self = Self::Cancelled { handle };
                true
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
