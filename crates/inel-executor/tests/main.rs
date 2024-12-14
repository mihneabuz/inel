use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Once},
    task::{Context, Poll, Waker},
};

use futures::{FutureExt, SinkExt, StreamExt};
use inel_executor::Executor;
use inel_interface::Reactor;

static TRACING: Once = Once::new();
pub fn setup_tracing() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    });
}

pub fn output<T>(init: T) -> (Rc<RefCell<T>>, Rc<RefCell<T>>) {
    let location = Rc::new(RefCell::new(init));
    (location.clone(), location)
}

#[derive(Default, Clone)]
struct TestReactor {
    inner: Arc<TestReactorInner>,
}

#[derive(Default)]
struct TestReactorInner {
    waited: Cell<usize>,
    wakers: RefCell<Vec<Waker>>,
}

impl TestReactor {
    fn waited(&self) -> usize {
        self.inner.waited.get()
    }
}

impl Reactor for TestReactor {
    type Handle = Vec<Waker>;

    fn wait(&self) {
        self.inner
            .wakers
            .borrow_mut()
            .drain(..)
            .for_each(|waker| waker.wake());

        let count = self.inner.waited.take() + 1;
        self.inner.waited.set(count);
    }

    fn with<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut Self::Handle) -> T,
    {
        let mut wakers = self.inner.wakers.borrow_mut();
        Some(f(&mut wakers))
    }
}

pub struct Wait {
    iter: usize,
    reactor: Option<TestReactor>,
}

impl Wait {
    fn new(iter: usize) -> Self {
        Self {
            iter,
            reactor: None,
        }
    }

    fn with_reactor(iter: usize, reactor: TestReactor) -> Self {
        Self {
            iter,
            reactor: Some(reactor),
        }
    }
}

impl Future for Wait {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.iter == 0 {
            return Poll::Ready(());
        }

        match self.reactor.as_ref() {
            Some(react) => react
                .with(|wakers| wakers.push(cx.waker().clone()))
                .unwrap(),
            None => cx.waker().wake_by_ref(),
        }

        self.as_mut().iter -= 1;
        Poll::Pending
    }
}

#[test]
fn block_on() {
    setup_tracing();
    let exe = Executor::default();
    let react = TestReactor::default();

    let num = |a: i32| async move { a };
    let sum = |a: i32, b: i32| async move { a + b };

    let x = exe.block_on(react, async move {
        let a = num(1).await;
        let b = num(2).await;
        sum(a, b).await
    });

    assert_eq!(x, 3);
}

#[test]
fn mixed() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (value, location) = output(0);

    exe.spawn(async move {
        *location.borrow_mut() = 10;
    });

    let other = exe.block_on(react, async move { 20 });

    assert_eq!(value.take(), 10);
    assert_eq!(other, 20);
}

#[test]
fn spawn_before() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (x, x_location) = output(0);
    let (y, y_location) = output(0);

    exe.spawn(async move {
        *x_location.borrow_mut() = 1;
    });

    exe.spawn(async move {
        *y_location.borrow_mut() = 2;
    });

    exe.run(react);

    assert_eq!(x.take() + y.take(), 3);
}

#[test]
fn channel() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    exe.spawn(async move {
        sender.send(10).unwrap();
    });

    exe.spawn(async move {
        *location.borrow_mut() = receiver.await.unwrap();
    });

    exe.run(react);

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_oneshot() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (sender, receiver) = futures::channel::oneshot::channel();
    let (value, location) = output(0);

    exe.spawn(async move {
        sender.send(10).unwrap();
    });

    exe.spawn(async move {
        *location.borrow_mut() = receiver.await.unwrap();
    });

    exe.run(react);

    assert_eq!(value.take(), 10);
}

#[test]
fn channel_mpsc() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);
    let mut sender_clone = sender.clone();

    exe.spawn(async move {
        for i in 0..100 {
            sender.send(i).await.unwrap();
        }
    });

    exe.spawn(async move {
        for _ in 0..10 {
            sender_clone.send(10).await.unwrap();
        }
    });

    let total = exe.block_on(react, async move {
        receiver.fold(0, |acc, x| async move { acc + x }).await
    });

    assert_eq!(total, 10 * 10 + (100 * 99) / 2);
}

#[test]
fn channel_join() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let (mut sender, receiver) = futures::channel::mpsc::channel(5);

    let sender_handle = exe.spawn(async move {
        for i in 0..5 {
            sender.send(i).await.unwrap();
        }
    });

    let total = exe.block_on(react, async move {
        sender_handle.join().await;
        receiver.fold(0, |acc, x| async move { acc + x }).await
    });

    assert_eq!(total, 10);
}

#[test]
fn wait_single() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let res = exe.block_on(react, Wait::new(20).then(|_| async { 10 }));

    assert_eq!(res, 10);
}

#[test]
fn wait_multi() {
    setup_tracing();
    let exe = Executor::new();
    let react = TestReactor::default();

    let f = |iter| {
        exe.spawn(async move {
            Wait::new(iter).await;
            iter
        })
    };

    let res1 = f(10);
    let res2 = f(100);
    let res3 = f(1000);

    exe.run(react);

    let total: usize = [res1, res2, res3]
        .iter_mut()
        .map(|handle| handle.try_join())
        .flatten()
        .sum();

    assert_eq!(total, 1110);
}

#[test]
fn with_reactor() {
    setup_tracing();

    let iter = 5;
    let exe = Executor::new();
    let react = TestReactor::default();

    let fut = Wait::with_reactor(iter, react.clone());

    exe.block_on(react.clone(), fut);

    assert_eq!(react.waited(), iter + 1);
}

#[test]
fn everything() {
    setup_tracing();

    let exe = Executor::new();
    let react = TestReactor::default();
    let (mut sender, mut receiver) = futures::channel::mpsc::channel(5);

    let fut1 = Wait::with_reactor(10, react.clone());
    let fut2 = Wait::with_reactor(100, react.clone());

    let mut h1 = exe.spawn(fut1.then(|_| async move {
        for i in 0..5 {
            assert!(sender.send(i).await.is_ok());
        }
    }));

    let mut h2 = exe.spawn(fut2.then(|_| async move {
        let mut sum = 0;
        while let Some(value) = receiver.next().await {
            sum += value;
        }
        sum
    }));

    exe.run(react.clone());

    assert_eq!(h1.try_join(), Some(()));
    assert_eq!(h2.try_join(), Some(10));

    assert_eq!(react.waited(), 101);
}
