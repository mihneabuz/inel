mod dir;
mod file;

use std::{
    future::Future,
    io::Result,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

pub use dir::DirBuilder;
pub use file::{DirectFile, File, Metadata, OpenOptions};
use futures::join;
use inel_interface::Reactor;
use inel_reactor::{
    buffer::StableBuffer,
    op::{self, OpExt},
    FileSlotKey,
};

use crate::GlobalReactor;

pub async fn write<P, C>(path: P, contents: C) -> Result<()>
where
    P: AsRef<Path>,
    C: StableBuffer,
{
    with_slot(async |slot| {
        let open = op::OpenAt::new(path, libc::O_RDWR | libc::O_CREAT | libc::O_TRUNC)
            .mode(0o666)
            .fixed(slot)
            .chain()
            .run_on(GlobalReactor);
        let write = op::Write::new(slot, contents).chain().run_on(GlobalReactor);
        let close = op::Close::new(slot).run_on(GlobalReactor);

        let (open, write, close) = join!(open, write, close);

        open?;
        write.1?;
        close?;

        Ok(())
    })
    .await
}

async fn with_slot<F, H, T>(f: F) -> Result<T>
where
    F: FnOnce(FileSlotKey) -> H,
    H: Future<Output = Result<T>>,
{
    WithSlot::new(f)?.await
}

pin_project_lite::pin_project! {
    struct WithSlot<F, T>
    where
        F: Future<Output = Result<T>>,
    {
        #[pin]
        future: F,
        slot: FileSlotKey,
    }

    impl<F, T> PinnedDrop for WithSlot<F, T>
    where
        F: Future<Output = Result<T>>,
    {
        fn drop(this: Pin<&mut Self>) {
            println!("RELEASING SLOT");
            let slot = *this.project().slot;
            GlobalReactor
                .with(|react| react.release_file_slot(slot))
                .unwrap();
        }
    }
}

impl<F, T> WithSlot<F, T>
where
    F: Future<Output = Result<T>>,
{
    pub fn new<H>(f: H) -> Result<Self>
    where
        H: FnOnce(FileSlotKey) -> F,
    {
        println!("ALLOCATING SLOT");
        let slot = GlobalReactor.with(|react| react.get_file_slot()).unwrap()?;
        let future = f(slot);
        Ok(Self { future, slot })
    }
}

impl<F, T> Future for WithSlot<F, T>
where
    F: Future<Output = Result<T>>,
{
    type Output = Result<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}
