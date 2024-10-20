use futures::task::{self, ArcWake};
use inel_reactor::Ring;
use std::{
    cell::RefCell,
    fs::{self, File},
    os::fd::{AsRawFd, RawFd},
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

    pub fn is_done(&self) -> bool {
        self.inner.borrow().is_done()
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

macro_rules! poll {
    ($fut:expr, $notifier:expr) => {{
        let waker = $notifier.waker();
        let mut context = std::task::Context::from_waker(&waker);
        std::future::Future::poll($fut.as_mut(), &mut context)
    }};
}

pub(crate) use poll;

pub struct TempFile {
    name: String,
    inner: Option<File>,
}

impl TempFile {
    pub fn new_name() -> String {
        format!("/tmp/inel_reactor_test_{}", uuid::Uuid::new_v4())
    }

    pub fn empty() -> Self {
        let name = Self::new_name();
        let file = File::create_new(&name).unwrap();
        Self {
            name,
            inner: Some(file),
        }
    }

    pub fn fd(&self) -> RawFd {
        self.inner.as_ref().unwrap().as_raw_fd()
    }

    pub fn name(&self) -> String {
        self.name.clone()
    }

    pub fn with_content(content: impl AsRef<str>) -> Self {
        let mut file = Self::empty();
        file.write(content);
        file
    }

    pub fn write(&mut self, content: impl AsRef<str>) {
        fs::write(&self.name, content.as_ref()).unwrap()
    }

    pub fn read(&mut self) -> String {
        fs::read_to_string(&self.name).unwrap()
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = self.inner.take();
        fs::remove_file(&self.name).unwrap();
    }
}

pub const MESSAGE: &str = "
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.
Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut
labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris
nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit
esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt
in culpa qui officia deserunt mollit anim id est laborum.";

macro_rules! assert_in_range {
    ($range:expr, $value:expr) => {{
        let range = $range;
        let value = $value;
        if (!range.contains(value)) {
            panic!("assertion `value in range` failed\n value: {value:?}\n range: {range:?}");
        }
    }};
}

pub(crate) use assert_in_range;
