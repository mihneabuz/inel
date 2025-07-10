use inel_reactor::cancellation::Cancellation;

#[test]
fn boxes() {
    let p1 = Box::into_raw(Box::new(10));
    let b1 = unsafe { Box::from_raw(p1) };
    let c1: Cancellation = b1.into();

    let p2 = Box::into_raw(vec![20; 20].into_boxed_slice());
    let b2 = unsafe { Box::from_raw(p2) };
    let c2: Cancellation = b2.into();

    let p3 = Box::into_raw(Box::new(30));
    let b3 = Some(unsafe { Box::from_raw(p3) });
    let c3: Cancellation = b3.into();

    let p4 = Box::into_raw(vec![40; 40].into_boxed_slice());
    let b4 = Some(unsafe { Box::from_raw(p4) });
    let c4: Cancellation = b4.into();

    assert_eq!(unsafe { p1.read() }, 10);
    c1.drop_raw();

    assert_eq!(unsafe { (p2 as *mut i32).read() }, 20);
    c2.drop_raw();

    assert_eq!(unsafe { p3.read() }, 30);
    c3.drop_raw();

    assert_eq!(unsafe { (p4 as *mut i32).read() }, 40);
    c4.drop_raw();
}

#[test]
fn vec() {
    let mut vec = vec![10; 10];
    let vec_parts = (vec.as_mut_ptr(), vec.len(), vec.capacity());
    let vec_cancel: Cancellation = vec.into();

    let mut str = String::from("Hello World!");
    let str_parts = (str.as_mut_ptr(), str.len(), str.capacity());
    let str_cancel: Cancellation = str.into();

    let vec_fake = unsafe { Vec::from_raw_parts(vec_parts.0, vec_parts.1, vec_parts.2) };
    assert_eq!(&vec_fake, &[10; 10]);
    std::mem::forget(vec_fake);
    vec_cancel.drop_raw();

    let str_fake = unsafe { String::from_raw_parts(str_parts.0, str_parts.1, str_parts.2) };
    assert_eq!(&str_fake, "Hello World!");
    std::mem::forget(str_fake);
    str_cancel.drop_raw();
}

#[test]
fn combine() {
    let p1 = Box::into_raw(Box::new(10));
    let b1 = unsafe { Box::from_raw(p1) };
    let c1: Cancellation = b1.into();

    let s = "Hello World!\n";
    let c2: Cancellation = s.into();
    let c3: Cancellation = s.as_bytes().into();
    let c4: Cancellation = None::<Box<i32>>.into();

    let p5 = Box::into_raw(Box::new(10));
    let b5 = unsafe { Box::from_raw(p1) };
    let c5: Cancellation = b5.into();

    let cancel = Cancellation::combine(vec![c1, c2, c3, c4, c5]);

    assert_eq!(unsafe { p1.read() }, 10);
    assert_eq!(unsafe { p5.read() }, 10);

    cancel.drop_raw();
}
