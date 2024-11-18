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
}

mod fixed {
    use inel_reactor::buffer::{Fixed, StableBuffer, View};

    use crate::helpers::runtime;

    #[test]
    fn simple() {
        let (reactor, _) = runtime();

        let buf = Box::new([b'_'; 1024]);
        let res = Fixed::register(buf, reactor);

        assert!(res.is_ok());

        let mut fixed = res.unwrap();
        fixed.inner_mut().fill_with(|| b'a');

        assert_eq!(fixed.inner(), &Box::new([b'a'; 1024]));
    }

    #[test]
    fn double() {
        let (reactor, _) = runtime();

        let buf = Box::new([b'_'; 1024]);
        let res1 = Fixed::register(buf, reactor.clone());
        let fixed1 = res1.unwrap();
        let res2 = Fixed::register(fixed1, reactor.clone());
        let mut fixed2 = res2.unwrap();

        fixed2.inner_mut().inner_mut().fill_with(|| b'a');

        assert_eq!(fixed2.inner().inner(), &Box::new([b'a'; 1024]));
    }

    #[test]
    fn multiple() {
        let (reactor, _) = runtime();

        let fixed1 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let fixed2 = Fixed::register(Vec::from([b'_'; 400]), reactor.clone()).unwrap();
        let fixed3 = Fixed::register(Box::new([b'_'; 400]), reactor.clone()).unwrap();
        let fixed4 = Fixed::register(Vec::from([b'_'; 400]), reactor.clone()).unwrap();

        assert_eq!(fixed1.inner().as_slice(), fixed2.inner().as_slice());
        assert_eq!(fixed3.inner().as_slice(), fixed4.inner().as_slice());
    }

    #[test]
    fn drop() {
        let (reactor, _) = runtime();
        const LEN: usize = 64;

        let fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        let fixed2 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();

        {
            let _fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
            let _fixed2 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();
        }

        let fixed3 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();

        {
            let _fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
            let _fixed2 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();
        }

        let fixed4 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();

        assert_eq!(fixed1.inner().as_slice(), fixed2.inner().as_slice());
        assert_eq!(fixed3.inner().as_slice(), fixed4.inner().as_slice());
    }

    #[test]
    fn unwrap() {
        let (reactor, _) = runtime();
        const LEN: usize = 80;

        let wrapped1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        let wrapped2 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();

        let buf1 = wrapped1.into_inner();
        let buf2 = wrapped2.into_inner();

        let fixed1 = Fixed::register(Box::new([b'_'; LEN]), reactor.clone()).unwrap();
        let fixed2 = Fixed::register(Vec::from([b'_'; LEN]), reactor.clone()).unwrap();

        assert_eq!(buf1.as_slice(), fixed2.inner().as_slice());
        assert_eq!(buf2.as_slice(), fixed1.inner().as_slice());
    }

    #[test]
    fn view() {
        let (reactor, _) = runtime();

        let mut buf = Fixed::register(Box::new([b'_'; 256]), reactor).unwrap();
        buf.inner_mut()[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = View::new(buf, 10..=20);
        assert_eq!(view.as_slice(), &[b'A'; 11]);
    }
}
