//! Real owning Host close/drain/detach, preserving the selected signed history.
//! Physical WD/WC and resident reuse remain separate from these reader tests.
use super::*;
mod effects;
mod finish;
mod next;
use crate::store::owner_evidence::canonical::text_lane::text_query_registry_counts_for_test;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

#[test]
fn actual_three_family_host_closes_and_detaches_original_roots_without_repeating_signer_or_core() {
    for family in 0..3 {
        let original = text_fixture(family, 0x80300 + family as u64);
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x80310 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [35; 32],
            channel: 7,
            text: "accepted history survives reader retirement",
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
                        let store = original.layout.checked_store().unwrap();
                        assert_eq!(text_query_registry_counts_for_test(store), (2, 2));
                        let head = std::fs::read(target.join("head.bin")).unwrap();
                        let wal = std::fs::read(target.join("text.wal")).unwrap();
                        let pages = accepted_files(&target);
                        assert!(host
                            .retry_close_accepted_create_roots(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .unwrap());
                        assert!(host.accepted_roots_detached_for_test());
                        assert_eq!(text_query_registry_counts_for_test(store), (0, 0));
                        for _ in 0..3 {
                            assert!(host
                                .retry_close_accepted_create_roots(
                                    &original.layout,
                                    request(),
                                    &mut NeverRng,
                                )
                                .unwrap());
                            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                            assert_eq!(accepted_files(&target), pages);
                        }
                        assert!(host
                            .retry_close_accepted_create_roots(
                                &original.layout,
                                V2CreateRequest {
                                    token: [35; 32],
                                    channel: 7,
                                    text: "substituted original",
                                },
                                &mut NeverRng,
                            )
                            .is_err());
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        assert!(host.accepted_create_saved_for_test());
                        assert!(host.accepted_create_final_for_test());
                    },
                );
            },
        );
    }
}

#[test]
fn actual_host_late_scope_stop_after_root_close_releases_own_readers_and_preserves_pending_detach()
{
    let original = text_fixture(0, 0x80340);
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let revoke = owner.warm_scope().unwrap().revocation();
    drop(owner);
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    let mut rng = ChaCha20Rng::seed_from_u64(0x80350);
    host.retry_empty_promotion(&original.layout, &mut rng)
        .unwrap();
    let request = || V2CreateRequest {
        token: [36; 32],
        channel: 7,
        text: "keep original after actual close",
    };
    host.retry_finalize_accepted_create_request(&original.layout, request(), &mut rng)
        .unwrap();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let pages = accepted_files(&target);
    host.set_after_accepted_roots_close_for_test(Box::new(move || revoke.revoke()));
    assert!(host
        .retry_close_accepted_create_roots(&original.layout, request(), &mut NeverRng,)
        .is_err());
    let store = original.layout.checked_store().unwrap();
    // Both registry entries remain closed, but actual logical reader Ends drained.
    assert_eq!(text_query_registry_counts_for_test(store), (2, 0));
    assert!(!host.accepted_roots_detached_for_test());
    for _ in 0..2 {
        assert!(host
            .retry_close_accepted_create_roots(&original.layout, request(), &mut NeverRng,)
            .is_err());
        assert_eq!(text_query_registry_counts_for_test(store), (2, 0));
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(accepted_files(&target), pages);
    }
    assert!(host.accepted_create_saved_for_test());
    assert!(host.accepted_create_final_for_test());
}
