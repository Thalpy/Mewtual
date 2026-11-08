//! Owning Host next-packet preparation through a real completed-WC predecessor.
//! No second A selection, owner-header signing or publication is claimed here.
use super::*;

#[test]
fn actual_three_family_host_prepares_one_next_packet_after_wc_without_restarting_genesis() {
    for family in 0..3 {
        let original = text_fixture(family, 0x82900 + family as u64);
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x82910 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let first = || V2CreateRequest {
            token: [45; 32],
            channel: 7,
            text: "first actual completed Host request",
        };
        let next = || V2CreateRequest {
            token: [46; 32],
            channel: 7,
            text: "next original packet from completed WC",
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
                        assert!(host
                            .retry_finish_accepted_create_request(
                                &original.layout,
                                first(),
                                &mut rng,
                            )
                            .unwrap());
                        let head = std::fs::read(target.join("head.bin")).unwrap();
                        let wal = std::fs::read(target.join("text.wal")).unwrap();
                        let pages = accepted_files(&target);
                        host.retry_prepare_completed_create_request(
                            &original.layout,
                            next(),
                            &mut rng,
                        )
                        .unwrap();
                        let digest = host.completed_create_digest_for_test().unwrap();
                        for _ in 0..3 {
                            host.retry_prepare_completed_create_request(
                                &original.layout,
                                next(),
                                &mut NeverRng,
                            )
                            .unwrap();
                            assert_eq!(host.completed_create_digest_for_test(), Some(digest));
                        }
                        // The same token cannot be rebound while original packet
                        // custody is live, even though the source itself is intact.
                        assert!(host
                            .retry_prepare_completed_create_request(
                                &original.layout,
                                V2CreateRequest {
                                    token: next().token,
                                    channel: 7,
                                    text: "different under same token",
                                },
                                &mut NeverRng,
                            )
                            .is_err());
                        assert_eq!(host.completed_create_digest_for_test(), Some(digest));
                        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        assert_eq!(accepted_files(&target), pages);
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        host.cancel_and_drain_admitted_promotion().unwrap();
                        assert!(host
                            .retry_prepare_completed_create_request(
                                &original.layout,
                                next(),
                                &mut NeverRng,
                            )
                            .is_err());
                        assert_eq!(host.completed_create_digest_for_test(), Some(digest));
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                    },
                );
            },
        );
    }
}
