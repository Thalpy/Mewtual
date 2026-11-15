//! Genuine Linux Current/settlement/Host chain. Keep this small: the Windows
//! share-mode and large query/create graphs have their own separate fixtures.
use super::*;
use crate::store::owner_evidence::canonical::text_lane::active_text_lane_count_for_test;
use crate::store::persistence::v2_text::V2TextPromotionTestCut;
use crate::store::{PreparedV2TextHost, PreparedV2TextRead, V2TextHostPolicy};

type Host = PreparedV2TextHost<MemNetwork, ChaCha20Rng>;

fn settled_host(number: u64) -> (Fixture, Host, PathBuf) {
    let mut f = fixture(number);
    f.preparation.retry(&f.layout, &mut f.rng).unwrap();
    let mut current = f.preparation.take_current(&f.layout).unwrap().unwrap();
    current
        .retry_seed_settlement(&f.layout, &mut f.rng)
        .unwrap();
    current.validate_original(&f.layout).unwrap();
    let target = current.resources.source.capture.target.clone();
    let host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_warm(current),
        V2TextHostPolicy::new(1024 * 1024 * 1024, 1024 * 1024).unwrap(),
    );
    (f, host, target)
}

#[test]
fn actual_linux_promotion_retries_partial_complete_and_renamed_work_on_the_same_host() {
    use V2TextPromotionTestCut::*;
    for (index, cut) in [
        SeedPartial,
        SeedStaged,
        SeedRenamed,
        DirectoryCreated,
        HeadStaged,
        HeadRenamed,
    ]
    .into_iter()
    .enumerate()
    {
        let (mut f, mut host, target) = settled_host(0x1eaf00 + index as u64);
        let store = f.layout.checked_store().unwrap();
        let source = std::fs::read(target.join("source.bin")).unwrap();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let scope = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        let entries = super::super::super::SDK_ENTRIES.with(|n| n.get());
        host.fail_next_promotion_work_for_test(cut);
        assert!(host.retry_empty_promotion(&f.layout, &mut f.rng).is_err());
        if cut != HeadRenamed {
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        }
        assert_eq!(active_text_lane_count_for_test(store), 1);
        // Every candidate was already drawn and cached by the original Host.
        // Real recovery must finish those bytes without a new nonce/signature.
        host.retry_empty_promotion(&f.layout, &mut NeverRng)
            .unwrap();
        let selected = std::fs::read(target.join("head.bin")).unwrap();
        assert_ne!(selected, head);
        assert!(!target.join("text.seed.stage.v1").exists());
        assert!(!target.join("head.stage").exists());
        assert_eq!(std::fs::read(target.join("source.bin")).unwrap(), source);
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            scope
        );
        assert_eq!(active_text_lane_count_for_test(store), 0);
        assert_eq!(super::super::super::SDK_ENTRIES.with(|n| n.get()), entries);
        host.retry_empty_promotion(&f.layout, &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
    }
}

#[test]
fn actual_linux_promotion_preserves_unknown_work_instead_of_treating_it_as_scratch() {
    let (mut f, mut host, target) = settled_host(0x1eaf11);
    let head = std::fs::read(target.join("head.bin")).unwrap();
    host.fail_next_promotion_work_for_test(V2TextPromotionTestCut::DirectoryCreated);
    assert!(host.retry_empty_promotion(&f.layout, &mut f.rng).is_err());
    let seed = std::fs::read(target.join("text.seed")).unwrap();
    let private = target.join("private.bin");
    std::fs::write(&private, b"possible later state").unwrap();
    for _ in 0..2 {
        assert!(host
            .retry_empty_promotion(&f.layout, &mut NeverRng)
            .is_err());
        assert_eq!(std::fs::read(&private).unwrap(), b"possible later state");
        assert_eq!(std::fs::read(target.join("text.seed")).unwrap(), seed);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    }
    // Remove only this test's injected object; the SAME original Host then
    // finishes its actual admitted debt and barriers without any new draw.
    std::fs::remove_file(private).unwrap();
    host.retry_empty_promotion(&f.layout, &mut NeverRng)
        .unwrap();
    assert_ne!(std::fs::read(target.join("head.bin")).unwrap(), head);
}
