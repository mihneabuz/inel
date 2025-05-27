use std::{
    cell::RefCell,
    fs::{self, File},
    future::Future,
    os::fd::{AsRawFd, RawFd},
    path::{Path, PathBuf},
    pin::pin,
    rc::Rc,
    str::FromStr,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Once,
    },
    task::Poll,
};

use futures::task::{self, ArcWake};
use rand::Rng;

use inel_interface::Reactor;
use inel_reactor::{
    op::{self, OpExt},
    BufferGroupKey, FileSlotKey, Ring,
};

macro_rules! assert_ready {
    ($poll:expr) => {{
        let std::task::Poll::Ready(res) = $poll else {
            panic!("poll not ready");
        };
        res
    }};
}

pub(crate) use assert_ready;

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
    const RESOURCES: u32 = 64;

    fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(
                Ring::options()
                    .submissions(Self::RESOURCES)
                    .fixed_buffers(Self::RESOURCES)
                    .auto_direct_files(Self::RESOURCES)
                    .manual_direct_files(Self::RESOURCES)
                    .build(),
            )),
        }
    }

    pub fn resources(&self) -> u32 {
        Self::RESOURCES
    }

    pub fn active(&self) -> u32 {
        self.inner.borrow().active()
    }

    pub fn is_done(&self) -> bool {
        self.inner.borrow().is_done()
    }

    pub fn try_get_file_slot(&self) -> Option<FileSlotKey> {
        self.with(|reactor| reactor.get_file_slot()).unwrap().ok()
    }

    pub fn get_file_slot(&self) -> FileSlotKey {
        self.try_get_file_slot().unwrap()
    }

    pub fn release_file_slot(&self, slot: FileSlotKey) {
        self.with(|reactor| reactor.release_file_slot(slot))
            .unwrap();
    }

    pub fn get_buffer_group(&self) -> BufferGroupKey {
        self.with(|reactor| reactor.get_buffer_group())
            .unwrap()
            .unwrap()
    }

    pub fn release_buffer_group(&self, key: BufferGroupKey) {
        self.with(|reactor| reactor.release_buffer_group(key))
            .unwrap();
    }

    pub fn register_file(&self, fd: RawFd) -> FileSlotKey {
        let notifier = WakeNotifier::new();
        let register = op::RegisterFile::new(fd).run_on(self.clone());
        let mut fut = pin!(register);

        assert!(poll!(fut, notifier).is_pending());
        assert_eq!(self.active(), 1);

        self.wait();

        let res = assert_ready!(poll!(fut, notifier));
        assert!(res.is_ok());

        res.unwrap()
    }

    pub fn block_on<F, T>(&self, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        let notifier = notifier();
        let mut fut = pin!(fut);
        loop {
            match poll!(fut, notifier) {
                Poll::Ready(res) => return res,
                Poll::Pending => {}
            }

            self.wait();
        }
    }
}

impl inel_interface::Reactor for ScopedReactor {
    type Handle = Ring;

    fn wait(&self) {
        self.inner.borrow_mut().wait()
    }

    fn with<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut Self::Handle) -> T,
    {
        let mut guard = self.inner.borrow_mut();
        Some(f(&mut guard))
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

pub fn reactor() -> ScopedReactor {
    setup_tracing();
    ScopedReactor::new()
}

pub fn notifier() -> WakeNotifier {
    WakeNotifier::new()
}

pub fn runtime() -> (ScopedReactor, WakeNotifier) {
    (reactor(), notifier())
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
        let seed = rand::rng()
            .sample_iter(rand::distr::Alphanumeric)
            .take(32)
            .map(|b| b as char)
            .collect::<String>();

        format!("/tmp/inel_reactor_test_{}", seed)
    }

    pub fn new_relative_name() -> String {
        let name = Self::new_name();
        let abs = PathBuf::from_str(&name).unwrap();
        Self::make_relative(abs).to_string_lossy().to_string()
    }

    pub fn dir() -> Self {
        let name = Self::new_name();
        std::fs::create_dir(&name).unwrap();
        Self { name, inner: None }
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

    fn make_relative(abs: PathBuf) -> PathBuf {
        let cwd = std::env::current_dir().unwrap();
        let backs = cwd.iter().count();

        let mut buf = PathBuf::new();
        (0..backs).for_each(|_| buf.push(".."));
        abs.into_iter().skip(1).for_each(|part| buf.push(part));

        buf
    }

    pub fn path(&self) -> &Path {
        Path::new(&self.name)
    }

    pub fn absolute_path(&self) -> PathBuf {
        std::fs::canonicalize(self.path()).unwrap()
    }

    pub fn relative_path(&self) -> PathBuf {
        Self::make_relative(self.absolute_path())
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
        if fs::remove_file(&self.name).is_err() {
            fs::remove_dir_all(&self.name).unwrap();
        }
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
