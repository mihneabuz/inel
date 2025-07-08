use std::pin::pin;

use inel_interface::Reactor;
use inel_reactor::{
    group::ReadBufferGroup,
    op::{self, OpExt},
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
        let p1 = group.provide(Box::new([0; 128])).run_on(reactor.clone());
        let p2 = group.provide(Box::new([0; 256])).run_on(reactor.clone());
        let p3 = group.provide(Box::new([0; 512])).run_on(reactor.clone());

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

    reactor.block_on(group.release().run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn read() {
    let (reactor, _) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    let p1 = group.provide(Box::new([0; 64])).run_on(reactor.clone());
    let p2 = group.provide(Box::new([0; 128])).run_on(reactor.clone());
    let p3 = group.provide(Box::new([0; 256])).run_on(reactor.clone());

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

    let p1 = group.provide(buf1.unwrap()).run_on(reactor.clone());
    let p2 = group.provide(buf2.unwrap()).run_on(reactor.clone());
    let p3 = group.provide(buf3.unwrap()).run_on(reactor.clone());

    reactor.block_on(p1).unwrap();
    reactor.block_on(p2).unwrap();
    reactor.block_on(p3).unwrap();

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let (_buf1, read1) = reactor.block_on(read1);
    assert_eq!(read1.unwrap(), 64);

    reactor.block_on(group.release().run_on(reactor.clone()));
    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();
    let group = ReadBufferGroup::new(reactor.clone()).unwrap();

    {
        let mut p1 = group.provide(Box::new([0; 128])).run_on(reactor.clone());
        let mut p2 = group.provide(Box::new([0; 128])).run_on(reactor.clone());
        let mut p3 = group.provide(Box::new([0; 128])).run_on(reactor.clone());

        let mut futures = [pin!(&mut p1), pin!(&mut p2), pin!(&mut p3)];
        for fut in &mut futures {
            assert!(poll!(fut, notifier).is_pending());
        }

        std::mem::drop(p1);
        std::mem::drop(p2);
        std::mem::drop(p3);

        reactor.wait();
    }

    reactor.block_on(group.release().run_on(reactor.clone()));
    assert!(reactor.is_done());
}
