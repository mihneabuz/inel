use std::{collections::VecDeque, task::Waker};

use slab::Slab;

use crate::{ring::RingResult, Cancellation};

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

pub struct CompletionSet {
    slab: Slab<Completion>,
    queues: ResultQueues,
}

impl CompletionSet {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            slab: Slab::with_capacity(capacity),
            queues: ResultQueues::with_capacity(4),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.slab.is_empty() && self.queues.is_empty()
    }

    pub fn insert(&mut self, waker: Waker) -> Key {
        Key(self.slab.insert(Completion::new(waker)))
    }

    fn with_completion<T, F>(&mut self, key: Key, fun: F) -> T
    where
        F: FnOnce(&mut Completion, &mut ResultQueues) -> T,
    {
        let completion = self.slab.get_mut(key.as_usize()).expect("unexpected key");
        let value = fun(completion, &mut self.queues);
        if completion.is_finished() {
            self.slab.remove(key.as_usize());
        }
        value
    }

    pub fn cancel(&mut self, key: Key, cancel: Cancellation) -> bool {
        tracing::debug!(?key, "Cancel");
        self.with_completion(key, move |comp, queues| comp.try_cancel(queues, cancel))
    }

    pub fn notify(&mut self, key: Key, result: RingResult) {
        tracing::debug!(?key, "Notify");
        self.with_completion(key, move |comp, queues| comp.try_notify(queues, result));
    }

    pub fn result(&mut self, key: Key) -> Option<RingResult> {
        tracing::debug!(?key, "Result");
        self.with_completion(key, move |comp, queues| comp.take_result(queues))
    }
}

#[derive(Clone, Copy)]
struct QueueHandle(u32);

struct ResultQueues {
    queues: Vec<VecDeque<RingResult>>,
    unused: Vec<QueueHandle>,
}

impl ResultQueues {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            queues: vec![VecDeque::new(); capacity],
            unused: (0..capacity as u32).map(QueueHandle).collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.queues.len() == self.unused.len()
    }

    fn handle(&mut self) -> QueueHandle {
        if let Some(handle) = self.unused.pop() {
            handle
        } else {
            let index = self.queues.len();
            self.queues.push(VecDeque::new());
            QueueHandle(index as u32)
        }
    }

    fn get(&mut self, handle: QueueHandle) -> &mut VecDeque<RingResult> {
        &mut self.queues[handle.0 as usize]
    }

    fn release(&mut self, handle: QueueHandle) {
        self.get(handle).clear();
        self.unused.push(handle);
    }
}

enum Completion {
    Vacant {
        waker: Waker,
    },

    Single {
        result: RingResult,
    },

    Multiple {
        waker: Waker,
        handle: QueueHandle,
        more: bool,
    },

    Cancelled {
        cancel: Cancellation,
    },

    Finished,
}

impl Completion {
    fn new(waker: Waker) -> Self {
        Completion::Vacant { waker }
    }

    fn is_finished(&self) -> bool {
        matches!(self, Completion::Finished)
    }

    fn try_cancel(&mut self, queues: &mut ResultQueues, cancel: Cancellation) -> bool {
        let already_completed = match self {
            Completion::Vacant { .. } => false,
            Completion::Single { .. } => true,
            Completion::Multiple { handle, more, .. } => {
                queues.release(*handle);
                !*more
            }
            _ => {
                unreachable!("Completion already cancelled");
            }
        };

        if already_completed {
            *self = Completion::Finished;
            cancel.drop_raw();
            false
        } else {
            *self = Completion::Cancelled { cancel };
            true
        }
    }

    fn try_notify(&mut self, queues: &mut ResultQueues, result: RingResult) {
        match std::mem::replace(self, Completion::Finished) {
            Completion::Vacant { waker } => {
                waker.wake_by_ref();

                *self = if result.has_more() {
                    let handle = queues.handle();
                    queues.get(handle).push_back(result);
                    Completion::Multiple {
                        waker,
                        handle,
                        more: true,
                    }
                } else {
                    Completion::Single { result }
                };
            }

            Completion::Multiple { waker, handle, .. } => {
                waker.wake_by_ref();

                queues.get(handle).push_back(result);
                *self = Completion::Multiple {
                    waker,
                    handle,
                    more: result.has_more(),
                }
            }

            Completion::Cancelled { cancel } => {
                cancel.drop_raw();
            }

            _ => {
                unreachable!("Completion already finished");
            }
        }
    }

    fn take_result(&mut self, queues: &mut ResultQueues) -> Option<RingResult> {
        match self {
            Completion::Single { result } => {
                let result = *result;
                *self = Completion::Finished;
                Some(result)
            }

            Completion::Multiple { handle, more, .. } => {
                let result = queues.get(*handle).pop_front();
                if queues.get(*handle).is_empty() && !*more {
                    queues.release(*handle);
                    *self = Completion::Finished;
                }
                result
            }

            _ => None,
        }
    }
}
