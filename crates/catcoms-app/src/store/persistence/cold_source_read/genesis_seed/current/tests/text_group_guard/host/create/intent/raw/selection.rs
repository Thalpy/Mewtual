//! Actual Host-owned I -> guarded key cut -> S Stage/Commit dispatch and retries.
//! SavedS is durable raw custody only; this test does not claim accepted history B.
use super::*;

fn signature_files(target: &Path) -> Vec<(OsString, Vec<u8>)> {
    let mut frames: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("s-")
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_os_string(),
                std::fs::read(path).unwrap(),
            )
        })
        .collect();
    frames.sort_by(|a, b| a.0.cmp(&b.0));
    frames
}

#[test]
fn actual_three_family_host_selects_durable_s_once_and_retries_preserve_original_after_corruption_and_close(
) {
    for family in 0..3 {
        let original = text_fixture(family, 0x7fbe0 + family as u64);
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
        let mut rng = ChaCha20Rng::seed_from_u64(0x7fbf0 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [30; 32],
            channel: 7,
            text: "one original durable raw signature",
        };
        host.retry_unsigned_intent_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        assert!(host.unsigned_intent_saved_for_test());
        assert!(!host.raw_signature_saved_for_test());
        assert!(signature_files(&target).is_empty());
        let selected_i = std::fs::read(target.join("head.bin")).unwrap();
        let wal_i = std::fs::read(target.join("text.wal")).unwrap();
        let intents = intent_files(&target);
        let charged = pool.charged_for_test();
        let dispatches = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            dispatches.clone(),
            None,
            || {
                // This production entry owns the genuine signer and distinct S Stage/
                // Commit permits. No fixture invokes its workers or supplies raw bytes.
                host.retry_raw_signature_create_request(&original.layout, request(), &mut rng)
                    .unwrap();
                assert!(host.raw_signature_saved_for_test());
                let selected_s = std::fs::read(target.join("head.bin")).unwrap();
                let wal_s = std::fs::read(target.join("text.wal")).unwrap();
                let signatures = signature_files(&target);
                assert_ne!(selected_s, selected_i);
                assert_ne!(wal_s, wal_i);
                assert_eq!(signatures.len(), 1);
                assert!(!target.join("head.stage").exists());
                assert!(!target.join("text.wal.stage.v1").exists());
                assert_eq!(pool.charged_for_test(), charged);
                for _ in 0..3 {
                    host.retry_raw_signature_create_request(
                        &original.layout,
                        request(),
                        &mut NeverRng,
                    )
                    .unwrap();
                    assert!(host.raw_signature_saved_for_test());
                    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
                    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal_s);
                    assert_eq!(signature_files(&target), signatures);
                    assert_eq!(intent_files(&target), intents);
                    assert_eq!(pool.charged_for_test(), charged);
                }
                assert_eq!(dispatches.load(Ordering::SeqCst), 1);
                assert!(host
                    .retry_raw_signature_create_request(
                        &original.layout,
                        V2CreateRequest {
                            token: [30; 32],
                            channel: 7,
                            text: "changed text under the durable S token",
                        },
                        &mut NeverRng
                    )
                    .is_err());
                let mut changed = selected_s.clone();
                let at = changed.len() / 2;
                changed[at] ^= 0x10;
                std::fs::write(target.join("head.bin"), &changed).unwrap();
                assert!(host
                    .retry_raw_signature_create_request(&original.layout, request(), &mut NeverRng)
                    .is_err());
                assert!(host.raw_signature_saved_for_test());
                assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), changed);
                assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal_s);
                assert_eq!(signature_files(&target), signatures);
                std::fs::write(target.join("head.bin"), &selected_s).unwrap();
                host.retry_raw_signature_create_request(&original.layout, request(), &mut NeverRng)
                    .unwrap();
                host.cancel_and_drain_admitted_promotion().unwrap();
                assert!(host
                    .retry_raw_signature_create_request(&original.layout, request(), &mut NeverRng)
                    .is_err());
                assert!(host.raw_signature_saved_for_test());
                assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected_s);
                assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal_s);
                assert_eq!(signature_files(&target), signatures);
                assert_eq!(intent_files(&target), intents);
                assert_eq!(pool.charged_for_test(), charged);
                assert_eq!(dispatches.load(Ordering::SeqCst), 1);
            },
        );
    }
}

mod accepted;
