use inel::io::AsyncReadOwned;

use crate::{setup_tracing, temp_file};

#[test]
fn simple() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let new = Box::new([0; 256]);
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let (new, res) = file.read_owned(new).await;

        assert!(res.is_ok_and(|read| read == 256));

        new
    });

    assert_eq!(new, old);

    std::fs::remove_file(&name).unwrap();
}

#[test]
fn offset() {
    setup_tracing();

    let name = temp_file();
    let name_clone = name.clone();

    let old = Box::new([b'a'; 256]);
    std::fs::write(&name, old.as_slice()).unwrap();

    let new = inel::block_on(async move {
        let new = Box::new([0; 256]);
        let mut file = inel::fs::File::open(name_clone).await.unwrap();

        let (new, res) = file.read_owned_at(128, new).await;

        assert!(res.is_ok_and(|read| read == 128));

        new
    });

    assert_eq!(&new.as_slice()[0..128], &[b'a'; 128]);
    assert_eq!(&new.as_slice()[128..], &[0; 128]);

    std::fs::remove_file(&name).unwrap();
}
