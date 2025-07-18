use std::{collections::VecDeque, task::Waker};

use slab::Slab;

use crate::{cancellation::Cancellation, ring::RingResult};

/// Identifies an operation submitted to the [Ring](crate::Ring)
#[derive(Clone, Copy, Debug)]
pub struct Key(u32);

impl Key {
    pub(crate) fn from_usize(value: usize) -> Self {
        Self(value as u32)
    }

    pub(crate) fn as_usize(&self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_u64(value: u64) -> Self {
        Self(value as u32)
    }

    pub(crate) fn as_u64(&self) -> u64 {
        self.0 as u64
    }
}

/// Keeps track of completions
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
        Key::from_usize(self.slab.insert(Completion::new(waker)))
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

/// Stores results for entries which generate multiple completions
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

/// Represents the completion state of a single submitted entry
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
        match self {
            Completion::Vacant { .. } => {
                *self = Completion::Cancelled { cancel };
                true
            }
            Completion::Single { result } => {
                cancel.consume(*result);
                cancel.drop_raw();

                *self = Completion::Finished;
                false
            }
            Completion::Multiple { handle, more, .. } => {
                for result in queues.get(*handle) {
                    cancel.consume(*result);
                }

                queues.release(*handle);

                if *more {
                    *self = Completion::Cancelled { cancel };
                    true
                } else {
                    cancel.drop_raw();
                    *self = Completion::Finished;
                    false
                }
            }
            _ => {
                unreachable!("Completion already cancelled");
            }
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
                cancel.consume(result);
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

#[cfg(test)]
mod test {
    use std::{cell::RefCell, collections::VecDeque, iter::repeat, rc::Rc, task::Waker};

    use crate::cancellation::consuming;

    use super::*;

    fn waker() -> Waker {
        Waker::noop().clone()
    }

    struct Lost {
        inner: Rc<RefCell<VecDeque<RingResult>>>,
    }

    impl Lost {
        fn pop(&self) -> Option<RingResult> {
            self.inner.borrow_mut().pop_front()
        }
    }

    fn cancel() -> (Cancellation, Lost) {
        let inner = Rc::new(RefCell::new(VecDeque::new()));
        let cancel = consuming!(
            RefCell<VecDeque<RingResult>>,
            inner.clone(),
            |inner, result| {
                inner.borrow_mut().push_back(result);
            }
        );
        let lost = Lost { inner };
        (cancel, lost)
    }

    fn result() -> RingResult {
        RingResult {
            ret: unsafe { libc::rand() },
            flags: 0,
        }
    }

    fn result_multi() -> RingResult {
        RingResult {
            ret: unsafe { libc::rand() },
            flags: 2,
        }
    }

    #[derive(Clone, Debug)]
    enum C {
        NotifySingle,
        NotifyMulti,
        Cancel,
        Result,
    }

    #[derive(Clone, Debug)]
    struct Case(Vec<C>);

    impl Case {
        fn fix(mut self) -> Self {
            let diff = self.0.iter().fold(0usize, |diff, case| match case {
                C::NotifySingle | C::NotifyMulti => diff + 1,
                C::Result => diff.checked_sub(1).unwrap_or(0),
                _ => diff,
            });
            self.0.extend(repeat(C::Result).take(diff));
            self
        }

        fn clone_extend(&self, tail: &[C]) -> Self {
            let mut new = self.clone();
            new.0.extend_from_slice(tail);
            new
        }
    }

    #[test]
    fn everything() {
        let mut cases: Vec<Case> = vec![];
        let mut curr: Vec<Case> = vec![Case(vec![])];
        let mut next: Vec<Case> = vec![];

        for _ in 0..16 {
            for case in curr.drain(..) {
                next.push(case.clone_extend(&[C::NotifyMulti]));
                next.push(case.clone_extend(&[C::Result]));

                cases.push(case.clone_extend(&[C::Cancel, C::NotifySingle]));
                cases.push(case.clone_extend(&[C::NotifySingle, C::Cancel]));
                cases.push(case.clone_extend(&[C::NotifySingle]).fix());
            }

            std::mem::swap(&mut curr, &mut next);
        }

        let mut set = CompletionSet::with_capacity(16);
        for case in cases {
            let key = set.insert(waker());
            let mut queue = VecDeque::new();
            let mut completed = false;

            for c in case.0 {
                match c {
                    C::NotifySingle => {
                        let single = result();
                        set.notify(key, single);

                        queue.push_back(single.ret());
                        completed = true;
                    }

                    C::NotifyMulti => {
                        let multi = result_multi();
                        set.notify(key, multi);

                        queue.push_back(multi.ret());
                    }

                    C::Cancel => {
                        let (cancel, lost) = cancel();
                        assert_eq!(set.cancel(key, cancel), !completed);

                        while let Some(lost) = lost.pop() {
                            assert_eq!(queue.pop_front(), Some(lost.ret()));
                        }

                        assert!(queue.is_empty());
                    }

                    C::Result => {
                        assert_eq!(queue.pop_front(), set.result(key).map(|res| res.ret()));
                    }
                }
            }

            assert!(set.is_empty());
        }
    }
}
