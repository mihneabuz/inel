use crate::helpers::*;

#[test]
fn write_buf() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".to_string();
    let contents_clone = contents.clone();

    inel::block_on(async move {
        let (_, res) = inel::fs::write_buf(name_clone, contents_clone).await;
        assert!(res.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));
    assert_eq!(std::fs::read_to_string(&name).unwrap(), contents);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn write() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".repeat(2);
    let contents_clone = contents.to_string();

    inel::block_on(async move {
        assert!(inel::fs::write(name_clone, contents_clone).await.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));
    assert_eq!(std::fs::read_to_string(&name).unwrap(), contents);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn read_buf() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".to_string();
    let contents_clone = contents.clone();

    std::fs::write(&name, &contents).unwrap();

    inel::block_on(async move {
        let (buf, res) = inel::fs::read_buf(name_clone, Box::new([0; 256])).await;
        assert!(res.is_ok());
        assert_eq!(
            String::from_utf8_lossy(&buf[0..res.unwrap()]),
            contents_clone
        );
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn read() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".repeat(10000).into_bytes();

    std::fs::write(&name, &contents).unwrap();

    inel::block_on(async move {
        let read = inel::fs::read(name_clone).await.unwrap();
        assert_eq!(contents, read);
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn read_to_string() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".repeat(10000);

    std::fs::write(&name, &contents).unwrap();

    inel::block_on(async move {
        let read = inel::fs::read_to_string(name_clone).await.unwrap();
        assert_eq!(contents, read);
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn exists() {
    setup_tracing();

    let good = temp_file();
    let good_clone = good.clone();
    let bad = good.join("nope");

    std::fs::write(&good, "file").unwrap();

    inel::block_on(async move {
        assert!(inel::fs::exists(good).await);
        assert!(!inel::fs::exists(bad).await);
    });

    std::fs::remove_file(&good_clone).unwrap();
}

#[test]
fn error() {
    setup_tracing();
    let name = temp_file();

    inel::block_on(async move {
        let (_, res) = inel::fs::read_buf(name, Box::new([0; 256])).await;
        assert!(res.is_err());
    });
}

#[test]
fn metadata() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name_clone, "hello file metadata!").unwrap();

    inel::block_on(async move {
        let meta = inel::fs::metadata(name).await.unwrap();

        assert_eq!(meta.is_file(), true);
        assert_eq!(meta.is_dir(), false);
        assert_eq!(meta.is_symlink(), false);

        assert!(meta.user_id() > 0);
        assert!(meta.group_id() > 0);

        meta.accessed().unwrap();
        meta.created().unwrap();
        meta.modified().unwrap();
    });

    std::fs::remove_file(&name_clone).unwrap();
}
