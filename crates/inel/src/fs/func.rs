use std::{io::Result, path::Path};

use futures::{AsyncReadExt, AsyncWriteExt};
use inel_reactor::{
    buffer::{StableBuffer, StableBufferMut},
    op::{self, OpExt},
};

use crate::{
    fs::{file::Advice, DirBuilder, File, Metadata},
    io::{BufReader, BufWriter},
    source::OwnedDirect,
    GlobalReactor,
};

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
        .fixed(&direct)
        .run_on(GlobalReactor)
        .await
        .is_ok()
}

pub async fn write_buf<P, C>(path: P, contents: C) -> (C, Result<()>)
where
    P: AsRef<Path>,
    C: StableBuffer,
{
    let direct = OwnedDirect::reserve().unwrap();

    let open = op::OpenAt::new(path, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)
        .mode(0o666)
        .fixed(&direct)
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

pub async fn read_buf<P, B>(path: P, buffer: B) -> (B, Result<usize>)
where
    P: AsRef<Path>,
    B: StableBufferMut,
{
    let direct = OwnedDirect::reserve().unwrap();

    let open = op::OpenAt::new(path, libc::O_RDWR)
        .fixed(&direct)
        .chain()
        .run_on(GlobalReactor);
    let read = op::Read::new(&direct, buffer).chain().run_on(GlobalReactor);

    let (open, read) = futures::future::join(open, read).await;
    let (buf, read) = read;

    let (buf, _) = btry!(buf, open);
    let (buf, read) = btry!(buf, read);

    (buf, Ok(read))
}

pub async fn read<P>(path: P) -> Result<Vec<u8>>
where
    P: AsRef<Path>,
{
    let file = File::open(path).await?;
    file.advise(Advice::Sequential).await?;
    let mut reader = BufReader::new(file);
    let mut data = Vec::new();
    reader.read_to_end(&mut data).await?;
    Ok(data)
}

pub async fn read_to_string<P>(path: P) -> Result<String>
where
    P: AsRef<Path>,
{
    let file = File::open(path).await?;
    file.advise(Advice::Sequential).await?;
    let mut reader = BufReader::new(file);
    let mut data = String::new();
    reader.read_to_string(&mut data).await?;
    Ok(data)
}

pub async fn write<P, C>(path: P, contents: C) -> Result<()>
where
    P: AsRef<Path>,
    C: AsRef<[u8]>,
{
    let file = File::create(path).await?;
    let mut writer = BufWriter::new(file);
    writer.write_all(contents.as_ref()).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn metadata<P>(path: P) -> Result<Metadata>
where
    P: AsRef<Path>,
{
    let file = File::open(path).await?;
    file.metadata().await
}

pub async fn create_file<P>(path: P) -> Result<()>
where
    P: AsRef<Path>,
{
    let _ = File::create(path).await?;
    Ok(())
}

pub async fn create_dir<P>(path: P) -> Result<()>
where
    P: AsRef<Path>,
{
    DirBuilder::new().create(path).await
}

pub async fn create_dir_all<P>(path: P) -> Result<()>
where
    P: AsRef<Path>,
{
    DirBuilder::new().recursive(true).create(path).await
}
