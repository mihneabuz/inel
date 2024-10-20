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

    assert!(std::fs::exists(name).is_ok_and(|exists| exists));
}

#[test]
fn open_file() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    std::fs::File::create(name).unwrap();

    inel::block_on(async move {
        assert!(inel::fs::File::open(name_clone).await.is_ok());
    });
}
