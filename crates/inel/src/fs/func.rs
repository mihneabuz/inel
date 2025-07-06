use std::{io::Result, path::Path};

use inel_reactor::{
    buffer::{StableBuffer, StableBufferMut},
    op::{self, OpExt},
};

use crate::{source::OwnedDirect, GlobalReactor};

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
    let direct = OwnedDirect::reserve().unwrap();
    op::OpenAt::new(path, libc::O_WRONLY)
        .fixed(direct.slot())
        .chain()
        .run_on(GlobalReactor)
        .await
        .is_ok()
}

pub async fn write<P, C>(path: P, contents: C) -> (C, Result<()>)
where
    P: AsRef<Path>,
    C: StableBuffer,
{
    let direct = OwnedDirect::reserve().unwrap();

    let open = op::OpenAt::new(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
        .mode(0o666)
        .fixed(direct.slot())
        .chain()
        .run_on(GlobalReactor);
    let write = op::Write::new(&direct, contents)
        .chain()
        .run_on(GlobalReactor);

    let (open, write) = futures::future::join(open, write).await;
    let (buf, write) = write;

    let (buf, _) = btry!(buf, open);
    let (buf, wrote) = btry!(buf, write);

    assert_eq!(buf.size(), wrote);

    (buf, Ok(()))
}

pub async fn read<P, B>(path: P, buffer: B) -> (B, Result<usize>)
where
    P: AsRef<Path>,
    B: StableBufferMut,
{
    let direct = OwnedDirect::reserve().unwrap();

    let open = op::OpenAt::new(path, libc::O_RDWR)
        .fixed(direct.slot())
        .chain()
        .run_on(GlobalReactor);
    let read = op::Read::new(&direct, buffer).chain().run_on(GlobalReactor);

    let (open, read) = futures::future::join(open, read).await;
    let (buf, read) = read;

    let (buf, _) = btry!(buf, open);
    let (buf, read) = btry!(buf, read);

    (buf, Ok(read))
}
