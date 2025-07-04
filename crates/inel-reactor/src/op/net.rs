use std::{
    io::Result,
    mem::{self, MaybeUninit},
    net::SocketAddr,
    os::fd::RawFd,
    ptr::addr_of_mut,
};

use io_uring::{opcode, squeue::Entry, types::DestinationSlot};

use crate::{
    op::{util, MultiOp, Op},
    ring::RingResult,
    util::{from_raw_addr, into_raw_addr, SocketAddrCRepr},
    AsSource, Cancellation, DirectSlot, Source,
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

    pub fn fixed(self, slot: &DirectSlot) -> SocketFixed {
        SocketFixed::from_raw(self, slot)
    }

    pub fn direct(self) -> SocketAuto {
        SocketAuto::from_raw(self)
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

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_fd(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct SocketFixed {
    inner: Socket,
    slot: DestinationSlot,
}

impl SocketFixed {
    fn from_raw(op: Socket, slot: &DirectSlot) -> Self {
        Self {
            inner: op,
            slot: slot.as_destination_slot(),
        }
    }
}

unsafe impl Op for SocketFixed {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        self.inner.entry_raw().file_index(Some(self.slot)).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct SocketAuto {
    inner: Socket,
}

impl SocketAuto {
    fn from_raw(op: Socket) -> Self {
        Self { inner: op }
    }
}

unsafe impl Op for SocketAuto {
    type Output = Result<DirectSlot>;

    fn entry(&mut self) -> Entry {
        self.inner
            .entry_raw()
            .file_index(Some(DestinationSlot::auto_target()))
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_direct(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct Bind {
    src: Source,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Bind {
    pub fn new(source: &impl AsSource, addr: SocketAddr) -> Self {
        let (addr, len) = into_raw_addr(addr);
        Self {
            src: source.as_source(),
            addr,
            len,
        }
    }
}

unsafe impl Op for Bind {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Bind::new(self.src.as_raw(), self.addr.as_ptr(), self.len).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_: u64) -> Option<Entry> {
        None
    }
}

pub struct Listen {
    src: Source,
    backlog: u32,
}

impl Listen {
    pub fn new(source: &impl AsSource, backlog: u32) -> Self {
        Self {
            src: source.as_source(),
            backlog,
        }
    }
}

unsafe impl Op for Listen {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Listen::new(self.src.as_raw(), self.backlog as i32).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_: u64) -> Option<Entry> {
        None
    }
}

pub struct Connect {
    src: Source,
    addr: SocketAddrCRepr,
    len: u32,
}

impl Connect {
    pub fn new(source: &impl AsSource, addr: SocketAddr) -> Self {
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

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }
}

pub struct Accept {
    src: Source,
    addr: Box<MaybeUninit<(SocketAddrCRepr, u32)>>,
}

impl Accept {
    pub fn new(source: &impl AsSource) -> Self {
        Self {
            src: source.as_source(),
            addr: Box::new_uninit(),
        }
    }

    pub fn fixed(self, slot: &DirectSlot) -> AcceptFixed {
        AcceptFixed::from_raw(self, slot)
    }

    pub fn direct(self) -> AcceptAuto {
        AcceptAuto::from_raw(self)
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

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_fd(&res).map(|fd| {
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
    slot: DestinationSlot,
}

impl AcceptFixed {
    fn from_raw(op: Accept, slot: &DirectSlot) -> Self {
        Self {
            inner: op,
            slot: slot.as_destination_slot(),
        }
    }
}

unsafe impl Op for AcceptFixed {
    type Output = Result<SocketAddr>;

    fn entry(&mut self) -> Entry {
        self.inner.entry_raw().file_index(Some(self.slot)).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res).map(|_| {
            let res = unsafe { self.inner.addr.assume_init() };
            from_raw_addr(&res.0, res.1)
        })
    }

    fn cancel(self) -> Cancellation {
        self.inner.cancel()
    }
}

pub struct AcceptAuto {
    inner: Accept,
}

impl AcceptAuto {
    fn from_raw(op: Accept) -> Self {
        Self { inner: op }
    }
}

unsafe impl Op for AcceptAuto {
    type Output = Result<(DirectSlot, SocketAddr)>;

    fn entry(&mut self) -> Entry {
        self.inner
            .entry_raw()
            .file_index(Some(DestinationSlot::auto_target()))
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_direct(&res).map(|slot| {
            let res = unsafe { self.inner.addr.assume_init() };
            (slot, from_raw_addr(&res.0, res.1))
        })
    }

    fn cancel(self) -> Cancellation {
        self.inner.cancel()
    }
}

pub struct Shutdown {
    src: Source,
    how: libc::c_int,
}

impl Shutdown {
    pub fn new(source: &impl AsSource, how: std::net::Shutdown) -> Self {
        Self {
            src: source.as_source(),
            how: match how {
                std::net::Shutdown::Read => libc::SHUT_RD,
                std::net::Shutdown::Write => libc::SHUT_WR,
                std::net::Shutdown::Both => libc::SHUT_RDWR,
            },
        }
    }
}

unsafe impl Op for Shutdown {
    type Output = Result<()>;

    fn entry(&mut self) -> Entry {
        opcode::Shutdown::new(self.src.as_raw(), self.how).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        util::expect_zero(&res)
    }

    fn entry_cancel(_key: u64) -> Option<Entry> {
        None
    }
}

pub struct AcceptMulti {
    src: Source,
}

impl AcceptMulti {
    pub fn new(source: &impl AsSource) -> Self {
        Self {
            src: source.as_source(),
        }
    }

    pub fn direct(self) -> AcceptMultiAuto {
        AcceptMultiAuto { src: self.src }
    }
}

unsafe impl Op for AcceptMulti {
    type Output = Result<RawFd>;

    fn entry(&mut self) -> Entry {
        opcode::AcceptMulti::new(self.src.as_raw()).build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }
}

impl MultiOp for AcceptMulti {
    fn next(&self, res: RingResult) -> Self::Output {
        util::expect_fd(&res)
    }
}

pub struct AcceptMultiAuto {
    src: Source,
}

unsafe impl Op for AcceptMultiAuto {
    type Output = Result<DirectSlot>;

    fn entry(&mut self) -> Entry {
        opcode::AcceptMulti::new(self.src.as_raw())
            .allocate_file_index(true)
            .build()
    }

    fn result(self, res: RingResult) -> Self::Output {
        self.next(res)
    }
}

impl MultiOp for AcceptMultiAuto {
    fn next(&self, res: RingResult) -> Self::Output {
        util::expect_direct(&res)
    }
}
