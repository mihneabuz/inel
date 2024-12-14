mod fs;
mod net;
mod read;
mod time;
mod write;

use io_uring::{opcode, squeue::Entry};

use crate::{Cancellation, Ring, Submission};

pub use fs::*;
pub use net::*;
pub use read::*;
pub use time::*;
pub use write::*;

/// Implements safe operations over a [Ring] by wrapping an [io_uring::opcode]
///
/// For driving an [Op], check [Submission]
///
/// # Safety
/// If the sqe entry returned by [Op::entry] contains any pointers to buffers,
/// then those buffers *must* be valid until the sqe completes or, if [Op::cancel]
/// is called, ownership of those buffers *must* be transfered to the returned
/// [Cancellation], so they can be dropped safely when the sqe completes.
///
/// Dropping an [Op] before completion without calling [Op::cancel] _might_ cause
/// writes/reads to/from freed memory.
///
pub unsafe trait Op {
    type Output;

    /// Build the sqe entry
    fn entry(&mut self) -> Entry;

    /// Produce the final result by consuming self and cqe result
    fn result(self, ret: i32) -> Self::Output;

    /// Attempt to cancel the sqe by returning:
    ///  - an sqe entry that signals the io_uring instance to
    ///    cancel the sqe returned by [Op::entry], if applicable
    ///  - a [Cancellation] that contains any buffers currently
    ///    referenced by the sqe returned by [Op::entry]
    fn cancel(self, _user_data: u64) -> (Option<Entry>, Cancellation)
    where
        Self: Sized,
    {
        (None, Cancellation::empty())
    }

    /// Wraps self into a [crate::Submission]
    fn run_on<R>(self, reactor: R) -> Submission<Self, R>
    where
        R: inel_interface::Reactor<Handle = Ring>,
        Self: Sized,
    {
        Submission::new(reactor, self)
    }
}

/// Implements safe multishot operations over a [Ring]
/// by wrapping an [io_uring::opcode]
pub trait MultiOp: Op {
    /// Produces a result without consuming self
    fn next(&self, ret: i32) -> Self::Output;
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
}

impl MultiOp for Nop {
    fn next(&self, ret: i32) -> Self::Output {
        assert_eq!(ret, 0);
    }
}
