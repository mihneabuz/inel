mod view {
    use std::ops::RangeBounds;

    use inel_reactor::buffer::StableBuffer;

    #[test]
    fn included() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..=20].copy_from_slice(&[b'A'; 11]);

        let view = buf.view(10..=20);
        assert_eq!(view.as_slice(), &[b'A'; 11]);
    }

    #[test]
    fn excluded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[10..20].copy_from_slice(&[b'A'; 10]);

        let view = buf.view(10..20);
        assert_eq!(view.as_slice(), &[b'A'; 10]);
    }

    #[test]
    fn unbounded() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..].copy_from_slice(&[b'A'; 256]);

        let view = buf.view(..);
        assert_eq!(view.as_slice(), &[b'A'; 256]);
    }

    #[test]
    fn mixed() {
        let mut buf = Box::new([b'_'; 256]);
        buf[..10].copy_from_slice(&[b'A'; 10]);

        let view = buf.view(..10);
        assert_eq!(view.as_slice(), &[b'A'; 10]);

        let mut buf = view.unview();
        buf[20..].copy_from_slice(&[b'A'; 236]);

        let view = buf.view(20..);
        assert_eq!(view.as_slice(), &[b'A'; 236]);
    }

    #[test]
    fn mutable() {
        let buf = Box::new([b'_'; 256]);
        let mut view = buf.view(10..20);
        view.as_mut_slice().copy_from_slice(&[b'A'; 10]);

        let buf = view.unview();
        assert_eq!(&buf.as_slice()[10..20], &[b'A'; 10]);
    }

    #[test]
    fn as_ref() {
        let buf = Box::new([b'_'; 256]);
        let view = buf.view(..);
        assert_eq!(view.as_ref(), &[b'_'; 256]);

        let buf = view.unview();
        let mut view = buf.view(..);
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

        let view = buf.view(VeryExclusiveRange { inner: (10, 20) });
        assert_eq!(view.as_slice(), &[b'A'; 9]);
    }
}
