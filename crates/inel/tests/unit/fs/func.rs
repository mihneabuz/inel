use crate::helpers::*;

#[test]
fn write() {
    setup_tracing();
    let name = temp_file();
    let name_clone = name.clone();
    let contents = "Hello World!\n".to_string();
    let contents_clone = "Hello World!\n".to_string();

    inel::block_on(async move {
        assert!(inel::fs::write(name_clone, contents_clone).await.is_ok());
    });

    assert!(std::fs::exists(&name).is_ok_and(|exists| exists));
    assert_eq!(std::fs::read_to_string(&name).unwrap(), contents);

    std::fs::remove_file(&name).unwrap();
}
