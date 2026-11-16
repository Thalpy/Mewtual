use super::*;

#[test]
fn quic_width_overflow_nonminimal_and_field_overrun_do_not_allocate() {
    for bytes in [
        &[0x40, 0][..],
        &[0x80, 0, 0, 63][..],
        &[0xc0][..],
        &[63][..],
        &[0x40][..],
    ] {
        let mut c = Cursor { rest: bytes };
        assert!(c.vector(32).is_err());
    }
    let mut bounded = [0u8; 66];
    bounded[0] = 0x40;
    bounded[1] = 64;
    let mut c = Cursor { rest: &bounded };
    assert_eq!(c.vector(64).unwrap().len(), 64);
    c.finish().unwrap();
    assert!(scan_work(usize::MAX).is_none());
}
