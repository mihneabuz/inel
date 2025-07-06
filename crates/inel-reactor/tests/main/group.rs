use std::pin::pin;

use inel_interface::Reactor;
use inel_reactor::op::{self, OpExt};

use crate::{
    assert_ready,
    helpers::{runtime, TempFile, MESSAGE},
    poll,
};

#[test]
fn provide() {
    let (reactor, notifier) = runtime();
    let group = reactor.get_buffer_group();

    let p1 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 0).run_on(reactor.clone()) };
    let p2 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 1).run_on(reactor.clone()) };
    let p3 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 3).run_on(reactor.clone()) };

    let mut futures = [pin!(p1), pin!(p2), pin!(p3)];
    for fut in &mut futures {
        assert!(poll!(fut, notifier).is_pending());
    }

    reactor.wait();

    for fut in &mut futures {
        assert!(notifier.try_recv().is_some());
        assert!(assert_ready!(poll!(fut, notifier)).1.is_ok());
    }

    reactor.release_buffer_group(group);
    assert!(reactor.is_done());
}

#[test]
fn remove() {
    let (reactor, _) = runtime();
    let group = reactor.get_buffer_group();

    let p1 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 0).run_on(reactor.clone()) };
    let p2 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 1).run_on(reactor.clone()) };
    let p3 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 128]), 3).run_on(reactor.clone()) };

    assert!(reactor.block_on(p1).1.is_ok());
    assert!(reactor.block_on(p2).1.is_ok());
    assert!(reactor.block_on(p3).1.is_ok());

    let remove = op::RemoveBuffers::new(&group, 3).run_on(reactor.clone());
    let res = reactor.block_on(remove);
    assert!(res.is_ok_and(|removed| removed == 3));

    reactor.release_buffer_group(group);
    assert!(reactor.is_done());
}

#[test]
fn read() {
    let (reactor, _) = runtime();
    let group = reactor.get_buffer_group();

    let p1 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 64]), 1).run_on(reactor.clone()) };
    let p2 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 64]), 7).run_on(reactor.clone()) };
    let p3 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 64]), 13).run_on(reactor.clone()) };

    let (buf1, res1) = reactor.block_on(p1);
    let (buf2, res2) = reactor.block_on(p2);
    let (buf3, res3) = reactor.block_on(p3);

    assert!(res1.is_ok());
    assert!(res2.is_ok());
    assert!(res3.is_ok());

    let file = TempFile::with_content(MESSAGE);

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read2 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read3 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read4 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());

    let (id1, read1) = reactor.block_on(read1).unwrap();
    let (id2, read2) = reactor.block_on(read2).unwrap();
    let (id3, read3) = reactor.block_on(read3).unwrap();
    assert!(reactor.block_on(read4).is_err());

    assert_eq!(read1, 64);
    assert_eq!(read2, 64);
    assert_eq!(read3, 64);

    assert_eq!(id1 + id2 + id3, 1 + 7 + 13);

    let p1 = unsafe { op::ProvideBuffer::new(&group, buf1, 1).run_on(reactor.clone()) };
    let p2 = unsafe { op::ProvideBuffer::new(&group, buf2, 1).run_on(reactor.clone()) };
    let p3 = unsafe { op::ProvideBuffer::new(&group, buf3, 1).run_on(reactor.clone()) };

    let (buf1, res1) = reactor.block_on(p1);
    let (buf2, res2) = reactor.block_on(p2);
    let (buf3, res3) = reactor.block_on(p3);

    assert!(res1.is_ok());
    assert!(res2.is_ok());
    assert!(res3.is_ok());

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let (_, read1) = reactor.block_on(read1).unwrap();
    assert_eq!(read1, 64);

    std::mem::drop(buf1);
    std::mem::drop(buf2);
    std::mem::drop(buf3);

    reactor.release_buffer_group(group);
    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();
    let group = reactor.get_buffer_group();

    let mut p1 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 64]), 1).run_on(reactor.clone()) };
    let mut p2 =
        unsafe { op::ProvideBuffer::new(&group, Box::new([0; 64]), 2).run_on(reactor.clone()) };
    let (mut fut1, mut fut2) = (pin!(&mut p1), pin!(&mut p2));

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    std::mem::drop(p1);
    std::mem::drop(p2);

    reactor.wait();

    let file = TempFile::with_content(MESSAGE);

    let read1 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read2 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());
    let read3 = op::ReadGroup::new(file.fd(), &group).run_on(reactor.clone());

    let (id1, _) = reactor.block_on(read1).unwrap();
    let (id2, _) = reactor.block_on(read2).unwrap();
    assert_eq!(id1 + id2, 1 + 2);
    assert!(reactor.block_on(read3).is_err());

    reactor.release_buffer_group(group);
    assert!(reactor.is_done());
}
