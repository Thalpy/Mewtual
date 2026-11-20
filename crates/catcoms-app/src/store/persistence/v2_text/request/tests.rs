//! Value/key codec DATA; no source/save/selected-index or action-authenticity fixture is faked.
use super::*;
fn token(phase: RequestPhaseData) -> TokenValueData {
    let context = TextContextIdData::from_bytes([1; 32]);
    let token = [2; 32];
    let request =
        TextRequestDigestData::from_request_data(context, token, 3, "private test text").unwrap();
    TokenValueData {
        transaction: TextTransactionIdData::from_request_data(context, token, request),
        token,
        request,
        retain: TextRetainIdData::from_bytes([4; 32]),
        action: ActionId::from_bytes([5; 32]),
        original: OriginalActionDigest::from_bytes([6; 32]),
        phase,
    }
}
fn accepted() -> AcceptedLocatorData {
    AcceptedLocatorData {
        header: OwnerHeaderId::from_bytes([7; 32]),
        record_page: PrivateChatPageId::from_bytes([8; 32]),
        record_row: 0,
        ordinal: 9,
        receipt: ReceiptDigestData::from_bytes([10; 32]),
    }
}
#[test]
fn stable_request_key_uses_canonical_source_fields_and_exact_text() {
    let context = TextContextIdData::from_bytes([1; 32]);
    let value = token(RequestPhaseData::Retained);
    value
        .check_request_data(context, [2; 32], 3, "private test text")
        .unwrap();
    let mut canonical = Encoder::new();
    put(&mut canonical, context.as_bytes()).unwrap();
    put(&mut canonical, &[2; 32]).unwrap();
    canonical.put_u128(3);
    put(&mut canonical, b"private test text").unwrap();
    assert!(value.request == TextRequestDigestData::of_data(&canonical.finish()));
    for _ in 0..3 {
        let reconstructed = token(RequestPhaseData::Retained);
        assert!(reconstructed.key_data(context) == value.key_data(context));
        assert!(reconstructed.request == value.request);
    }
    for (c, t, ch, text) in [
        (
            TextContextIdData::from_bytes([11; 32]),
            [2; 32],
            3,
            "private test text",
        ),
        (context, [12; 32], 3, "private test text"),
        (context, [2; 32], 4, "private test text"),
        (context, [2; 32], 3, "changed"),
    ] {
        assert!(value.check_request_data(c, t, ch, text).is_err());
    }
    assert!(TextRequestDigestData::from_request_data(context, [2; 32], 3, "").is_err());
    assert!(TextRequestDigestData::from_request_data(
        context,
        [2; 32],
        3,
        &"x".repeat(MAX_TEXT_BYTES + 1)
    )
    .is_err());
    let mut canonical = Encoder::new();
    put(&mut canonical, context.as_bytes()).unwrap();
    put(&mut canonical, &value.token).unwrap();
    assert!(TextTokenKeyData::of_data(&canonical.finish()) == value.key_data(context));
    let mut canonical = Encoder::new();
    put(&mut canonical, context.as_bytes()).unwrap();
    put(&mut canonical, &value.token).unwrap();
    put(&mut canonical, value.request.as_bytes()).unwrap();
    assert!(TextTransactionIdData::of_data(&canonical.finish()) == value.transaction);
}
#[test]
fn exact_value_caps_and_all_prefixes_phase_mismatch_and_lengths() {
    for phase in [
        RequestPhaseData::Retained,
        RequestPhaseData::Accepted(accepted()),
        RequestPhaseData::Stopped,
    ] {
        let value = token(phase);
        let raw = value.encode_data().unwrap();
        assert!(TokenValueData::decode_data(&raw).unwrap() == value);
        assert_eq!(
            raw.len(),
            if matches!(phase, RequestPhaseData::Accepted(_)) {
                336
            } else {
                218
            }
        );
        for n in 0..raw.len() {
            assert!(TokenValueData::decode_data(&raw[..n]).is_err());
        }
        let mut bad = raw.to_vec();
        bad.push(0);
        assert!(TokenValueData::decode_data(&bad).is_err());
        let mut bad = raw.to_vec();
        bad[0] = 0;
        assert!(TokenValueData::decode_data(&bad).is_err());
        let mut bad = raw.to_vec();
        bad[1..5].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(TokenValueData::decode_data(&bad).is_err());
        let mut bad = raw.to_vec();
        bad[0] = if matches!(phase, RequestPhaseData::Accepted(_)) {
            1
        } else {
            2
        };
        assert!(TokenValueData::decode_data(&bad).is_err());
    }
    assert!(TokenValueData::decode_data(&vec![0; MAX_TOKEN_VALUE_BYTES + 1]).is_err());
    assert_eq!(
        format!("{:?}", token(RequestPhaseData::Accepted(accepted()))),
        "TokenValueData { .. }"
    );
    assert_eq!(
        format!("{:?}", RequestPhaseData::Accepted(accepted())),
        "Accepted { .. }"
    );
}
#[test]
fn accepted_locator_row_bound_applies_to_both_encoder_and_decoder() {
    for record_row in [0u16, 63] {
        let value = token(RequestPhaseData::Accepted(AcceptedLocatorData {
            record_row,
            ..accepted()
        }));
        let raw = value.encode_data().unwrap();
        assert!(TokenValueData::decode_data(&raw).unwrap() == value);
    }
    let valid = token(RequestPhaseData::Accepted(accepted()))
        .encode_data()
        .unwrap();
    let row_offset = 1 + 6 * 36 + 1 + 2 * 36;
    for record_row in [64u16, u16::MAX] {
        let value = token(RequestPhaseData::Accepted(AcceptedLocatorData {
            record_row,
            ..accepted()
        }));
        assert!(value.encode_data().is_err());
        let mut raw = valid.to_vec();
        raw[row_offset..row_offset + 2].copy_from_slice(&record_row.to_be_bytes());
        assert!(TokenValueData::decode_data(&raw).is_err());
    }
}
#[test]
fn publication_values_and_ordinal_key_have_separate_complete_geometry() {
    let t = token(RequestPhaseData::Accepted(accepted()));
    let value = PublicationValueData {
        transaction: t.transaction,
        action: t.action,
        receipt: accepted().receipt,
        header: accepted().header,
        publication: TextPublicationIdData::from_bytes([13; 32]),
        phase: PublicationPhaseData::Eligible,
    };
    let raw = value.encode_data().unwrap();
    assert_eq!(raw.len(), MAX_PUBLICATION_VALUE_BYTES);
    assert!(PublicationValueData::decode_data(&raw).unwrap() == value);
    for n in 0..raw.len() {
        assert!(PublicationValueData::decode_data(&raw[..n]).is_err());
    }
    let mut bad = raw.to_vec();
    bad.push(0);
    assert!(PublicationValueData::decode_data(&bad).is_err());
    let mut bad = raw.to_vec();
    bad[MAX_PUBLICATION_VALUE_BYTES - 1] = 3;
    assert!(PublicationValueData::decode_data(&bad).is_err());
    assert!(TokenValueData::decode_data(&raw).is_err());
    assert!(PublicationValueData::decode_data(&t.encode_data().unwrap()).is_err());
    for n in [0, 1, u64::MAX] {
        assert_eq!(
            PublicationValueData::ordinal_from_key_data(PublicationValueData::ordinal_key_data(n))
                .unwrap(),
            n
        );
    }
    let mut bad = PublicationValueData::ordinal_key_data(1);
    bad[0] = 1;
    assert!(PublicationValueData::ordinal_from_key_data(bad).is_err());
    let ack = PublicationValueData {
        phase: PublicationPhaseData::Acknowledged,
        ..value
    };
    assert!(PublicationValueData::decode_data(&ack.encode_data().unwrap()).unwrap() == ack);
}
