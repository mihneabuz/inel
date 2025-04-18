use std::io::Write;

use inel_reactor::buffer::StableBuffer;

#[test]
fn string() {
    let mut s = String::with_capacity(256);
    s.push_str("Hello World!\n");

    assert_eq!(StableBuffer::size(&s), 13);

    s.as_mut_slice().write("Overwritten!\n".as_bytes()).unwrap();

    assert_eq!(s.as_slice(), "Overwritten!\n".as_bytes());
    assert_eq!(s, String::from("Overwritten!\n"));
}

#[test]
fn vec() {
    let mut v = Vec::with_capacity(256);
    v.extend_from_slice("Hello World!\n".as_bytes());

    assert_eq!(StableBuffer::size(&v), 13);

    v.as_mut_slice().write("Overwritten!\n".as_bytes()).unwrap();

    assert_eq!(v.as_slice(), "Overwritten!\n".as_bytes());
    assert_eq!(
        String::from_utf8(v).unwrap(),
        String::from("Overwritten!\n")
    );
}

#[test]
fn boxed() {
    let mut b = Box::new([0; 256]);

    assert_eq!(StableBuffer::size(&b), 256);

    b.as_mut_slice().write("Overwritten!\n".as_bytes()).unwrap();

    assert_eq!(&b.as_slice()[0..13], "Overwritten!\n".as_bytes());
    assert_eq!(&b.as_slice()[13..], [0; 256 - 13]);
}

mod view {
    use std::ops::RangeBounds;

    use inel_reactor::buffer::{StableBuffer, View};

    #[test]
    fn included() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = View::new(buf, 10..=20);
        assert_eq!(view.as_slice(), &[b'A'; 11]);
    }

    #[test]
    fn excluded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..20].copy_from_slice(&[b'A'; 10]);

        let view = View::new(buf, 10..20);
        assert_eq!(view.as_slice(), &[b'A'; 10]);
    }

    #[test]
    fn unbounded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..].copy_from_slice(&[b'A'; 256]);

        let view = View::new(buf, ..);
        assert_eq!(view.as_slice(), &[b'A'; 256]);
    }

    #[test]
    fn mixed() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..10].copy_from_slice(&[b'A'; 10]);

        let view = View::new(buf, ..10);
        assert_eq!(view.as_slice(), &[b'A'; 10]);

        let mut buf = view.unview();
        buf[20..].copy_from_slice(&[b'A'; 236]);

        let view = View::new(buf, 20..);
        assert_eq!(view.as_slice(), &[b'A'; 236]);
    }

    #[test]
    fn mutable() {
        let buf = Box::new([b'_'; 256]);
        let mut view = View::new(buf, 10..20);
        view.as_mut_slice().copy_from_slice(&[b'A'; 10]);

        let buf = view.unview();
        assert_eq!(&buf.as_slice()[10..20], &[b'A'; 10]);
    }

    #[test]
    fn as_ref() {
        let buf = Box::new([b'_'; 256]);
        let view = View::new(buf, ..);
        assert_eq!(view.as_ref(), &[b'_'; 256]);

        let buf = view.unview();
        let mut view = View::new(buf, ..);
        assert_eq!(view.as_mut(), &[b'_'; 256]);
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
        assert_eq!(view.as_slice(), &[b'A'; 9]);
    }

    #[test]
    fn string() {
        let mut view = View::new(String::from("Hello World!"), 0..5);
        view.as_mut_slice().copy_from_slice(b"Never");

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
}

mod fixed {
    use std::ops::Deref;

    use inel_reactor::buffer::{Fixed, StableBuffer, View};

    use crate::helpers::runtime;

    #[test]
    fn simple() {
        let (reactor, _) = runtime();

        let buf = Box::new([b'_'; 1024]);
        let res = Fixed::register(buf, reactor);

        assert!(res.is_ok());

        let mut fixed = res.unwrap();
        fixed.fill_with(|| b'a');

        assert_eq!(fixed.deref(), &[b'a'; 1024]);
    }

    #[test]
    fn multiple() {
        let (reactor, _) = runtime();

        let fixed1 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let mut fixed2 = Fixed::new(400, reactor.clone()).unwrap();
        fixed2.fill(b'_');

        let fixed3 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let mut fixed4 = Fixed::new(400, reactor.clone()).unwrap();
        fixed4.fill(b'_');

        assert_eq!(fixed1.as_slice(), fixed2.as_slice());
        assert_eq!(fixed3.as_slice(), fixed4.as_slice());
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

        assert_eq!(fixed1.as_slice(), fixed2.as_slice());
        assert_eq!(fixed3.as_slice(), fixed4.as_slice());
    }

    #[test]
    fn view() {
        let (reactor, _) = runtime();

        let mut buf = Fixed::register(Box::new([b'_'; 256]), reactor).unwrap();
        buf[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = View::new(buf, 10..=20);
        assert_eq!(view.as_slice(), &[b'A'; 11]);
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
