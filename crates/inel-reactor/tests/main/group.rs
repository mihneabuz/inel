use std::{io::Write, os::fd::AsRawFd, pin::pin, rc::Rc};

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
    let (mut reactor, notifier) = runtime();
    let group = Rc::new(ReadBufferGroup::register(&mut reactor).unwrap());

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

    assert_eq!(group.present(), 3);

    let group = Rc::into_inner(group).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}

#[test]
fn release_empty() {
    let (mut reactor, _) = runtime();
    let group = ReadBufferGroup::register(&mut reactor).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}

#[test]
fn provide_detach() {
    let (mut reactor, notifier) = runtime();
    let group = Rc::new(ReadBufferGroup::register(&mut reactor).unwrap());

    op::ProvideBuffer::new(&group, Box::new([0; 128])).run_detached(&mut reactor);
    op::ProvideBuffer::new(&group, Box::new([0; 256])).run_detached(&mut reactor);
    op::ProvideBuffer::new(&group, Box::new([0; 512])).run_detached(&mut reactor);

    reactor.wait();
    assert!(notifier.try_recv().is_none());

    assert_eq!(group.present(), 3);

    let group = Rc::into_inner(group).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}

#[test]
fn read() {
    let (mut reactor, _) = runtime();
    let group = Rc::new(ReadBufferGroup::register(&mut reactor).unwrap());

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

    let group = Rc::into_inner(group).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}

#[test]
fn read_multi() {
    let (mut reactor, _) = runtime();
    let group = Rc::new(ReadBufferGroup::register(&mut reactor).unwrap());

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

    let group = Rc::into_inner(group).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (mut reactor, notifier) = runtime();
    let group = Rc::new(ReadBufferGroup::register(&mut reactor).unwrap());

    {
        let mut p1 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let mut p2 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());
        let mut p3 = op::ProvideBuffer::new(&group, Box::new([0; 128])).run_on(reactor.clone());

        let mut futures = [pin!(&mut p1), pin!(&mut p2), pin!(&mut p3)];
        for fut in &mut futures {
            assert!(poll!(fut, notifier).is_pending());
        }

        reactor.wait();

        std::mem::drop(p1);
        std::mem::drop(p2);
        std::mem::drop(p3);
    }

    assert_eq!(group.present(), 3);

    let file = TempFile::with_content(MESSAGE);

    {
        let mut read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
        let mut read2 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
        let mut read3 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());

        let mut futures = [pin!(&mut read1), pin!(&mut read2), pin!(&mut read3)];
        for fut in &mut futures {
            assert!(poll!(fut, notifier).is_pending());
        }

        reactor.wait();

        std::mem::drop(read1);
        std::mem::drop(read2);
        std::mem::drop(read3);

        reactor.wait();
    }

    assert_eq!(group.present(), 0);

    let mut cancelled = Vec::new();
    while let Some(buffer) = group.get_cancelled() {
        cancelled.push(buffer);
    }

    assert_eq!(cancelled.len(), 3);

    for buffer in cancelled {
        reactor
            .block_on(op::ProvideBuffer::new(&group, buffer).run_on(reactor.clone()))
            .unwrap();
    }

    assert_eq!(group.present(), 3);

    let (pipe_reader, mut pipe_writer) = std::io::pipe().unwrap();
    let fd = pipe_reader.as_raw_fd();
    pipe_writer.write(MESSAGE.as_bytes()).unwrap();

    {
        let mut read = op::ReadGroupMulti::new(fd, &group).run_on(reactor.clone());
        assert!(poll!(pin!(&mut read), notifier).is_pending());

        reactor.wait();

        std::mem::drop(read);
    }

    assert_eq!(group.present(), 0);

    let mut cancelled = Vec::new();
    while let Some(buffer) = group.get_cancelled() {
        cancelled.push(buffer);
    }

    assert_eq!(cancelled.len(), 3);

    let group = Rc::into_inner(group).unwrap();
    let group = reactor.block_on(op::RemoveBuffers::new(group).run_on(reactor.clone()));
    group.release(&mut reactor);
    assert!(reactor.is_done());
}
