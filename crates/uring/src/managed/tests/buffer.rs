use crate::managed::IoBuf;

#[test]
fn view_bounds() {
    let v = b"hello world".as_slice().view(..);
    assert_eq!((v.begin(), v.end(), v.size()), (0, 11, 11));

    let v = b"hello world".as_slice().view(6..);
    assert_eq!((v.begin(), v.end()), (6, 11));

    let v = b"hello world".as_slice().view(..=4);
    assert_eq!((v.begin(), v.end()), (0, 5));

    let v = vec![0u8; 4].view(4..4);
    assert_eq!(v.size(), 0);
}

#[test]
fn view_uses_initialized_bytes() {
    // spare capacity is not part of the view
    let v = Vec::<u8>::with_capacity(64).view(..);
    assert_eq!(v.size(), 0);
}

#[test]
fn view_clamps_past_end() {
    let v = vec![0u8; 4].view(2..10);
    assert_eq!((v.begin(), v.end(), v.size()), (2, 4, 2));

    let v = vec![0u8; 4].view(6..);
    assert_eq!((v.begin(), v.end(), v.size()), (4, 4, 0));
}

#[test]
fn view_reversed_is_empty() {
    #[allow(clippy::reversed_empty_ranges)]
    let v = vec![0u8; 4].view(3..2);
    assert_eq!((v.begin(), v.end(), v.size()), (3, 3, 0));
}
