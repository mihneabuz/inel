use std::{ffi::CString, rc::Rc};

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

#[test]
fn from_boxed_slice_drops_every_item() {
    let (flags, items): (Vec<_>, Vec<_>) = (0..3).map(|_| flag()).unzip();
    let c: Cancellation = items.into_boxed_slice().into();
    assert!(flags.iter().all(|f| !dropped(f)));
    c.drop_raw();
    assert!(flags.iter().all(|f| dropped(f)));
}

#[test]
fn from_empty_boxed_slice() {
    let c: Cancellation = Box::<[u32]>::default().into();
    c.drop_raw();
}

#[test]
fn from_box_str() {
    let c: Cancellation = Box::<str>::from("hello").into();
    c.drop_raw();
}

#[test]
fn from_vec() {
    let mut v = Vec::<u8>::with_capacity(32);
    v.extend_from_slice(b"hello");
    let c: Cancellation = v.into();
    c.drop_raw();
}

#[test]
fn from_vec_wide_items() {
    // the capacity is counted in items, not bytes
    let mut v = Vec::<u64>::with_capacity(16);
    v.extend([1, 2, 3]);
    let c: Cancellation = v.into();
    c.drop_raw();
}

#[test]
fn from_empty_vec() {
    let c: Cancellation = Vec::<u32>::new().into();
    c.drop_raw();
}

#[test]
fn from_string() {
    let mut s = String::with_capacity(32);
    s.push_str("hello");
    let c: Cancellation = s.into();
    c.drop_raw();
}

#[test]
fn from_cstring() {
    let c: Cancellation = CString::new("hello").unwrap().into();
    c.drop_raw();
}

#[test]
fn from_rc_releases_one_reference() {
    let (f, tracked) = flag();
    let rc = Rc::new(tracked);
    let c: Cancellation = Rc::clone(&rc).into();
    assert_eq!(Rc::strong_count(&rc), 2);

    c.drop_raw();
    assert_eq!(Rc::strong_count(&rc), 1);
    assert!(!dropped(&f));

    drop(rc);
    assert!(dropped(&f));
}

#[test]
fn from_rc_last_reference_drops() {
    let (f, tracked) = flag();
    let c: Cancellation = Rc::new(tracked).into();
    assert!(!dropped(&f));
    c.drop_raw();
    assert!(dropped(&f));
}

#[test]
fn from_rc_slice_drops_every_item() {
    let (flags, items): (Vec<_>, Vec<_>) = (0..3).map(|_| flag()).unzip();
    let rc: Rc<[_]> = items.into();
    let c: Cancellation = Rc::clone(&rc).into();
    assert_eq!(Rc::strong_count(&rc), 2);

    c.drop_raw();
    assert_eq!(Rc::strong_count(&rc), 1);
    assert!(flags.iter().all(|f| !dropped(f)));

    drop(rc);
    assert!(flags.iter().all(|f| dropped(f)));
}

#[test]
fn from_rc_str() {
    let rc: Rc<str> = Rc::from("hello");
    let c: Cancellation = Rc::clone(&rc).into();
    assert_eq!(Rc::strong_count(&rc), 2);

    c.drop_raw();
    assert_eq!(Rc::strong_count(&rc), 1);
    assert_eq!(&*rc, "hello");
}

#[test]
fn from_option_rc() {
    let (f, tracked) = flag();
    let c: Cancellation = Some(Rc::new(tracked)).into();
    c.drop_raw();
    assert!(dropped(&f));

    let c: Cancellation = Option::<Rc<DropFlag>>::None.into();
    c.drop_raw();
}
