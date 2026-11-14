use crate::managed::IoBuf;

/// `(offset, len)` of the bytes `buf` exposes for writes, relative to `base`.
fn window<B: IoBuf>(buf: B, base: *const u8) -> (usize, usize) {
    let raw = buf.into_raw();
    let (ptr, len) = B::raw_buf(&raw);
    let offset = ptr as usize - base as usize;
    // SAFETY: `raw` came from `into_raw` and nothing was submitted
    drop(unsafe { B::from_raw(raw) });
    (offset, len)
}

#[test]
fn view_bounds() {
    let data = b"hello world".as_slice();
    let base = data.as_ptr();

    let v = data.view(..);
    assert_eq!((v.begin(), v.end()), (0, None));
    assert_eq!(window(v, base), (0, 11));

    let v = data.view(6..);
    assert_eq!((v.begin(), v.end()), (6, None));
    assert_eq!(window(v, base), (6, 5));

    let v = data.view(..=4);
    assert_eq!((v.begin(), v.end()), (0, Some(5)));
    assert_eq!(window(v, base), (0, 5));

    let v = vec![0u8; 4].view(4..4);
    assert_eq!(window(v, std::ptr::null()).1, 0);
}

#[test]
fn view_uses_initialized_bytes() {
    // spare capacity is not part of the view
    let v = Vec::<u8>::with_capacity(64).view(..);
    assert_eq!(window(v, std::ptr::null()).1, 0);
}

#[test]
fn view_clamps_past_end() {
    let vec = vec![0u8; 4];
    let base = vec.as_ptr();
    assert_eq!(window(vec.view(2..10), base), (2, 2));

    let vec = vec![0u8; 4];
    let base = vec.as_ptr();
    assert_eq!(window(vec.view(6..), base), (4, 0));
}

#[test]
fn view_reversed_is_empty() {
    let vec = vec![0u8; 4];
    let base = vec.as_ptr();
    #[allow(clippy::reversed_empty_ranges)]
    let v = vec.view(3..2);
    assert_eq!(window(v, base), (3, 0));
}
