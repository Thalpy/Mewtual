//! Encoder-pair guards, separate from the genuine index2 JOIN/cold-open test.
//! Deliberately constructed internal pairs below establish no descriptor,
//! membership, physical mount or version3 handler authority.
use super::*;

#[test]
fn native_namespace_pair_encoder_preserves_all_closed_entry_versions() {
    let vault = [0x96; 32];
    let plain = crate::store::critical_namespaces::encode_empty(&vault).unwrap();
    let mut parsed = codec::parse(&plain, &vault).unwrap();
    let mut output = Encoder::new();
    append_original_archive(&mut output, &parsed).unwrap();
    assert!(output.finish().is_empty());
    let payload = b"exact internal encoder payload";
    for version in [1, 2, 3] {
        parsed.native_registry_version = Some(version);
        parsed.native_registry_payload = Some(payload);
        assert_eq!(archive_suffix_length(&parsed).unwrap(), 12 + payload.len());
        let mut output = Encoder::new();
        append_original_archive(&mut output, &parsed).unwrap();
        let bytes = output.finish();
        let mut input = Decoder::new(&bytes);
        assert_eq!(input.get_u32().unwrap(), 7);
        assert_eq!(input.get_u32().unwrap(), version);
        assert_eq!(input.get_bytes().unwrap(), payload);
        input.finish().unwrap();
    }
}

#[test]
fn native_namespace_pair_encoder_rejects_inconsistent_or_unknown_pairs_before_output() {
    let vault = [0x97; 32];
    let plain = crate::store::critical_namespaces::encode_empty(&vault).unwrap();
    let mut parsed = codec::parse(&plain, &vault).unwrap();
    for (version, payload) in [
        (None, Some(&b"payload only"[..])),
        (Some(1), None),
        (Some(2), None),
        (Some(3), None),
        (Some(0), Some(&b"unknown"[..])),
        (Some(4), Some(&b"future"[..])),
        (Some(u32::MAX), Some(&b"unknown"[..])),
    ] {
        parsed.native_registry_version = version;
        parsed.native_registry_payload = payload;
        assert!(archive_suffix_length(&parsed).is_err());
        let mut output = Encoder::new();
        output.put_u32(0xfeed);
        assert!(append_original_archive(&mut output, &parsed).is_err());
        assert_eq!(output.len(), 4);
    }
}
