//! Real SavedA→separately paid unsigned SDK DATA. No I/S/B or signer permit is faked.
use super::*;
use crate::store::persistence::v2_text::PreparedV2TextAcceptedQuery;
use catcoms_replication::chat::owner_state::transition::REPLAY_MATH_PRIVATE_INVOICE;
use catcoms_sync::owner_v2::APPEND_DATA_BUFFER_BYTES;

#[test]
fn actual_three_family_unsigned_child_has_its_own_fees_and_never_advances_or_replaces_a() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7f380 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .clone();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7f390 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [25; 32],
            channel: 7,
            text: "one unsigned original child",
        };
        host.retry_retain_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let before_query = pool.charged_for_test();
        host.retry_accepted_create_lookup(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert!(host.accepted_lookup_absent_for_test());
        assert_eq!(
            pool.charged_for_test(),
            before_query + PreparedV2TextAcceptedQuery::metadata_fee_bytes_for_test()
        );
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        // The new accepted-query payment is already held; unsigned math and SDK
        // payments below remain separate additions, rather than borrowing its fee.
        let before_domain = pool.charged_for_test();
        host.retry_prepare_projection_domain(&original.layout)
            .unwrap();
        assert_eq!(pool.charged_for_test(), before_domain + 16 * 1024);
        let before = pool.charged_for_test();
        host.retry_prepare_unsigned_create_data(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(
            pool.charged_for_test(),
            before + REPLAY_MATH_PRIVATE_INVOICE + APPEND_DATA_BUFFER_BYTES
        );
        let digest = host
            .prepared_unsigned_digest_for_test(&original.layout)
            .unwrap();
        assert!(host.unsigned_original_retained_for_test());
        host.retry_prepare_unsigned_custody_data(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(
            host.prepared_unsigned_digest_for_test(&original.layout)
                .unwrap(),
            digest
        );
        assert!(host.unsigned_original_retained_for_test());
        assert_eq!(
            pool.charged_for_test(),
            before + REPLAY_MATH_PRIVATE_INVOICE + APPEND_DATA_BUFFER_BYTES
        );
        host.retry_prepare_unsigned_create_data(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(
            host.prepared_unsigned_digest_for_test(&original.layout)
                .unwrap(),
            digest
        );
        assert_eq!(
            pool.charged_for_test(),
            before + REPLAY_MATH_PRIVATE_INVOICE + APPEND_DATA_BUFFER_BYTES
        );
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        // Same-length source damage cannot turn a cached plan into current authority.
        let mut corrupt = head.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        std::fs::write(target.join("head.bin"), &corrupt).unwrap();
        assert!(host
            .retry_prepare_unsigned_create_data(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert!(host.unsigned_original_retained_for_test());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), corrupt);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        std::fs::write(target.join("head.bin"), &head).unwrap();
        host.retry_prepare_unsigned_create_data(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(
            host.prepared_unsigned_digest_for_test(&original.layout)
                .unwrap(),
            digest
        );
        host.cancel_and_drain_admitted_promotion().unwrap();
        assert!(host
            .retry_prepare_unsigned_create_data(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert!(
            host.unsigned_original_retained_for_test(),
            "cancellation retains the SAME opaque SDK original and its payment"
        );
        assert_eq!(
            pool.charged_for_test(),
            before + REPLAY_MATH_PRIVATE_INVOICE + APPEND_DATA_BUFFER_BYTES
        );
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    }
}
