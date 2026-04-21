use rustix::ffi::CString;

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
fn from_vec() {
    let (f1, t1) = flag();
    let (f2, t2) = flag();
    let (f3, t3) = flag();
    let c: Cancellation = vec![t1, t2, t3].into();
    assert!(!dropped(&f1));
    c.drop_raw();
    assert!(dropped(&f1));
    assert!(dropped(&f2));
    assert!(dropped(&f3));
}

#[test]
fn from_string() {
    let c: Cancellation = String::from("hello world").into();
    c.drop_raw();
}

#[test]
fn from_cstring() {
    let c: Cancellation = CString::new("hello").unwrap().into();
    c.drop_raw();
}

#[test]
fn from_static_ref_is_noop() {
    let c: Cancellation = b"static bytes".as_slice().into();
    c.drop_raw();
}

#[test]
fn from_static_str_is_noop() {
    let c: Cancellation = "static str".into();
    c.drop_raw();
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

#[test]
fn from_boxed_slice() {
    let (f1, t1) = flag();
    let (f2, t2) = flag();
    let v: Box<[DropFlag]> = vec![t1, t2].into_boxed_slice();
    let c: Cancellation = v.into();
    assert!(!dropped(&f1));
    c.drop_raw();
    assert!(dropped(&f1));
    assert!(dropped(&f2));
}
