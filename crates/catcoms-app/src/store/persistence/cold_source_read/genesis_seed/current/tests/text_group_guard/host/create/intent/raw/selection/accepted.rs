//! Production Host B selection and distinct same-verifier finalization. WC, lane
//! retirement, resident reuse and native publication remain outside this regression.
use super::*;
mod retirement;

fn accepted_files(target: &Path) -> Vec<(OsString, Vec<u8>)> {
    let mut pages: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            (
                path.file_name().unwrap().to_os_string(),
                std::fs::read(path).unwrap(),
            )
        })
        .collect();
    pages.sort_by(|a, b| a.0.cmp(&b.0));
    pages
}

#[test]
fn actual_three_family_host_selects_b_then_advances_once_drains_and_finalizes_across_retries_damage_and_close(
) {
    for family in 0..3 {
        let original = text_fixture(family, 0x80040 + family as u64);
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
        let mut rng = ChaCha20Rng::seed_from_u64(0x80050 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [32; 32],
            channel: 7,
            text: "one genuine accepted original",
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
                        host.retry_accepted_create_request(&original.layout, request(), &mut rng)
                            .unwrap();
                        assert!(host.accepted_create_saved_for_test());
                        assert!(
                            !host.accepted_create_final_for_test(),
                            "SavedB alone does not advance/finalize the model"
                        );
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 0);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 0);
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        let head = std::fs::read(target.join("head.bin")).unwrap();
                        let wal = std::fs::read(target.join("text.wal")).unwrap();
                        let pages = accepted_files(&target);
                        let charged = pool.charged_for_test();
                        // Actual entry performs the guarded same-Core cut, destroys old math
                        // outside Current, then acquires a distinct fresh final B guard.
                        host.retry_finalize_accepted_create_request(
                            &original.layout,
                            request(),
                            &mut NeverRng,
                        )
                        .unwrap();
                        assert!(host.accepted_create_final_for_test());
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        for _ in 0..3 {
                            host.retry_finalize_accepted_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng,
                            )
                            .unwrap();
                            assert!(
                                host.accepted_create_saved_for_test()
                                    && host.accepted_create_final_for_test()
                            );
                            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                            assert_eq!(accepted_files(&target), pages);
                            assert_eq!(pool.charged_for_test(), charged);
                        }
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                        assert!(host
                            .retry_finalize_accepted_create_request(
                                &original.layout,
                                V2CreateRequest {
                                    token: [32; 32],
                                    channel: 7,
                                    text: "changed text under the original accepted token"
                                },
                                &mut NeverRng
                            )
                            .is_err());
                        let mut changed = head.clone();
                        let at = changed.len() / 2;
                        changed[at] ^= 0x40;
                        std::fs::write(target.join("head.bin"), &changed).unwrap();
                        assert!(host
                            .retry_finalize_accepted_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng
                            )
                            .is_err());
                        assert!(
                            host.accepted_create_saved_for_test()
                                && host.accepted_create_final_for_test(),
                            "historical cached outputs stay owned while fresh use refuses"
                        );
                        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), changed);
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        assert_eq!(accepted_files(&target), pages);
                        std::fs::write(target.join("head.bin"), &head).unwrap();
                        host.retry_finalize_accepted_create_request(
                            &original.layout,
                            request(),
                            &mut NeverRng,
                        )
                        .unwrap();
                        host.cancel_and_drain_admitted_promotion().unwrap();
                        assert!(host
                            .retry_finalize_accepted_create_request(
                                &original.layout,
                                request(),
                                &mut NeverRng
                            )
                            .is_err());
                        assert!(
                            host.accepted_create_saved_for_test()
                                && host.accepted_create_final_for_test()
                        );
                        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                        assert_eq!(accepted_files(&target), pages);
                        assert_eq!(pool.charged_for_test(), charged);
                        assert_eq!(key_calls.load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[0].load(Ordering::SeqCst), 1);
                        assert_eq!(cuts[1].load(Ordering::SeqCst), 1);
                    },
                );
            },
        );
    }
}
