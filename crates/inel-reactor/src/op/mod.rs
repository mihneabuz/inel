mod direct;
mod fs;
mod group;
mod net;
mod read;
mod time;
mod write;

use std::{
    io::{Error, Result},
    os::fd::RawFd,
};

use io_uring::{
    opcode,
    squeue::{Entry, Flags},
};

use crate::{
    cancellation::Cancellation, ring::Ring, ring::RingResult, submission::Submission, RingReactor,
};

pub use direct::*;
pub use fs::*;
pub use group::*;
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
/// Dropping an [Op] before completion without calling [Op::cancel] _may_ cause
/// writes/reads to/from freed memory.
///
pub unsafe trait Op: Sized {
    type Output;

    /// Build the sqe entry
    fn entry(&mut self) -> Entry;

    /// Produce the final result by consuming self and cqe result
    fn result(self, res: RingResult) -> Self::Output;

    /// Comsume self and return a [Cancellation] that contains any
    /// buffers currently referenced by the sqe returned by [Op::entry]
    fn cancel(self) -> Cancellation {
        Cancellation::empty()
    }

    /// Build an sqe entry that cancels an entry created with [Op::entry]
    fn entry_cancel(key: u64) -> Option<Entry> {
        Some(opcode::AsyncCancel::new(key).build())
    }
}

/// Marks an [Op] that can generate multiple completions
pub trait MultiOp: Op {
    /// Produces a result without consuming self
    fn next(&self, res: RingResult) -> Self::Output;
}

/// Marks an [Op] that can be run detached, when the completion result is not relevant
pub trait DetachOp: Op {
    /// Submits an entry to the [Ring]
    fn run_detached<R>(mut self, reactor: &mut R)
    where
        R: inel_interface::Reactor<Handle = Ring>,
        Self: Op + Sized,
    {
        unsafe { reactor.submit_detached(self.entry()) };
    }
}

pub trait OpExt {
    /// Wraps self into a [Chain] that will bind this [Op] with the next one submited
    fn chain(self) -> Chain<Self>
    where
        Self: Op + Sized;

    /// Wraps self into a [Submission] to be used as a [std::future::Future]
    fn run_on<R>(self, reactor: R) -> Submission<Self, R>
    where
        R: inel_interface::Reactor<Handle = Ring>,
        Self: Op + Sized;
}

impl<O: Op> OpExt for O {
    fn chain(self) -> Chain<Self> {
        Chain::new(self)
    }

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

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }
}

impl MultiOp for Nop {
    fn next(&self, res: RingResult) -> Self::Output {
        assert!(res.ret() == 0 || res.ret() == -libc::ECANCELED)
    }
}

impl DetachOp for Nop {}

/// Wraps an [Op] and adds the link flag to the generated entry.
/// This will it to be linked to the next [Op] submitted to the [Ring].
pub struct Chain<O> {
    inner: O,
}

impl<O> Chain<O> {
    pub fn new(op: O) -> Self {
        Self { inner: op }
    }
}

unsafe impl<O> Op for Chain<O>
where
    O: Op,
{
    type Output = O::Output;

    fn entry(&mut self) -> Entry {
        O::entry(&mut self.inner).flags(Flags::IO_LINK)
    }

    fn result(self, res: RingResult) -> Self::Output {
        O::result(self.inner, res)
    }

    fn cancel(self) -> Cancellation {
        O::cancel(self.inner)
    }

    fn entry_cancel(key: u64) -> Option<Entry> {
        O::entry_cancel(key)
    }
}

pub(crate) mod util {
    use super::*;

    use crate::source::DirectAutoFd;

    pub(crate) fn expect_zero(res: &RingResult) -> Result<()> {
        let ret = res.ret();
        match ret {
            0 => Ok(()),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            _ => unreachable!(),
        }
    }

    pub(crate) fn expect_fd(res: &RingResult) -> Result<RawFd> {
        let ret = res.ret();
        match ret {
            1.. => Ok(ret),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            0 => unreachable!(),
        }
    }

    pub(crate) fn expect_direct(res: &RingResult) -> Result<DirectAutoFd> {
        let ret = res.ret();
        match ret {
            1.. => Ok(DirectAutoFd::from_raw_slot(ret as u32)),
            ..0 => Err(Error::from_raw_os_error(-ret)),
            0 => unreachable!(),
        }
    }

    pub(crate) fn expect_positive(res: &RingResult) -> Result<usize> {
        let ret = res.ret();
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret as usize)
        }
    }
}
