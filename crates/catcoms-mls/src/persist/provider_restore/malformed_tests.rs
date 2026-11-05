//! Production restore regressions for malformed stored SDK JSON, not a heap-containment claim.
//! No catch_unwind masks the former panic: calling the real decoders must simply return Error.
use super::*;

fn changed_provider_row(label: &[u8], value: &[u8]) -> Vec<u8> {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let snapshot = snapshot_server(&device, &group).unwrap();
    let mut outer = Decoder::new(&snapshot);
    let mut map = deserialize_map(outer.get_bytes().unwrap()).unwrap();
    let signer = outer.get_bytes().unwrap();
    let group = outer.get_bytes().unwrap();
    outer.finish().unwrap();
    let key = if let Some(key) = map.keys().find(|key| key.starts_with(label)) {
        key.clone()
    } else {
        // List rows can legitimately be absent in a fresh founder. Reuse the actual SDK's
        // serialized GroupId/version suffix from its genuine Tree row; do not invent a
        // different group-key encoding or populate a fake membership/Current witness.
        assert!(matches!(label, b"OwnLeafNodes" | b"ProposalQueueRefs"));
        let tree = map.keys().find(|key| key.starts_with(b"Tree")).unwrap();
        let mut key = label.to_vec();
        key.extend_from_slice(&tree[b"Tree".len()..]);
        key
    };
    map.insert(key, value.to_vec());
    let mut altered = Encoder::new();
    altered.put_bytes(&serialize_map(&map).unwrap()).unwrap();
    altered.put_bytes(signer).unwrap();
    altered.put_bytes(group).unwrap();
    altered.finish()
}

#[test]
fn actual_legacy_and_provider_bounded_restore_refuse_malformed_typed_rows_without_panicking() {
    for label in [
        b"SignatureKeyPair".as_slice(),
        b"Tree",
        b"GroupContext",
        b"InterimTranscriptHash",
        b"ConfirmationTag",
        b"MessageSecrets",
        b"ResumptionPsk",
        b"GroupState",
    ] {
        for malformed in [b"{".as_slice(), b"null"] {
            let snapshot = changed_provider_row(label, malformed);
            assert!(restore_server(&snapshot).is_err());
            assert!(
                restore_server_provider_bounded(&snapshot, snapshot.len(), 1024 * 1024).is_err()
            );
        }
    }
}

#[test]
fn actual_restore_refuses_malformed_outer_and_inner_sdk_list_rows_without_panicking() {
    for label in [b"OwnLeafNodes".as_slice(), b"ProposalQueueRefs"] {
        // The first two refuse the outer Vec<Vec<u8>>. The third is an outer-valid byte list
        // containing invalid inner JSON, and must fail through the existing Result as well.
        for malformed in [b"{".as_slice(), b"[null]", b"[[123]]"] {
            let snapshot = changed_provider_row(label, malformed);
            assert!(restore_server(&snapshot).is_err());
            assert!(
                restore_server_provider_bounded(&snapshot, snapshot.len(), 1024 * 1024).is_err()
            );
        }
    }
}

#[test]
fn actual_restore_refuses_valid_sdk_proposal_reference_with_missing_companion_row() {
    // Construct a real SDK ProposalRef using its public hash constructor. It is deliberately
    // dangling stored metadata, not an authenticated proposal/membership or a production writer.
    let provider = OpenMlsRustCrypto::default();
    let reference = openmls::prelude::ProposalRef::new(
        b"test-only orphan proposal",
        crate::CIPHERSUITE,
        provider.crypto(),
        b"MLS 1.0 Proposal Reference",
    )
    .unwrap();
    let reference_json = serde_json::to_vec(&reference).unwrap();
    let refs = serde_json::to_vec(&vec![reference_json]).unwrap();
    let snapshot = changed_provider_row(b"ProposalQueueRefs", &refs);
    // PublicGroup::load traverses the actual refs and then the missing proposal row. Returning
    // empty or silently skipping it would misrepresent retained obligations; return Error.
    assert!(restore_server(&snapshot).is_err());
    assert!(restore_server_provider_bounded(&snapshot, snapshot.len(), 1024 * 1024).is_err());
}
