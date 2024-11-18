use futures::AsyncWriteExt;
use inel::{io::AsyncWriteOwned, sys::StableBuffer};

use super::*;

#[test]
fn simple() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let file = inel::fs::File::create(name_clone).await.unwrap();
        let mut writer = inel::io::BufWriter::new(file);

        assert!(writer.buffer().is_some());
        assert!(writer.capacity().is_some());

        let buf = Box::new([b'a'; 256]);
        let res = writer.write_all(buf.clone().as_slice()).await;
        assert!(res.is_ok());

        assert!(writer.buffer().is_some());
        assert!(writer.capacity().is_some());

        let res = writer.flush().await;
        assert!(res.is_ok());

        assert!(writer.buffer().is_some());
        assert!(writer.capacity().is_some());

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(buf.as_slice(), data.as_bytes());

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn inner() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let buf = inel::block_on(async move {
        let file = inel::fs::File::create(name_clone).await.unwrap();
        let mut writer = inel::io::BufWriter::new(file);

        let buf = Box::new([b'a'; 256]);
        let (buf, res) = writer.inner_mut().write_owned(buf).await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 256);

        let res = writer.inner().sync().await;
        assert!(res.is_ok());

        let res = writer.close().await;
        assert!(res.is_ok());

        let _ = writer.into_inner();

        buf
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(buf.as_slice(), data.as_bytes());

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn lines() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let lines = 1000;
    let line = String::from("Hello World!\n");
    let expect = line.repeat(lines);

    inel::block_on(async move {
        let file = inel::fs::File::create(name_clone).await.unwrap();
        let mut writer = inel::io::BufWriter::new(file);

        for _ in 0..lines {
            let res = writer.write(line.as_bytes()).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap(), line.len());
        }

        let res = writer.flush().await;
        assert!(res.is_ok());
    });

    let data = std::fs::read_to_string(&name).unwrap();
    assert_eq!(data, expect);

    std::fs::remove_file(&name).unwrap();
}

mod fixed {
    use super::*;

    #[test]
    fn simple() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let buf = inel::block_on(async move {
            let file = inel::fs::File::create(name_clone).await.unwrap();
            let mut writer = inel::io::BufWriter::new(file).fix().unwrap();

            assert!(writer.buffer().is_some());
            assert!(writer.capacity().is_some());

            let buf = Box::new([b'a'; 256]);
            let res = writer.write_all(buf.clone().as_slice()).await;
            assert!(res.is_ok());

            assert!(writer.buffer().is_some());
            assert!(writer.capacity().is_some());

            let res = writer.flush().await;
            assert!(res.is_ok());

            assert!(writer.buffer().is_some());
            assert!(writer.capacity().is_some());

            buf
        });

        let data = std::fs::read_to_string(&name).unwrap();
        assert_eq!(buf.as_slice(), data.as_bytes());

        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn inner() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let buf = inel::block_on(async move {
            let file = inel::fs::File::create(name_clone).await.unwrap();
            let mut writer = inel::io::BufWriter::new(file).fix().unwrap();

            let buf = Box::new([b'a'; 256]);
            let (buf, res) = writer.inner_mut().write_owned(buf).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap(), 256);

            let res = writer.inner().sync().await;
            assert!(res.is_ok());

            let res = writer.close().await;
            assert!(res.is_ok());

            let _ = writer.into_inner();

            buf
        });

        let data = std::fs::read_to_string(&name).unwrap();
        assert_eq!(buf.as_slice(), data.as_bytes());

        std::fs::remove_file(&name).unwrap();
    }

    #[test]
    fn lines() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let lines = 1000;
        let line = String::from("Hello World!\n");
        let expect = line.repeat(lines);

        inel::block_on(async move {
            let file = inel::fs::File::create(name_clone).await.unwrap();
            let mut writer = inel::io::BufWriter::new(file).fix().unwrap();

            for _ in 0..lines {
                let res = writer.write(line.as_bytes()).await;

                assert!(res.is_ok());
                assert_eq!(res.unwrap(), line.len());
            }

            let res = writer.flush().await;
            assert!(res.is_ok());
        });

        let data = std::fs::read_to_string(&name).unwrap();
        assert_eq!(data, expect);

        std::fs::remove_file(&name).unwrap();
    }
}
