use std::{
    io::Result,
    mem::{self, MaybeUninit},
    net::SocketAddr,
    os::fd::RawFd,
    ptr::addr_of_mut,
};

use io_uring::{opcode, squeue::Entry};

use crate::{
    op::{util, MultiOp, Op},
    util::{from_raw_addr, into_raw_addr, SocketAddrCRepr},
    AsSource, Cancellation, FileSlotKey, Source,
};

pub struct Socket {
    domain: i32,
    typ: i32,
    proto: i32,
}

impl Socket {
    pub fn stream_from_addr(addr: &SocketAddr) -> Self {
        let domain = if addr.is_ipv4() {
            libc::AF_INET
        } else {
            libc::AF_INET6
        };

        Self::new(domain, libc::SOCK_STREAM)
    }

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

    pub fn fixed(self, slot: FileSlotKey) -> SocketFixed {
        SocketFixed::from_raw(self, slot)
    }

    fn entry_raw(&self) -> opcode::Socket {
        opcode::Socket::new(self.domain, self.typ, self.proto)
    }
}

unsafe impl Op for Socket {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        self.entry_raw().build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_fd(ret)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct SocketFixed {
    inner: Socket,
    slot: FileSlotKey,
}

impl SocketFixed {
    fn from_raw(op: Socket, slot: FileSlotKey) -> Self {
        Self { inner: op, slot }
    }
}

unsafe impl Op for SocketFixed {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        self.inner
            .entry_raw()
            .file_index(Some(self.slot.as_destination_slot()))
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_zero(ret)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct Connect {
    src: Source,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Connect {
    pub fn new(source: impl AsSource, addr: SocketAddr) -> Self {
        let (addr, len) = into_raw_addr(addr);
        Self {
            src: source.as_source(),
            addr,
            len,
        }
    }
}

unsafe impl Op for Connect {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Connect::new(self.src.as_raw(), self.addr.as_ptr(), self.len).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_zero(ret)
    }
}

pub struct Accept {
    src: Source,
    addr: Box<MaybeUninit<(SocketAddrCRepr, u32)>>,
}

impl Accept {
    pub fn new(source: impl AsSource) -> Self {
        Self {
            src: source.as_source(),
            addr: Box::new_uninit(),
        }
    }

    pub fn fixed(self, slot: FileSlotKey) -> AcceptFixed {
        AcceptFixed::from_raw(self, slot)
    }

    fn entry_raw(&mut self) -> opcode::Accept {
        let ptr = self.addr.as_mut_ptr();
        let (addr, len) = unsafe { (addr_of_mut!((*ptr).0), addr_of_mut!((*ptr).1)) };
        unsafe {
            len.write(mem::size_of::<SocketAddrCRepr>() as u32);
        }
        opcode::Accept::new(self.src.as_raw(), addr as *mut _, len)
    }
}

unsafe impl Op for Accept {
    type Output = Result<(RawFd, SocketAddr)>;

    fn entry(&mut self) -> Entry {
        self.entry_raw().build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_fd(ret).map(|fd| {
            let res = unsafe { self.addr.assume_init() };
            (fd, from_raw_addr(&res.0, res.1))
        })
    }

    fn cancel(self) -> Cancellation {
        self.addr.into()
    }
}

pub struct AcceptFixed {
    inner: Accept,
    slot: FileSlotKey,
}

impl AcceptFixed {
    fn from_raw(op: Accept, slot: FileSlotKey) -> Self {
        Self { inner: op, slot }
    }
}

unsafe impl Op for AcceptFixed {
    type Output = Result<SocketAddr>;

    fn entry(&mut self) -> Entry {
        self.inner
            .entry_raw()
            .file_index(Some(self.slot.as_destination_slot()))
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_zero(ret).map(|_| {
            let res = unsafe { self.inner.addr.assume_init() };
            from_raw_addr(&res.0, res.1)
        })
    }

    fn cancel(self) -> Cancellation {
        self.inner.cancel()
    }
}

pub struct Shutdown {
    src: Source,
    how: std::net::Shutdown,
}

impl Shutdown {
    pub fn new(source: impl AsSource, how: std::net::Shutdown) -> Self {
        Self {
            src: source.as_source(),
            how,
        }
    }
}

unsafe impl Op for Shutdown {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        let how = match self.how {
            std::net::Shutdown::Read => libc::SHUT_RD,
            std::net::Shutdown::Write => libc::SHUT_WR,
            std::net::Shutdown::Both => libc::SHUT_RDWR,
        };
        opcode::Shutdown::new(self.src.as_raw(), how).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        util::expect_zero(ret)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct AcceptMulti {
    src: Source,
}

impl AcceptMulti {
    pub fn new(source: impl AsSource) -> Self {
        Self {
            src: source.as_source(),
        }
    }
}

unsafe impl Op for AcceptMulti {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::AcceptMulti::new(self.src.as_raw()).build()
    }

    fn result(self, ret: i32) -> Self::Output {
        self.next(ret)
    }
}

impl MultiOp for AcceptMulti {
    fn next(&self, ret: i32) -> Self::Output {
        util::expect_fd(ret)
    }
}

pub struct AcceptMultiFixed {
    src: Source,
}

impl AcceptMultiFixed {
    pub fn new(source: impl AsSource) -> Self {
        Self {
            src: source.as_source(),
        }
    }
}

unsafe impl Op for AcceptMultiFixed {
    type Output = Result<FileSlotKey>;

    fn entry(&mut self) -> Entry {
        opcode::AcceptMulti::new(self.src.as_raw())
            .allocate_file_index(true)
            .build()
    }

    fn result(self, ret: i32) -> Self::Output {
        self.next(ret)
    }
}

impl MultiOp for AcceptMultiFixed {
    fn next(&self, ret: i32) -> Self::Output {
        util::expect_positive(ret).map(|slot| FileSlotKey::from_raw_slot(slot as u32))
    }
}
