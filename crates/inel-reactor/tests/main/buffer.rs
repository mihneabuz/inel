use std::io::Write;

use inel_reactor::buffer::{StableBuffer, StableBufferMut};

#[test]
fn staticref() {
    let s: &'static str = "Hello World!\n";
    let b: &'static [u8] = b"Hello World!\n";

    assert_eq!(StableBuffer::size(&s), 13);
    assert_eq!(StableBuffer::size(&b), 13);

    assert_eq!(s.stable_slice(), b);
    assert_eq!(b.stable_slice(), s.as_bytes());
}

#[test]
fn string() {
    let mut s = String::with_capacity(256);
    s.push_str("Hello World!\n");

    assert_eq!(StableBuffer::size(&s), 13);

    s.stable_mut_slice().write(b"Overwritten!\n").unwrap();

    assert_eq!(s.stable_slice(), b"Overwritten!\n");
    assert_eq!(s, String::from("Overwritten!\n"));
}

#[test]
fn vec() {
    let mut v = Vec::with_capacity(256);
    v.extend_from_slice(b"Hello World!\n");

    assert_eq!(StableBuffer::size(&v), 13);

    v.as_mut_slice().write(b"Overwritten!\n").unwrap();

    assert_eq!(v.as_slice(), b"Overwritten!\n");
    assert_eq!(
        String::from_utf8(v).unwrap(),
        String::from("Overwritten!\n")
    );
}

#[test]
fn boxed() {
    let mut b = Box::new([0; 256]);

    assert_eq!(StableBuffer::size(&b), 256);

    b.stable_mut_slice().write(b"Overwritten!\n").unwrap();

    assert_eq!(&b.stable_slice()[0..13], b"Overwritten!\n");
    assert_eq!(&b.stable_slice()[13..], [0; 256 - 13]);
}

#[test]
fn null() {
    let mut b = None;
    assert_eq!(StableBuffer::size(&b), 0);
    assert_eq!(StableBuffer::stable_slice(&b), &[]);
    assert_eq!(StableBufferMut::stable_mut_slice(&mut b), &mut []);
}

mod view {
    use std::ops::RangeBounds;

    use inel_reactor::buffer::{StableBuffer, StableBufferMut, View};

    #[test]
    fn included() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = View::new(buf, 10..=20);
        assert_eq!(view.stable_slice(), &[b'A'; 11]);
    }

    #[test]
    fn excluded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..20].copy_from_slice(&[b'A'; 10]);

        let view = View::new(buf, 10..20);
        assert_eq!(view.stable_slice(), &[b'A'; 10]);
    }

    #[test]
    fn unbounded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..].copy_from_slice(&[b'A'; 256]);

        let view = View::new(buf, ..);
        assert_eq!(view.stable_slice(), &[b'A'; 256]);
    }

    #[test]
    fn mixed() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..10].copy_from_slice(&[b'A'; 10]);

        let view = View::new(buf, ..10);
        assert_eq!(view.stable_slice(), &[b'A'; 10]);

        let mut buf = view.unview();
        buf[20..].copy_from_slice(&[b'A'; 236]);

        let view = View::new(buf, 20..);
        assert_eq!(view.stable_slice(), &[b'A'; 236]);
    }

    #[test]
    fn mutable() {
        let buf = Box::new([b'_'; 256]);
        let mut view = View::new(buf, 10..20);
        view.stable_mut_slice().copy_from_slice(&[b'A'; 10]);

        let buf = view.unview();
        assert_eq!(&buf.stable_slice()[10..20], &[b'A'; 10]);
    }

    #[test]
    fn custom() {
        let mut buf = Box::new([b'_'; 256]);
        buf[11..20].copy_from_slice(&[b'A'; 9]);

        struct VeryExclusiveRange {
            inner: (usize, usize),
        }

        impl RangeBounds<usize> for VeryExclusiveRange {
            fn start_bound(&self) -> std::ops::Bound<&usize> {
                std::ops::Bound::Excluded(&self.inner.0)
            }

            fn end_bound(&self) -> std::ops::Bound<&usize> {
                std::ops::Bound::Excluded(&self.inner.1)
            }
        }

        let view = View::new(buf, VeryExclusiveRange { inner: (10, 20) });
        assert_eq!(view.stable_slice(), &[b'A'; 9]);
    }

    #[test]
    fn string() {
        let mut view = View::new(String::from("Hello World!"), 0..5);
        view.stable_mut_slice().copy_from_slice(b"Never");

        assert_eq!(view.inner().as_str(), "Never World!");
    }

    #[test]
    fn capacity() {
        let buf = Box::new([b'_'; 256]);
        let view = View::new(buf, 64..128 + 64);

        assert_eq!(view.size(), 128);
    }

    #[test]
    fn inner() {
        let buf = vec![b'_'; 256];
        let mut view = View::new(buf, 64..128 + 64);

        assert_eq!(view.inner().len(), 256);
        assert_eq!(view.inner_mut().drain(..).count(), 256);

        view.inner_mut().truncate(0);
        assert_eq!(view.inner().len(), 0);
    }

    #[test]
    fn cursor() {
        let buf = vec![b'_'; 256];
        let mut cursor = View::new(buf, ..10);

        assert_eq!(cursor.stable_slice(), &[b'_'; 10]);
        assert_eq!(cursor.is_empty(), false);

        cursor.set_pos(5);

        assert_eq!(cursor.stable_slice(), &[b'_'; 5]);
        assert_eq!(cursor.is_empty(), false);

        cursor.set_pos(0);

        assert_eq!(cursor.stable_slice(), &[]);
        assert_eq!(cursor.is_empty(), true);

        cursor.set_pos(10);
        *cursor.inner_mut() = vec![b'_'; 2];

        assert_eq!(cursor.stable_slice(), &[b'_'; 2]);
    }

    #[test]
    fn double_cursor() {
        let buf = vec![b'_'; 256];
        let mut cursor = View::new(buf, 10..20);

        assert_eq!(cursor.stable_slice(), &[b'_'; 10]);
        assert_eq!(cursor.is_empty(), false);

        cursor.consume(5);

        assert_eq!(cursor.stable_slice(), &[b'_'; 5]);
        assert_eq!(cursor.is_empty(), false);

        cursor.consume(5);

        assert_eq!(cursor.stable_slice(), &[]);
        assert_eq!(cursor.is_empty(), true);

        let buf = None;
        let cursor = View::new(buf, 10..20);

        assert_eq!(cursor.stable_slice(), &[]);
    }
}

mod fixed {
    use inel_reactor::buffer::{Fixed, StableBuffer, StableBufferMut, View};

    use crate::helpers::runtime;

    #[test]
    fn simple() {
        let (reactor, _) = runtime();

        let buf = Box::new([b'_'; 1024]);
        let res = Fixed::register(buf, reactor);

        assert!(res.is_ok());

        let mut fixed = res.unwrap();
        fixed.stable_mut_slice().fill_with(|| b'a');

        assert_eq!(fixed.stable_slice(), &[b'a'; 1024]);
    }

    #[test]
    fn multiple() {
        let (reactor, _) = runtime();

        let fixed1 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let mut fixed2 = Fixed::new(400, reactor.clone()).unwrap();
        fixed2.stable_mut_slice().fill(b'_');

        let fixed3 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let mut fixed4 = Fixed::new(400, reactor.clone()).unwrap();
        fixed4.stable_mut_slice().fill(b'_');

        assert_eq!(fixed1.stable_slice(), fixed2.stable_slice());
        assert_eq!(fixed3.stable_slice(), fixed4.stable_slice());
    }

    #[test]
    fn drop() {
        let (reactor, _) = runtime();
        const LEN: usize = 64;

        let fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        let fixed2 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();

        {
            let _fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
            let _fixed2 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        }

        let fixed3 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();

        {
            let _fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
            let _fixed2 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        }

        let fixed4 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();

        assert_eq!(fixed1.stable_slice(), fixed2.stable_slice());
        assert_eq!(fixed3.stable_slice(), fixed4.stable_slice());
    }

    #[test]
    fn view() {
        let (reactor, _) = runtime();

        let mut buf = Fixed::register(Box::new([b'_'; 256]), reactor).unwrap();
        buf.stable_mut_slice()[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = View::new(buf, 10..=20);
        assert_eq!(view.stable_slice(), &[b'A'; 11]);
    }

    #[test]
    fn error() {
        let (reactor, _) = runtime();
        let b = Box::new([b'_'; 256]);

        let bufs = (0..reactor.resources())
            .map(|_| Fixed::register(b.clone(), reactor.clone()).unwrap())
            .collect::<Vec<_>>();

        assert!(Fixed::register(b.clone(), reactor.clone()).is_err());

        std::mem::drop(bufs);

        assert!(Fixed::register(b.clone(), reactor.clone()).is_ok());
    }
}
