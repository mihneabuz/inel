use std::{io::Write, os::fd::FromRawFd};

use inel::{
    buffer::{StableBuffer, StableBufferExt, StableBufferMut},
    group::{BufferShareGroup, ReadBufferSet, WriteBufferSet},
};

use crate::helpers::temp_file;

#[test]
fn read() {
    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, &[b'a'; 4096]).unwrap();

    inel::block_on(async move {
        let mut file = inel::fs::File::open(name_clone).await.unwrap();
        let group = ReadBufferSet::empty().unwrap();

        let buffer = vec![0; 256].into_boxed_slice();
        group.insert(buffer);

        let (buf, read) = group.read(&mut file).await.unwrap();
        assert_eq!(read, 256);
        assert_eq!(&buf[0..256], &[b'a'; 256]);
        group.insert(buf);
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn write() {
    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        let mut file = inel::fs::File::create(name_clone).await.unwrap();
        let group = WriteBufferSet::empty();

        let buffer = vec![b'a'; 256].into_boxed_slice();
        let wrote = group.write(&mut file, buffer.view(..)).await.unwrap();
        assert_eq!(wrote, 256);

        let buffer = group.get();
        assert_eq!(buffer.as_slice(), &[b'a'; 256]);

        group.insert(buffer);
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn combo() {
    let write_name = temp_file();
    let write_name_clone = write_name.clone();

    let read_name = temp_file();
    let read_name_clone = read_name.clone();

    std::fs::write(&read_name, &[b'a'; 4096]).unwrap();

    inel::block_on(async move {
        let mut write_file = inel::fs::File::create(write_name_clone).await.unwrap();
        let mut read_file = inel::fs::File::open(read_name_clone).await.unwrap();

        let group = BufferShareGroup::options()
            .buffer_capacity(1024)
            .initial_read_buffers(1)
            .initial_write_buffers(1)
            .build()
            .await
            .unwrap();

        let mut buffer = group.get_write_buffer();
        buffer.as_mut_slice().write(&[b'a'; 256]).unwrap();
        let wrote = group
            .write(&mut write_file, buffer.view(..256))
            .await
            .unwrap();
        assert_eq!(wrote, 256);

        let (buf, read) = group.read(&mut read_file).await.unwrap();
        assert_eq!(&buf[..read], &vec![b'a'; read]);
        group.insert_read_buffer(buf);
    });

    std::fs::remove_file(&write_name).unwrap();
}

#[test]
fn error() {
    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, &[b'a'; 4096]).unwrap();

    inel::block_on(async move {
        let mut file = inel::fs::File::open(name_clone).await.unwrap();
        let mut bad_file = unsafe { inel::fs::File::from_raw_fd(1337) };

        let group = BufferShareGroup::new().await.unwrap();

        let buffer = group.get_write_buffer();
        let res = group.write(&mut file, buffer.view(..)).await;
        assert!(res.is_err());

        let res = group.read(&mut bad_file).await;
        assert!(res.is_err());
    });

    std::fs::remove_file(&name).unwrap();
}
