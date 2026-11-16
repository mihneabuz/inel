use core::task::Waker;

use rustix::io_uring::*;

use crate::{
    managed::{cancellation::Cancellation, group::BufGroupId, group::BufId},
    sys::{Cqe, Sqe},
    utils::{Deque, UnsafeSlab},
};

/// Marks a prepared SQE as detached.
///
/// Its completions will be skipped and we don't need to insert an entry for it in
/// [`Completions`].
pub const fn mark_detached(sqe: &mut Sqe) {
    sqe.user_data(Key::IGNORE);
}

/// Decodes a CQE.
///
/// Returns `None` if the source SQE was marked as detached, otherwise returns a pair
/// of [`Key`], which can be used to identify the SQE, and [`CompletionResult`] which
/// contains the actual result and flags from `cqe`.
pub const fn parse_cqe(cqe: &Cqe) -> Option<(Key, CompletionResult)> {
    if cqe.user_data() == Key::IGNORE {
        return None;
    }

    let result = CompletionResult {
        res: cqe.result(),
        flags: cqe.flags(),
    };

    Some((Key::from_raw(cqe.user_data()), result))
}

/// The results of a CQE.
///
/// It contains the return value and flags of a CQE, with some helpers for reading them.
#[derive(Clone, Copy)]
pub struct CompletionResult {
    res: i32,
    flags: IoringCqeFlags,
}

impl CompletionResult {
    /// The raw i32 result of the CQE.
    pub const fn result(&self) -> i32 {
        self.res
    }

    /// Returns true if the CQE is not the last one (for multishot ops).
    pub const fn has_more(&self) -> bool {
        self.flags.contains(IoringCqeFlags::MORE)
    }

    /// Returns the buffer id associated with this completion.
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

/// Identifies a submitted op.
///
/// It contains the key to the [`Completions`] slab and it's also written into the
/// SQE's `user_data` field, so it can be retrieved from the CQE.
///
/// This is the only way to smuggle information between the SQE and CQE, so we also
/// use it to store the buffer group id for buffer select ops.
///
/// Because we have to write it into the `user_data` field, it is required to keep it
/// at or under 8 bytes. Right now, we are also using `repr(packed)` to keep the
/// `SubmissionState` that holds it under 8 bytes. This is a nice optimization that
/// we would like to keep - since it benefits all ops.
///
/// # Safety
///
/// A `Key` must not be used anymore after a using it to cancel the op or witnessing
/// the final result CQE - since the handler it identifies will be cleaned up.
#[repr(Rust, packed)]
#[derive(PartialEq, Eq, Debug)]
pub struct Key {
    slot: u32,
    gid: Option<BufGroupId>,
}

impl Key {
    /// The `user_data` of ops whose completions are ignored (detached).
    /// Valid keys should never reach this value.
    const IGNORE: u64 = u32::MAX as u64;

    /// The buffer group the op picks a buffer from, if any.
    pub const fn group_id(&self) -> Option<BufGroupId> {
        self.gid
    }

    /// Encodes the key as `user_data`: slot in bits 0..32, group in 32..48.
    pub const fn as_raw(&self) -> u64 {
        let gid = match self.gid {
            Some(gid) => gid.as_raw(),
            None => 0,
        };

        self.slot as u64 | (gid as u64) << 32
    }

    /// Decodes a CQE's `user_data`. Inverse of [`Self::as_raw`].
    const fn from_raw(raw: u64) -> Self {
        Self {
            slot: raw as u32,
            gid: BufGroupId::from_raw((raw >> 32) as u16),
        }
    }
}

/// The slab keeping the completion states.
///
/// Contains the state machine for each SQE which is currently active in the kernel.
/// When creating a new entry, the user must know if that SQE will post exactly one or
/// multiple CQEs, and call the corresponding method, [`Self::insert_single`] or
/// [`Self::insert_multi`] respectively.
///
/// Uses an [`UnsafeSlab`] because it does not add any tracking state to the entries,
/// meaning we can keep them at 32 bytes size.
pub struct Completions {
    slab: UnsafeSlab<CompletionHandler>,
}

impl Completions {
    /// Create a new completions slab with initial `capacity`.
    pub fn with_capacity(capacity: u32) -> Self {
        Self {
            slab: UnsafeSlab::with_capacity(capacity),
        }
    }

    /// Returns true if there are no active entries in the slab.
    pub const fn is_empty(&self) -> bool {
        self.slab.len() == 0
    }

    /// Registers the single-shot op prepared in `sqe` and sets its `user_data`.
    /// Must be called after prep, so the key records the buffer group the SQE selects from.
    ///
    /// # Safety
    /// The SQE prepped in `sqe` must post a single CQE.
    pub unsafe fn insert_single(&mut self, sqe: &mut Sqe, waker: Waker) -> Key {
        self.insert(sqe, CompletionHandler::new_single(waker))
    }

    /// Registers the multi-shot op prepared in `sqe` and sets its `user_data`.
    /// Must be called after prep, so the key records the buffer group the SQE selects from.
    /// Unlike [`Self::insert_single`], this is safe since it can handle any number of CQEs.
    pub fn insert_multi(&mut self, sqe: &mut Sqe, waker: Waker) -> Key {
        self.insert(sqe, CompletionHandler::new_multi(waker))
    }

    fn insert(&mut self, sqe: &mut Sqe, handler: CompletionHandler) -> Key {
        let key = Key {
            slot: self.slab.insert(handler),
            gid: sqe.buf_group().and_then(BufGroupId::from_raw),
        };

        sqe.user_data(key.as_raw());

        key
    }

    fn with_completion<T, F>(&mut self, key: &Key, fun: F) -> T
    where
        F: FnOnce(&mut CompletionHandler) -> T,
    {
        // TODO: this is not technically safe, since it relies on the fact that key
        // is not used after the completion handler has been removed, which is not
        // guaranteed today by the unsafe boundaries. We would have to either add
        // the explitic contract somwhere or move to a safer design where keys are
        // passed by value.
        let completion = unsafe { self.slab.get_mut(key.slot) };
        let value = fun(completion);
        if completion.is_finished() {
            unsafe { self.slab.remove(key.slot) };
        }
        value
    }

    /// Push a new `result` to the entry identified by `key`.
    /// Returns whether the result was stored for the op.
    pub fn notify(&mut self, key: &Key, result: CompletionResult) -> bool {
        self.with_completion(key, move |comp| comp.try_notify(result))
    }

    /// Attempt to cancel the entry identified by `key`.
    /// Results that were posted but never read are passed to `unread`.
    /// Returns whether the initial SQE is still active in the kernel.
    pub fn cancel<F>(&mut self, key: &Key, handle: Cancellation, unread: F) -> bool
    where
        F: FnMut(CompletionResult),
    {
        self.with_completion(key, move |comp| comp.try_cancel(handle, unread))
    }

    /// Update the `waker` of the entry identified by `key`.
    pub fn update(&mut self, key: &Key, waker: &Waker) {
        self.with_completion(key, move |comp| comp.update_waker(waker));
    }

    /// Pop the next result of the entry identified by `key`.
    pub fn result(&mut self, key: &Key) -> Option<CompletionResult> {
        self.with_completion(key, move |comp| comp.take_result())
    }
}

/// The state machine of an active SQE.
///
/// The initial state is different based on how the handler was created:
/// - [`Self::new_single`], single-shot -> [`Self::Single`]
/// - [`Self::new_multi`], multi-shot -> [`Self::Multi`]
///
/// The [`Self::Single`] uses a single [`Option`] to keep its result, while the
/// [`Self::Multi`] has a queue of results to maintain a fifo order.
///
/// `(Single | Multi)` -> `[Cancelled]` -> `Finished`
///
/// Once an entry reaches the [`Self::Finished`] state, it is cleaned up by the slab.
enum CompletionHandler {
    Single {
        waker: Waker,
        slot: Option<CompletionResult>,
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
    /// Create a new handler for a single-shot op.
    const fn new_single(waker: Waker) -> Self {
        Self::Single { waker, slot: None }
    }

    /// Create a new handler for a multi-shot op.
    const fn new_multi(waker: Waker) -> Self {
        Self::Multi {
            waker,
            queue: Deque::new(),
        }
    }

    /// Push a new `result` to the handler.
    /// Returns whether the result was stored.
    fn try_notify(&mut self, result: CompletionResult) -> bool {
        match self {
            Self::Single { waker, slot } => {
                debug_assert!(!result.has_more());
                waker.wake_by_ref();
                *slot = Some(result);
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

    /// Try to move a handler to the cancelled state.
    /// Returns false if the handler was already completed by having its final
    /// result, in which case it is also moved to the finished state.
    fn try_cancel<F>(&mut self, handle: Cancellation, mut unread: F) -> bool
    where
        F: FnMut(CompletionResult),
    {
        match self {
            Self::Single { slot, .. } => {
                if let Some(result) = slot {
                    unread(*result);
                    handle.drop_raw();
                    *self = Self::Finished;
                    false
                } else {
                    *self = Self::Cancelled { handle };
                    true
                }
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

    /// Pop the next result from the handler.
    /// If it is the last one, also moves the handler to the finished state.
    fn take_result(&mut self) -> Option<CompletionResult> {
        match self {
            Self::Single { slot, .. } => slot.and_then(|result| {
                *self = Self::Finished;
                Some(result)
            }),
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

    /// Update the waker of an active handler.
    fn update_waker(&mut self, new_waker: &Waker) {
        match self {
            Self::Single { waker, .. } | Self::Multi { waker, .. } => {
                if !waker.will_wake(new_waker) {
                    *waker = new_waker.clone();
                }
            }
            _ => unreachable!("Cannot update waker in this state"),
        }
    }

    /// Returns whether the handler is in the finished state.
    const fn is_finished(&self) -> bool {
        matches!(self, Self::Finished)
    }
}
