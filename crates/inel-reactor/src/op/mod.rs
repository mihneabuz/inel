use std::time::Duration;

use io_uring::{opcode, squeue::Entry, types::Timespec};

use crate::{completion::Cancellation, Ring, Submission};

/// This trait allows implementing safe operations over a [crate::Ring].
///
/// # Safety
/// todo!
pub unsafe trait Op {
    type Output;

    /// Build io_uring entry that is submitted
    fn entry(&mut self) -> Entry;

    /// Consume self and io_uring result to produce final result
    fn result(self, ret: i32) -> Self::Output;

    /// Create cancelation entry if necessary
    fn cancel(&self, user_data: u64) -> Option<(Entry, Cancellation)>;

    fn run_on<R>(self, reactor: R) -> Submission<Self, R>
    where
        R: inel_interface::Reactor<Handle = Ring>,
        Self: Sized,
    {
        Submission::new(reactor, self)
    }
}

pub struct Nop;

unsafe impl Op for Nop {
    type Output = ();

    fn entry(&mut self) -> Entry {
        opcode::Nop::new().build()
    }

    fn result(self, ret: i32) -> Self::Output {
        assert_eq!(ret, 0);
    }

    fn cancel(&self, _key: u64) -> Option<(Entry, Cancellation)> {
        None
    }
}

pub struct Timeout {
    abs: Timespec,
}

impl Timeout {
    pub fn new(time: Duration) -> Self {
        Self {
            abs: Timespec::from(time),
        }
    }
}

unsafe impl Op for Timeout {
    type Output = ();

    fn entry(&mut self) -> Entry {
        opcode::Timeout::new(&self.abs).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        assert!(ret == -62 || ret == -125);
    }

    fn cancel(&self, user_data: u64) -> Option<(Entry, Cancellation)> {
        Some((
            opcode::TimeoutRemove::new(user_data).build(),
            Cancellation::empty(),
        ))
    }
}
