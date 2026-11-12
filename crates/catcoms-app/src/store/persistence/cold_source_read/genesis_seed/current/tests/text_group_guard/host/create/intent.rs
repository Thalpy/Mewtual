//! Actual three-family facade through retain/query/SDK custody/I selection. Persisted
//! unsigned I is not an acceptance receipt, raw signature or native chat activation.
use super::*;
use std::ffi::OsString;
use std::path::Path;

fn intent_files(target: &Path) -> Vec<(OsString, Vec<u8>)> {
    let mut frames: Vec<_> = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("i-")
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
fn actual_three_family_host_selects_original_unsigned_i_and_preserves_it_across_retries_and_stop() {
    for family in 0..3 {
        let original = text_fixture(family, 0x7fa00 + family as u64);
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
        let source = std::fs::read(target.join("source.bin")).unwrap();
        let admissions = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        drop(owner);
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        let mut rng = ChaCha20Rng::seed_from_u64(0x7fa10 + family as u64);
        host.retry_empty_promotion(&original.layout, &mut rng)
            .unwrap();
        let request = || V2CreateRequest {
            token: [28; 32],
            channel: 7,
            text: "one original unsigned intent",
        };
        host.retry_unsigned_intent_create_request(&original.layout, request(), &mut rng)
            .unwrap();
        assert!(host.unsigned_intent_saved_for_test());
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let intents = intent_files(&target);
        assert_eq!(
            intents.len(),
            1,
            "the real first I names exactly one intent"
        );
        let charged = pool.charged_for_test();
        for _ in 0..2 {
            host.retry_unsigned_intent_create_request(&original.layout, request(), &mut NeverRng)
                .unwrap();
            assert!(host.unsigned_intent_saved_for_test());
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
            assert_eq!(intent_files(&target), intents);
            assert_eq!(pool.charged_for_test(), charged);
        }
        assert!(host
            .retry_unsigned_intent_create_request(
                &original.layout,
                V2CreateRequest {
                    token: [28; 32],
                    channel: 7,
                    text: "a different body under the original token",
                },
                &mut NeverRng,
            )
            .is_err());
        assert!(host.unsigned_intent_saved_for_test());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(intent_files(&target), intents);
        assert_eq!(pool.charged_for_test(), charged);

        let mut changed = head.clone();
        let at = changed.len() / 2;
        changed[at] ^= 0x80;
        std::fs::write(target.join("head.bin"), &changed).unwrap();
        assert!(host
            .retry_unsigned_intent_create_request(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert!(host.unsigned_intent_saved_for_test());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), changed);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(intent_files(&target), intents);
        assert_eq!(pool.charged_for_test(), charged);
        std::fs::write(target.join("head.bin"), &head).unwrap();
        host.retry_unsigned_intent_create_request(&original.layout, request(), &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(intent_files(&target), intents);

        host.cancel_and_drain_admitted_promotion().unwrap();
        assert!(host
            .retry_unsigned_intent_create_request(&original.layout, request(), &mut NeverRng)
            .is_err());
        assert!(host.unsigned_intent_saved_for_test());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert_eq!(intent_files(&target), intents);
        assert_eq!(std::fs::read(target.join("source.bin")).unwrap(), source);
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            admissions
        );
        assert_eq!(pool.charged_for_test(), charged);
    }
}

mod raw;
