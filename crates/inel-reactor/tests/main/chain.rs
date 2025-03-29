use std::{pin::pin, time::Duration};

use inel_interface::Reactor;
use inel_reactor::op::{self, OpExt};

use crate::{assert_ready, helpers::runtime, poll};

#[test]
fn simple() {
    let (reactor, notifier) = runtime();

    let mut timeout1 = op::Timeout::new(Duration::from_millis(10)).run_on(reactor.clone());
    let mut timeout2 = op::Timeout::new(Duration::from_millis(50))
        .chain()
        .run_on(reactor.clone());

    let mut nop = op::Nop.run_on(reactor.clone());

    let mut fut1 = pin!(&mut timeout1);
    let mut fut2 = pin!(&mut timeout2);
    let mut fut3 = pin!(&mut nop);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    reactor.wait();

    assert_ready!(poll!(fut1, notifier));
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());

    reactor.wait();

    assert_ready!(poll!(fut2, notifier));
    assert_ready!(poll!(fut3, notifier));

    assert!(reactor.is_done());
}

#[test]
fn cancel() {
    let (reactor, notifier) = runtime();

    let mut timeout1 = op::Timeout::new(Duration::from_millis(10)).run_on(reactor.clone());
    let mut timeout2 = op::Timeout::new(Duration::from_millis(50))
        .chain()
        .run_on(reactor.clone());

    let mut nop = op::Nop.run_on(reactor.clone());

    let mut fut1 = pin!(&mut timeout1);
    let mut fut2 = pin!(&mut timeout2);
    let mut fut3 = pin!(&mut nop);

    assert!(poll!(fut1, notifier).is_pending());
    assert!(poll!(fut2, notifier).is_pending());
    assert!(poll!(fut3, notifier).is_pending());
    assert_eq!(reactor.active(), 3);

    std::mem::drop(timeout2);

    reactor.wait();

    assert!(poll!(fut1, notifier).is_pending());
    assert_ready!(poll!(fut3, notifier));

    reactor.wait();

    assert_ready!(poll!(fut1, notifier));

    assert!(reactor.is_done());
}
