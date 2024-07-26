use futures::task::{self, ArcWake};
use inel_reactor::Ring;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Once,
    },
};

static TRACING: Once = Once::new();
pub fn setup_tracing() {
    TRACING.call_once(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::filter::EnvFilter::from_default_env())
            .init();
    });
}

pub struct ScopedReactor {
    inner: Rc<RefCell<Ring>>,
}

impl Clone for ScopedReactor {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl ScopedReactor {
    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Ring::new())),
        }
    }

    pub fn active(&self) -> u32 {
        self.inner.borrow().active()
    }
}

impl inel_interface::Reactor for ScopedReactor {
    type Handle = Ring;

    fn wait(&self) {
        self.inner.borrow_mut().wait()
    }

    fn with<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&mut Self::Handle) -> T,
    {
        let mut guard = self.inner.borrow_mut();
        f(&mut guard)
    }
}

pub struct WakeNotifier {
    send: Sender<()>,
    recv: Receiver<()>,
}

impl WakeNotifier {
    fn new() -> Self {
        let (send, recv) = mpsc::channel();
        Self { send, recv }
    }

    pub fn waker(&self) -> task::Waker {
        task::waker(Arc::new(Waker {
            send: self.send.clone(),
        }))
    }

    pub fn try_recv(&self) -> Option<()> {
        self.recv.try_recv().ok()
    }
}

struct Waker {
    send: Sender<()>,
}

impl ArcWake for Waker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        tracing::debug!("Waker::wake_by_ref");
        arc_self.send.send(()).unwrap();
    }
}

pub fn runtime() -> (ScopedReactor, WakeNotifier) {
    setup_tracing();

    let reactor = ScopedReactor::new();
    let notifier = WakeNotifier::new();

    (reactor, notifier)
}
