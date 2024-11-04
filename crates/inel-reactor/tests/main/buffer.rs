mod view {
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
}
