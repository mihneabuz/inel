use crate::helpers::{setup_tracing, temp_file};

#[test]
fn create() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        let res = inel::fs::DirBuilder::new().create(name_clone).await;
        assert!(res.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_dir(&name).unwrap();
}

#[test]
fn error() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, "Hello World!\n").unwrap();

    inel::block_on(async move {
        let res = inel::fs::DirBuilder::default().create(name_clone).await;
        assert!(res.is_err());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn recursive() {
    setup_tracing();

    let base = temp_file();
    let name = base.join("hello").join("world").join("!");
    let name_clone = name.clone();

    inel::block_on(async move {
        let res = inel::fs::DirBuilder::new()
            .recursive(true)
            .create(name_clone)
            .await;

        assert!(res.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_dir_all(&base).unwrap();
}
