//! Genuine Host-owned distinct WD/WC dispatch. No test invokes either worker or
//! creates a retirement permit; the production caller owns both real admissions.
use super::*;
use crate::store::persistence::v2_text::TextWalFrameData;

#[test]
fn actual_three_family_host_selects_wc_after_private_deletion_and_retries_preserve_public_history()
{
    for family in 0..3 {
        let original = text_fixture(family, 0x80380 + family as u64);
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x80390 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [37; 32],
            channel: 7,
            text: "public history remains after private retirement",
        };
        let key_calls = Arc::new(AtomicUsize::new(0));
        let cuts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            key_calls.clone(),
            None,
            || {
                PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
                    cuts.clone(),
                    || {
                        host.retry_finalize_accepted_create_request(
                            &original.layout,
                            request(),
                            &mut rng,
                        )
                        .unwrap();
                        let head = std::fs::read(target.join("head.bin")).unwrap();
                        let before = accepted_files(&target);
                        assert!(host
                            .retry_retire_accepted_create_files(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .unwrap());
                        let after = accepted_files(&target);
                        let removed: Vec<_> = before
                            .iter()
                            .filter(|entry| !after.iter().any(|kept| kept.0 == entry.0))
                            .collect();
                        assert_eq!(removed.len(), 4, "only the exact original Plan is deleted");
                        assert_eq!(
                            removed
                                .iter()
                                .filter(|(name, _)| { name.to_string_lossy().starts_with("p-") })
                                .count(),
                            2
                        );
                        assert_eq!(
                            removed
                                .iter()
                                .filter(|(name, _)| { name.to_string_lossy().starts_with("i-") })
                                .count(),
                            1
                        );
                        assert_eq!(
                            removed
                                .iter()
                                .filter(|(name, _)| { name.to_string_lossy().starts_with("s-") })
                                .count(),
                            1
                        );
                        // Every surviving immutable artifact is byte-identical. In
                        // particular the signed public history is not a delete target.
                        for kept in &after {
                            assert!(before.contains(kept));
                        }
                        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                        let wal = std::fs::read(target.join("text.wal")).unwrap();
                        let store = original.layout.checked_store().unwrap();
                        let key = store.keys.db_key().unwrap();
                        let derived =
                            blake3::derive_key("catcoms/private-chat-text-wal-key/v1", &key);
                        let plain =
                            catcoms_crypto::unseal(&derived, &crate::store::unframe(&wal).unwrap())
                                .unwrap();
                        assert!(matches!(
                            TextWalFrameData::decode_data(&plain).unwrap(),
                            TextWalFrameData::Completion(_)
                        ));
                        assert_eq!(text_query_registry_counts_for_test(store), (0, 0));
                        for _ in 0..3 {
                            assert!(host
                                .retry_retire_accepted_create_files(
                                    &original.layout,
                                    request(),
                                    &mut NeverRng,
                                )
                                .unwrap());
                            assert_eq!(accepted_files(&target), after);
                            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        }
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        host.cancel_and_drain_admitted_promotion().unwrap();
                        assert!(
                            host.retry_retire_accepted_create_files(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .is_err(),
                            "historical WC cannot authorize a stopped host"
                        );
                        assert_eq!(accepted_files(&target), after);
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                    },
                );
            },
        );
    }
}
