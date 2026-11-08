//! Exercise the actual facade dispatcher, rather than manually choosing repair roles.
use super::*;
use crate::store::persistence::v2_text::V2TextPromotionTestCut;

#[test]
fn actual_three_family_host_recovers_partial_or_complete_promotion_without_resealing() {
    for (family, cut) in [
        (0, V2TextPromotionTestCut::SeedPartial),
        (1, V2TextPromotionTestCut::DirectoryCreated),
        (2, V2TextPromotionTestCut::SeedStaged),
    ] {
        let original = text_fixture(family, 0x7ee00 + family as u64);
        let store = original.layout.checked_store().unwrap();
        let owner = original.owner.borrow_current().unwrap();
        let target = owner.source().source().capture.target.to_path_buf();
        let source = std::fs::read(target.join("source.bin")).unwrap();
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let scope = std::fs::read(store.dir.join("chat-admissions.bin")).unwrap();
        drop(owner);
        let entries = sdk_entries();
        let mut host = PreparedV2TextHost::from_read(
            PreparedV2TextRead::from_original_owner(original.owner),
            host_policy(),
        );
        host.fail_next_promotion_work_for_test(cut);
        assert!(host
            .retry_empty_promotion(
                &original.layout,
                &mut ChaCha20Rng::seed_from_u64(0x7ef00 + family as u64),
            )
            .is_err());
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(active_text_lane_count_for_test(store), 1);
        // No new candidate, nonce, signature or source export may occur during recovery.
        // The same original cached frames must suffice through fresh effect admissions.
        host.retry_empty_promotion(&original.layout, &mut NeverRng)
            .unwrap();
        let selected = std::fs::read(target.join("head.bin")).unwrap();
        assert_ne!(selected, head);
        assert!(!target.join("text.seed.stage.v1").exists());
        assert_eq!(std::fs::read(target.join("source.bin")).unwrap(), source);
        assert_eq!(
            std::fs::read(store.dir.join("chat-admissions.bin")).unwrap(),
            scope
        );
        assert_eq!(active_text_lane_count_for_test(store), 0);
        assert_eq!(sdk_entries(), entries);
        host.retry_empty_promotion(&original.layout, &mut NeverRng)
            .unwrap();
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), selected);
    }
}

#[test]
fn actual_host_preserves_wrong_child_instead_of_falling_back_to_scratch() {
    let original = text_fixture(0, 0x7ee11);
    let store = original.layout.checked_store().unwrap();
    let owner = original.owner.borrow_current().unwrap();
    let target = owner.source().source().capture.target.to_path_buf();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    drop(owner);
    let entries = sdk_entries();
    let mut host = PreparedV2TextHost::from_read(
        PreparedV2TextRead::from_original_owner(original.owner),
        host_policy(),
    );
    host.fail_next_promotion_work_for_test(V2TextPromotionTestCut::DirectoryCreated);
    assert!(host
        .retry_empty_promotion(&original.layout, &mut ChaCha20Rng::seed_from_u64(0x7ef11),)
        .is_err());
    let seed = std::fs::read(target.join("text.seed")).unwrap();
    let child = std::fs::read_dir(target.join("text.pages"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("p-")
        })
        .expect("actual directory-page create completed before the injected failure");
    // The public private-page envelope begins with a small big-endian domain length.
    // This byte is therefore a proven WRONG prefix, not an incomplete valid original.
    let foreign_prefix = [0xff];
    std::fs::write(&child, foreign_prefix).unwrap();
    for _ in 0..2 {
        assert!(host
            .retry_empty_promotion(&original.layout, &mut NeverRng)
            .is_err());
        assert_eq!(std::fs::read(&child).unwrap(), foreign_prefix);
        assert_eq!(std::fs::read(target.join("text.seed")).unwrap(), seed);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert!(!target.join("text.seed.stage.v1").exists());
        assert!(!target.join("head.stage").exists());
        assert_eq!(active_text_lane_count_for_test(store), 1);
        assert_eq!(sdk_entries(), entries);
    }
}
