use futures::AsyncReadExt;
use inel::io::AsyncReadOwned;

use super::*;

#[test]
fn simple() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 4096]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let mut reader = inel::io::BufReader::new(file);

        assert!(reader.buffer().is_some());
        assert!(reader.capacity().is_some());

        let mut new = Box::new([b'_'; 256]);
        let res = reader.read(new.as_mut_slice()).await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 256);

        assert!(reader.buffer().is_some());
        assert!(reader.capacity().is_some());

        new
    });

    assert_eq!(new.as_slice(), &old[0..256]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn inner() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 4096]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let mut reader = inel::io::BufReader::new(file);

        let new = Box::new([0; 256]);
        let (new, res) = reader.inner_mut().read_owned(new).await;

        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 256);

        let res = reader.inner().sync_data().await;
        assert!(res.is_ok());

        let _ = reader.into_inner();

        new
    });

    assert_eq!(new.as_slice(), &old[0..256]);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn lines() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let lines = 1000;
    let content = String::from("Hello World!\n").repeat(lines);
    std::fs::write(&name, &content).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let reader = inel::io::BufReader::new(file);

        let mut counter = 0;
        reader
            .lines()
            .map(|line| {
                counter += 1;

                assert!(line.is_ok());
                assert_eq!(line.as_ref().unwrap(), "Hello World!");

                line
            })
            .for_each(|_| async {})
            .await;

        assert_eq!(counter, lines);
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn error() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, [b'a'; 4096]).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::options()
            .readable(false)
            .writable(true)
            .open(&name_clone)
            .await
            .unwrap();

        let mut reader = inel::io::BufReader::new(file);

        let mut buf = String::new();
        let res = reader.read_to_string(&mut buf).await;
        assert!(res.is_err());
    });

    std::fs::remove_file(&name).unwrap();
}

mod fixed {
    use super::*;

    #[test]
    fn simple() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let old = Box::new([b'a'; 4096]);
        std::fs::write(&name, old.as_slice()).unwrap();

        let new = inel::block_on(async move {
            let file = inel::fs::File::open(name_clone).await.unwrap();
            let mut reader = inel::io::BufReader::new(file).fix().unwrap();

            assert!(reader.buffer().is_some());
            assert!(reader.capacity().is_some());

            let mut new = Box::new([b'_'; 256]);
            let res = reader.read(new.as_mut_slice()).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap(), 256);

            assert!(reader.buffer().is_some());
            assert!(reader.capacity().is_some());

            new
        });

        assert_eq!(new.as_slice(), &old[0..256]);

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn inner() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let old = Box::new([b'a'; 4096]);
        std::fs::write(&name, old.as_slice()).unwrap();

        let new = inel::block_on(async move {
            let file = inel::fs::File::open(name_clone).await.unwrap();
            let mut reader = inel::io::BufReader::new(file).fix().unwrap();

            let new = Box::new([0; 256]);
            let (new, res) = reader.inner_mut().read_owned(new).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap(), 256);

            let res = reader.inner().sync_data().await;
            assert!(res.is_ok());

            let _ = reader.into_inner();

            new
        });

        assert_eq!(new.as_slice(), &old[0..256]);

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn lines() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let lines = 1000;
        let content = String::from("Hello World!\n").repeat(lines);
        std::fs::write(&name, &content).unwrap();

        inel::block_on(async move {
            let file = inel::fs::File::open(name_clone).await.unwrap();
            let reader = inel::io::BufReader::new(file).fix().unwrap();

            let mut counter = 0;
            reader
                .lines()
                .map(|line| {
                    counter += 1;

                    assert!(line.is_ok());
                    assert_eq!(line.as_ref().unwrap(), "Hello World!");

                    line
                })
                .for_each(|_| async {})
                .await;

            assert_eq!(counter, lines);
        });

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn error() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        std::fs::write(&name, [b'a'; 128_000]).unwrap();

        inel::block_on(async move {
            let file = inel::fs::File::options()
                .readable(false)
                .writable(true)
                .open(&name_clone)
                .await
                .unwrap();

            let mut reader = inel::io::BufReader::new(file).fix().unwrap();

            let mut buf = String::new();
            let res = reader.read_to_string(&mut buf).await;
            assert!(res.is_err());
        });

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }
}

mod group {
    use inel::io::ReadBuffers;

    use super::*;

    #[test]
    fn simple() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let old = Box::new([b'a'; 4096]);
        std::fs::write(&name, old.as_slice()).unwrap();

        let new = inel::block_on(async move {
            let group = ReadBuffers::new(2048, 4).await.unwrap();

            let file = inel::fs::File::open(name_clone).await.unwrap();
            let mut reader = group.provide_to(file);

            let mut new = Box::new([b'_'; 256]);
            let res = reader.read(new.as_mut_slice()).await;

            assert!(res.is_ok());
            assert_eq!(res.unwrap(), 256);

            new
        });

        assert_eq!(new.as_slice(), &old[0..256]);

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn lines() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        let lines = 1000;
        let content = String::from("Hello World!\n").repeat(lines);
        std::fs::write(&name, &content).unwrap();

        inel::block_on(async move {
            let group = ReadBuffers::new(256, 4).await.unwrap();

            let file = inel::fs::File::open(name_clone).await.unwrap();
            let reader = group.provide_to(file);

            let mut counter = 0;
            reader
                .lines()
                .map(|line| {
                    counter += 1;

                    assert!(line.is_ok());
                    assert_eq!(line.as_ref().unwrap(), "Hello World!");

                    line
                })
                .for_each(|_| async {})
                .await;

            assert_eq!(counter, lines);
        });

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }

    #[test]
    fn error() {
        setup_tracing();

        let name = temp_file();
        let name_clone = name.clone();

        std::fs::write(&name, [b'a'; 128_000]).unwrap();

        inel::block_on(async move {
            let group = ReadBuffers::new(2048, 4).await.unwrap();

            let file = inel::fs::File::options()
                .readable(false)
                .writable(true)
                .open(&name_clone)
                .await
                .unwrap();

            let mut reader = group.provide_to(file);

            let mut buf = String::new();
            let res = reader.read_to_string(&mut buf).await;
            assert!(res.is_err());
        });

        std::fs::remove_file(&name).unwrap();
        assert!(inel::is_done());
    }
}
