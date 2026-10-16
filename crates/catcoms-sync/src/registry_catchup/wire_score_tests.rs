use super::*;
#[test]
fn score_page_scope_is_explicit_and_legacy_queries_remain_byte_identical() {
    for (target, tag, object) in [
        (StudioTarget::Index { channel: [7; 16] }, 15u16, [0; 16]),
        (
            StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            },
            16,
            [9; 16],
        ),
        (
            StudioTarget::Score {
                channel: [7; 16],
                object: [9; 16],
            },
            0x8010,
            [9; 16],
        ),
    ] {
        let bytes = encode_scoped_query(PageScope::Studio(target), 55, &[], None, None).unwrap();
        let mut expected = Encoder::new();
        expected
            .put_u8(1)
            .put_u16(tag)
            .put_u128(u128::from_be_bytes([7; 16]))
            .put_u128(u128::from_be_bytes(object))
            .put_u128(55)
            .put_u8(0);
        expected.put_bytes(&[]).unwrap();
        expected.put_bytes(&[]).unwrap();
        assert_eq!(bytes, expected.finish());
        let q = decode_scoped_query(KIND_STUDIO_PAGE, &bytes).unwrap();
        assert_eq!(q.scope, PageScope::Studio(target));
        assert_eq!(q.doc_id, 55);
        assert!(q.heads.is_empty() && q.seed.is_none() && q.cursor.is_none());
        assert_eq!(
            q.scope.doc_type(),
            if tag == 15 {
                DocType::StudioIndex
            } else {
                DocType::StudioObject
            }
        );
        let mut bad = bytes;
        bad[1..3].copy_from_slice(&0x8011u16.to_be_bytes());
        assert!(decode_scoped_query(KIND_STUDIO_PAGE, &bad).is_err());
    }
}
