//! Genuine source recapture: a new paid read has new runtime ownership but the SAME
//! frozen persistent C/token/request/Txn identities. No Source or selected O is invented.
use super::*;
use crate::store::persistence::v2_text::{
    TextRequestDigestData, TextTokenKeyData, TextTransactionIdData,
};

#[test]
fn actual_source_context_hash_and_ids_repeat_across_independent_genuine_reads() {
    for family in 0..3 {
        let original = super::promotion::promoted_text_fixture(family, 0x6e700 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let mut first = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        let mut second = PreparedV2TextParentCapture::prepare(store, owner.source()).unwrap();
        for capture in [&mut first, &mut second] {
            let resources = capture.resources().clone();
            store
                .with_v2_text_current(&owner, |a| {
                    let guard = store.lock_v2_text_current_group(a, &resources)?;
                    let (expected, _, _) = guard.original_genesis_context_data()?;
                    let canonical =
                        crate::store::critical_namespaces::archive_bootstrap::founder_codec::encode_context(
                            &expected.context,
                        )?;
                    // Independent explicit frozen grammar, not the implementation helper.
                    let mut hash = blake3::Hasher::new();
                    hash.update(b"catcoms/private-chat-text-source-context/v1\0");
                    hash.update(&(canonical.len() as u32).to_be_bytes());
                    hash.update(&canonical);
                    let expected_digest = *hash.finalize().as_bytes();
                    let selected = capture.capture(&guard)?;
                    assert_eq!(
                        selected.head_data().context.spec_data().source_context_digest,
                        expected_digest,
                    );
                    assert_ne!(expected_digest, *blake3::hash(&canonical).as_bytes());
                    Ok(())
                })
                .unwrap();
        }
        let first = first.selected_data().unwrap();
        let second = second.selected_data().unwrap();
        assert_eq!(first.head_id_data(), second.head_id_data());
        assert_eq!(
            first.head_data().context.encode_data().unwrap(),
            second.head_data().context.encode_data().unwrap(),
        );
        let c1 = first.head_data().context.id_data().unwrap();
        let c2 = second.head_data().context.id_data().unwrap();
        assert_eq!(c1, c2);
        let token = [9; 32];
        let r1 = TextRequestDigestData::from_request_data(c1, token, 7, "same original request")
            .unwrap();
        let r2 = TextRequestDigestData::from_request_data(c2, token, 7, "same original request")
            .unwrap();
        assert_eq!(r1, r2);
        assert_eq!(
            TextTokenKeyData::from_request_data(c1, token),
            TextTokenKeyData::from_request_data(c2, token),
        );
        assert_eq!(
            TextTransactionIdData::from_request_data(c1, token, r1),
            TextTransactionIdData::from_request_data(c2, token, r2),
        );
        // Distinct actual read debits are NOT aliases of the same runtime receipt even
        // though their persistent commitments deliberately agree.
        assert!(first
            .resources()
            .require_same_original(second.resources())
            .is_err());
    }
}
