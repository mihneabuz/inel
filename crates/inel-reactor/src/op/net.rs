use std::{
    io::{Error, Result},
    mem::{self, MaybeUninit},
    net::SocketAddr,
    os::fd::RawFd,
    ptr::addr_of_mut,
};

use io_uring::{
    opcode::{self, AsyncCancel},
    squeue::Entry,
    types::Fd,
};

use crate::{
    util::{from_raw_addr, into_raw_addr, SocketAddrCRepr},
    Cancellation,
};

use super::Op;

pub struct Socket {
    domain: i32,
    typ: i32,
    proto: i32,
}

impl Socket {
    pub fn new(domain: i32, typ: i32) -> Self {
        Self {
            domain,
            typ,
            proto: 0,
        }
    }

    pub fn proto(mut self, proto: i32) -> Self {
        self.proto = proto;
        self
    }
}

unsafe impl Op for Socket {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::Socket::new(self.domain, self.typ, self.proto).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(ret)
        }
    }
}

pub struct Connect {
    fd: RawFd,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Connect {
    pub fn new(fd: RawFd, addr: SocketAddr) -> Self {
        let (addr, len) = into_raw_addr(addr);
        Self { fd, addr, len }
    }
}

unsafe impl Op for Connect {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::Connect::new(Fd(self.fd), self.addr.as_ptr(), self.len).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            Ok(self.fd)
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (
            Some(AsyncCancel::new(user_data).build()),
            Cancellation::empty(),
        )
    }
}

pub struct Accept {
    fd: RawFd,
    addr: Box<MaybeUninit<(SocketAddrCRepr, u32)>>,
}

impl Accept {
    pub fn new(fd: RawFd) -> Self {
        Self {
            fd,
            addr: Box::new_uninit(),
        }
    }
}

unsafe impl Op for Accept {
    type Output = Result<(RawFd, SocketAddr)>;

    fn entry(&mut self) -> Entry {
        let ptr = self.addr.as_mut_ptr();
        let (addr, len) = unsafe { (addr_of_mut!((*ptr).0), addr_of_mut!((*ptr).1)) };
        unsafe {
            len.write(mem::size_of::<SocketAddrCRepr>() as u32);
        }
        opcode::Accept::new(Fd(self.fd), addr as *mut _, len).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        if ret < 0 {
            Err(Error::from_raw_os_error(-ret))
        } else {
            let res = unsafe { self.addr.assume_init() };
            Ok((ret, from_raw_addr(&res.0, res.1)))
        }
    }

    fn cancel(self, user_data: u64) -> (Option<Entry>, Cancellation) {
        (Some(AsyncCancel::new(user_data).build()), self.addr.into())
    }
}
