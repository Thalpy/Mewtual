//! Complete the actual Host request, dispose its old graph, and retry from only
//! the same flat resident. This is not second-Create or native-posting acceptance.
use super::*;

#[test]
fn actual_three_family_host_finishes_to_resident_releases_old_graph_and_refuses_genesis_reentry() {
    for family in 0..3 {
        let original = text_fixture(family, 0x82600 + family as u64);
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
        let mut rng = ChaCha20Rng::seed_from_u64(0x82610 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [39; 32],
            channel: 7,
            text: "same verifier after actual Host completion",
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
                        let before = pool.charged_for_test();
                        assert!(host
                            .retry_finish_accepted_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .unwrap());
                        assert!(host.resident_without_old_request_for_test());
                        let after = pool.charged_for_test();
                        assert!(
                            after < before,
                            "the old paid transaction graph actually drained"
                        );
                        let head = std::fs::read(target.join("head.bin")).unwrap();
                        let wal = std::fs::read(target.join("text.wal")).unwrap();
                        let pages = accepted_files(&target);
                        for _ in 0..3 {
                            assert!(host
                                .retry_finish_accepted_create_request(
                                    &original.layout,
                                    request(),
                                    &mut NeverRng,
                                )
                                .unwrap());
                            assert_eq!(pool.charged_for_test(), after);
                        }
                        for changed in [
                            V2CreateRequest {
                                token: [40; 32],
                                channel: 7,
                                text: request().text,
                            },
                            V2CreateRequest {
                                token: request().token,
                                channel: 8,
                                text: request().text,
                            },
                            V2CreateRequest {
                                token: request().token,
                                channel: 7,
                                text: "different request",
                            },
                        ] {
                            assert!(host
                                .retry_finish_accepted_create_request(
                                    &original.layout,
                                    changed,
                                    &mut NeverRng,
                                )
                                .is_err());
                        }
                        // Clearing the old O1 Saved must not open a route back into
                        // genesis/first-only preparation or acquire a replacement model.
                        assert!(host
                            .retry_empty_promotion(&original.layout, &mut NeverRng)
                            .is_err());
                        assert!(host
                            .retry_original_seed_settlement(&original.layout, &mut NeverRng)
                            .is_err());
                        assert!(host
                            .retry_token_lookup(&original.layout, request().token)
                            .is_err());
                        assert!(host
                            .retry_prepare_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .is_err());
                        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        assert_eq!(accepted_files(&target), pages);
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        host.cancel_and_drain_admitted_promotion().unwrap();
                        assert!(host
                            .retry_finish_accepted_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .is_err());
                        assert!(host.resident_without_old_request_for_test());
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        assert_eq!(accepted_files(&target), pages);
                    },
                );
            },
        );
    }
}
