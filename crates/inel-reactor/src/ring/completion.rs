use std::{collections::VecDeque, mem, task::Waker};

use slab::Slab;

use crate::{ring::RingResult, Cancellation};

#[derive(Debug)]
enum Results {
    Vacant,
    Single(RingResult),
    Multiple((VecDeque<RingResult>, bool)),
}

impl Results {
    fn push(&mut self, result: RingResult) {
        match self {
            Results::Vacant => {
                *self = if result.has_more() {
                    Results::Multiple((VecDeque::from([result]), true))
                } else {
                    Results::Single(result)
                };
            }

            Results::Multiple((queue, has_more)) => {
                *has_more = result.has_more();
                queue.push_back(result);
            }

            Results::Single(_) => unreachable!("Already got result"),
        }
    }

    fn pop(&mut self) -> Option<RingResult> {
        match self {
            Results::Vacant => None,
            Results::Single(result) => Some(*result),
            Results::Multiple((queue, _)) => queue.pop_front(),
        }
    }

    fn has_more(&self) -> bool {
        match self {
            Results::Vacant => true,
            Results::Single(_) => false,
            Results::Multiple((queue, has_more)) => !queue.is_empty() || *has_more,
        }
    }
}

enum Completion {
    Active((Waker, Results)),
    Cancelled(Cancellation),
    Finished,
}

impl Completion {
    pub fn new(waker: Waker) -> Self {
        Completion::Active((waker, Results::Vacant))
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Completion::Finished)
    }

    fn take_cancel(&mut self) -> Cancellation {
        if let Completion::Cancelled(cancel) = mem::replace(self, Completion::Finished) {
            cancel
        } else {
            panic!("Cannot take cancel");
        }
    }

    pub fn try_cancel(&mut self, cancel: Cancellation) -> bool {
        match self {
            Completion::Active((_, results)) => {
                if results.has_more() {
                    *self = Completion::Cancelled(cancel);
                    true
                } else {
                    *self = Completion::Finished;
                    cancel.drop_raw();
                    false
                }
            }

            Completion::Cancelled(_) | Completion::Finished => {
                unreachable!("Completion already cancelled");
            }
        }
    }

    pub fn try_notify(&mut self, result: RingResult) {
        match self {
            Completion::Active((waker, results)) => {
                waker.wake_by_ref();
                results.push(result);
            }

            Completion::Cancelled(_) => {
                self.take_cancel().drop_raw();
            }

            Completion::Finished => {
                unreachable!("Completion already finished");
            }
        }
    }

    pub fn take_result(&mut self) -> Option<RingResult> {
        match self {
            Completion::Active((_, results)) => {
                let res = results.pop();

                if res.is_some_and(|res| !res.has_more()) {
                    *self = Completion::Finished;
                }

                res
            }

            _ => None,
        }
    }
}

pub struct CompletionSet {
    slab: Slab<Completion>,
}

impl CompletionSet {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slab: Slab::with_capacity(capacity),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slab.is_empty()
    }

    pub fn insert(&mut self, waker: Waker) -> Key {
        Key(self.slab.insert(Completion::new(waker)))
    }

    fn with_completion<T, F>(&mut self, key: Key, fun: F) -> T
    where
        F: FnOnce(&mut Completion) -> T,
    {
        let completion = self.slab.get_mut(key.as_usize()).expect("unexpected key");
        let value = fun(completion);
        if completion.is_finished() {
            self.slab.remove(key.as_usize());
        }
        value
    }

    pub fn cancel(&mut self, key: Key, cancel: Cancellation) -> bool {
        tracing::debug!(?key, "Cancel");
        self.with_completion(key, move |comp| comp.try_cancel(cancel))
    }

    pub fn notify(&mut self, key: Key, result: RingResult) {
        tracing::debug!(?key, "Notify");
        self.with_completion(key, move |comp| comp.try_notify(result));
    }

    pub fn result(&mut self, key: Key) -> Option<RingResult> {
        tracing::debug!(?key, "Result");
        self.with_completion(key, move |comp| comp.take_result())
    }
}

/// Idenitifies an operation submitted to the [Ring](crate::Ring)
#[derive(Clone, Copy, Debug)]
pub struct Key(usize);

impl Key {
    fn as_usize(&self) -> usize {
        self.0
    }

    pub(crate) fn from_u64(value: u64) -> Self {
        Self(value as usize)
    }

    pub(crate) fn as_u64(&self) -> u64 {
        self.0 as u64
    }
}
