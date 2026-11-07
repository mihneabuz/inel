use crate::managed::{
    cancellation::Cancellation,
    tests::{DropFlag, dropped, flag},
};

#[test]
fn empty() {
    let c = Cancellation::empty();
    c.drop_raw();
}

#[test]
fn from_box() {
    let (f, tracked) = flag();
    let c: Cancellation = Box::new(tracked).into();
    assert!(!dropped(&f));
    c.drop_raw();
    assert!(dropped(&f));
}

#[test]
fn from_option_some() {
    let (f, tracked) = flag();
    let c: Cancellation = Some(Box::new(tracked)).into();
    assert!(!dropped(&f));
    c.drop_raw();
    assert!(dropped(&f));
}

#[test]
fn from_option_none() {
    let c: Cancellation = Option::<Box<DropFlag>>::None.into();
    c.drop_raw();
}

#[test]
fn drop_by_ref() {
    let (f, tracked) = flag();
    let c: Cancellation = Box::new(tracked).into();
    unsafe { c.drop_by_ref() };
    assert!(dropped(&f));
}
