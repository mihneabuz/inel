use std::{
    future::Future,
    io::Result,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use futures::join;
use inel_interface::Reactor;
use inel_reactor::{
    buffer::{StableBuffer, StableBufferMut},
    op::{self, OpExt},
    FileSlotKey,
};

use crate::GlobalReactor;

macro_rules! btry {
    ($buf:expr, $res:expr) => {{
        match $res {
            Ok(x) => ($buf, x),
            Err(e) => {
                return ($buf, Err(e));
            }
        }
    }};
}

pub async fn exists<P>(path: P) -> bool
where
    P: AsRef<Path>,
{
    with_slot(async |slot| {
        let open = op::OpenAt::new(path, libc::O_WRONLY)
            .fixed(slot)
            .chain()
            .run_on(GlobalReactor);

        let close = op::Close::new(slot).run_on(GlobalReactor);

        let (open, _) = join!(open, close);

        open.map(|_| true).unwrap_or(false)
    })
    .await
}

pub async fn write<P, C>(path: P, contents: C) -> (C, Result<()>)
where
    P: AsRef<Path>,
    C: StableBuffer,
{
    with_slot(async |slot| {
        let open = op::OpenAt::new(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
            .mode(0o666)
            .fixed(slot)
            .chain()
            .run_on(GlobalReactor);
        let write = op::Write::new(slot, contents).chain().run_on(GlobalReactor);
        let close = op::Close::new(slot).run_on(GlobalReactor);

        let (open, write, _) = join!(open, write, close);
        let (buf, write) = write;

        let (buf, _) = btry!(buf, open);
        let (buf, wrote) = btry!(buf, write);

        assert_eq!(buf.size(), wrote);

        (buf, Ok(()))
    })
    .await
}

pub async fn read<P, B>(path: P, buffer: B) -> (B, Result<usize>)
where
    P: AsRef<Path>,
    B: StableBufferMut,
{
    with_slot(async |slot| {
        let open = op::OpenAt::new(path, libc::O_RDWR)
            .fixed(slot)
            .chain()
            .run_on(GlobalReactor);
        let read = op::Read::new(slot, buffer).chain().run_on(GlobalReactor);
        let close = op::Close::new(slot).run_on(GlobalReactor);

        let (open, read, _) = join!(open, read, close);
        let (buf, read) = read;

        let (buf, _) = btry!(buf, open);
        let (buf, read) = btry!(buf, read);

        (buf, Ok(read))
    })
    .await
}

async fn with_slot<F, H, T>(f: F) -> T
where
    F: FnOnce(FileSlotKey) -> H,
    H: Future<Output = T>,
{
    WithSlot::new(f).unwrap().await
}

pin_project_lite::pin_project! {
    struct WithSlot<F, T>
    where
        F: Future<Output = T>,
    {
        #[pin]
        future: F,
        slot: FileSlotKey,
    }

    impl<F, T> PinnedDrop for WithSlot<F, T>
    where
        F: Future<Output = T>,
    {
        fn drop(this: Pin<&mut Self>) {
            let slot = *this.project().slot;
            GlobalReactor
                .with(|react| react.release_file_slot(slot))
                .unwrap();
        }
    }
}

impl<F, T> WithSlot<F, T>
where
    F: Future<Output = T>,
{
    pub fn new<H>(f: H) -> Result<Self>
    where
        H: FnOnce(FileSlotKey) -> F,
    {
        let slot = GlobalReactor.with(|react| react.get_file_slot()).unwrap()?;
        let future = f(slot);
        Ok(Self { future, slot })
    }
}

impl<F, T> Future for WithSlot<F, T>
where
    F: Future<Output = T>,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}
