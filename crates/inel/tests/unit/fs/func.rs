use crate::helpers::*;

#[test]
fn write() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".to_string();
    let contents_clone = "Hello World!\n".to_string();

    inel::block_on(async move {
        assert!(inel::fs::write(name_clone, contents_clone).await.1.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));
    assert_eq!(std::fs::read_to_string(&name).unwrap(), contents);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn read() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".to_string();
    let contents_clone = "Hello World!\n".to_string();

    std::fs::write(&name, &contents).unwrap();

    inel::block_on(async move {
        let (buf, res) = inel::fs::read(name_clone, Box::new([0; 256])).await;
        assert!(res.is_ok());
        assert_eq!(
            String::from_utf8_lossy(&buf[0..res.unwrap()]),
            contents_clone
        );
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
