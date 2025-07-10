use std::{io::Write, os::fd::AsRawFd, pin::pin};

use futures::StreamExt;
use inel_interface::Reactor;
use inel_reactor::{
    group::ReadBufferGroup,
    op::{self, DetachOp, OpExt},
};

use crate::{
    assert_ready,
    helpers::{runtime, TempFile, MESSAGE},
    poll,
};

#[test]
fn provide() {
    let (reactor, notifier) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    {
        let p1 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let p2 = op::ProvideBuffer::new(&group, Box::new([0; 256])).run_on(reactor.clone());
        let p3 = op::ProvideBuffer::new(&group, Box::new([0; 512])).run_on(reactor.clone());

        let mut futures = [pin!(p1), pin!(p2), pin!(p3)];
        for fut in &mut futures {
            assert!(poll!(fut, notifier).is_pending());
        }

        reactor.wait();

        for fut in &mut futures {
            assert!(notifier.try_recv().is_some());
            assert!(assert_ready!(poll!(fut, notifier)).is_ok());
        }
    }

    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn release_empty() {
    let (reactor, _) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();
    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn provide_detach() {
    let (mut reactor, notifier) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    op::ProvideBuffer::new(&group, Box::new([0; 128])).run_detached(&mut reactor);
    op::ProvideBuffer::new(&group, Box::new([0; 256])).run_detached(&mut reactor);
    op::ProvideBuffer::new(&group, Box::new([0; 512])).run_detached(&mut reactor);

    reactor.wait();
    assert!(notifier.try_recv().is_none());

    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn read() {
    let (reactor, _) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    let p1 = op::ProvideBuffer::new(&group, Box::new([0; 64])).run_on(reactor.clone());
    let p2 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
    let p3 = op::ProvideBuffer::new(&group, Box::new([0; 256])).run_on(reactor.clone());

    reactor.block_on(p1).unwrap();
    reactor.block_on(p2).unwrap();
    reactor.block_on(p3).unwrap();

    let file = TempFile::with_content(MESSAGE);

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read2 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read3 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read4 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());

    let (buf1, read1) = reactor.block_on(read1);
    let (buf2, read2) = reactor.block_on(read2);
    let (buf3, read3) = reactor.block_on(read3);
    let (buf4, read4) = reactor.block_on(read4);
    assert!(buf4.is_none());
    assert!(read4.is_err());

    assert_eq!(read1.unwrap(), 64);
    assert_eq!(read2.unwrap(), 128);
    assert_eq!(read3.unwrap(), 256);

    let p1 = op::ProvideBuffer::new(&group, buf1.unwrap()).run_on(reactor.clone());
    let p2 = op::ProvideBuffer::new(&group, buf2.unwrap()).run_on(reactor.clone());
    let p3 = op::ProvideBuffer::new(&group, buf3.unwrap()).run_on(reactor.clone());

    reactor.block_on(p1).unwrap();
    reactor.block_on(p2).unwrap();
    reactor.block_on(p3).unwrap();

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let (_buf1, read1) = reactor.block_on(read1);
    assert_eq!(read1.unwrap(), 64);

    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    {
        let mut p1 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let mut p2 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let mut p3 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());

        let mut futures = [pin!(&mut p1), pin!(&mut p2), pin!(&mut p3)];
        for fut in &mut futures {
            assert!(poll!(fut, notifier).is_pending());
        }

        std::mem::drop(p1);
        std::mem::drop(p2);
        std::mem::drop(p3);

        reactor.wait();
    }

    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn read_multi() {
    let (mut reactor, _) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    let (pipe_reader, mut pipe_writer) = std::io::pipe().unwrap();
    let fd = pipe_reader.as_raw_fd();
    pipe_writer.write(MESSAGE.as_bytes()).unwrap();

    {
        let p1 = op::ProvideBuffer::new(&group, Box::new([0; 64])).run_on(reactor.clone());
        let p2 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let p3 = op::ProvideBuffer::new(&group, Box::new([0; 256])).run_on(reactor.clone());

        reactor.block_on(p1).unwrap();
        reactor.block_on(p2).unwrap();
        reactor.block_on(p3).unwrap();

        let mut read = op::ReadGroupMulti::new(fd, &group).run_on(reactor.clone());

        let (buf1, read1) = reactor.block_on(read.next()).unwrap();
        let (buf2, read2) = reactor.block_on(read.next()).unwrap();
        let (buf3, read3) = reactor.block_on(read.next()).unwrap();
        let (buf4, read4) = reactor.block_on(read.next()).unwrap();

        let buf1 = buf1.unwrap();
        let buf2 = buf2.unwrap();
        let _buf3 = buf3.unwrap();
        assert_eq!(read1.unwrap(), 64);
        assert_eq!(read2.unwrap(), 128);
        assert_eq!(read3.unwrap(), 256);

        assert!(buf4.is_none());
        assert!(read4.is_err());

        op::ProvideBuffer::new(&group, buf1).run_detached(&mut reactor);
        op::ProvideBuffer::new(&group, buf2).run_detached(&mut reactor);
    }

    reactor.block_on(op::ReleaseGroup::new(group).run_on(reactor.clone()));
    assert!(reactor.is_done());
}
