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
    fn cancel(self, _user_data: u64) -> (Option<Entry>, Cancellation)
    where
        Self: Sized,
    {
        (None, Cancellation::empty())
    }

    fn run_on<R>(self, reactor: R) -> Submission<Self, R>
    where
        R: inel_interface::Reactor<Handle = Ring>,
        Self: Sized,
    {
        Submission::new(reactor, self)
    }
}

/// This trait allows implementing safe multishot operations over a [crate::Ring].
///
/// # Safety
/// todo!
pub unsafe trait MultiOp: Op {
    /// Consume self and io_uring result to produce final result
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

unsafe impl MultiOp for Nop {
    fn next(&self, ret: i32) -> Self::Output {
        assert_eq!(ret, 0);
    }
}
