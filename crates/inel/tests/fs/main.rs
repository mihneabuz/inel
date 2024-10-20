mod helpers;

use helpers::*;

#[test]
fn create_file() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    inel::block_on(async move {
        assert!(inel::fs::File::create(name_clone).await.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn open_file() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::File::create(&name).unwrap();

    inel::block_on(async move {
        assert!(inel::fs::File::open(name_clone).await.is_ok());
    });

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn file_metadata() {
    setup_tracing();

    let data = "Hello world!\n";
    let name = temp_file();
    let name_clone = name.clone();

    std::fs::write(&name, data.as_bytes()).unwrap();

    inel::block_on(async move {
        let file = inel::fs::File::open(name_clone).await.unwrap();
        let stats = file.metadata().await.unwrap();

        assert!(stats.is_file());
        assert_eq!(stats.len() as usize, data.len());
    });

    std::fs::remove_file(&name).unwrap();
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
