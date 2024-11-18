mod file;
mod helpers;
mod read;
mod write;

use helpers::*;

#[test]
fn create_dir() {
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
fn create_dir_recursive() {
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

#[test]
fn dir_metadata() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::create_dir(&name).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert!(stats.is_dir());
    });

    std::fs::remove_dir(&name).unwrap();
}
