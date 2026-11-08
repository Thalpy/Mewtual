//! Actual selected S -> SAME SDK Header -> ONE owning Core candidate. Neither a
//! candidate nor unchanged O_S supplies a SavedB, accepted projection or publication.
use super::*;
use std::sync::atomic::AtomicUsize;

fn counts() -> Arc<[AtomicUsize; 2]> {
    Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)])
}

fn observed(counts: &[AtomicUsize; 2]) -> [usize; 2] {
    std::array::from_fn(|i| counts[i].load(Ordering::SeqCst))
}

fn selected_s(family: usize, number: u64) -> RawFixture {
    let mut f = raw_fixture(family, number);
    let stage = stage_s(&f);
    f.signature.run_stage(stage).unwrap();
    let commit = commit_s(&f);
    f.signature.run_commit(commit).unwrap();
    validate_saved_s(&f).unwrap();
    f
}

#[test]
fn actual_three_family_candidate_requires_selected_s_and_retries_one_original_without_effects() {
    for family in 0..3 {
        let mut f = raw_fixture(family, 0x7fc00 + family as u64 * 16);
        let calls = counts();
        let _notice = f
            .signature
            .observe_candidate_work_for_test(calls.clone(), false);
        let store = f.original.layout.checked_store().unwrap();
        assert!(f
            .signature
            .materialize_candidate_after_saved_s(store, &f.original.owner)
            .is_err());
        assert_eq!(observed(&calls), [0, 0]);
        assert!(f.signature.verified_candidate_digest_for_test().is_err());
        let stage = stage_s(&f);
        f.signature.run_stage(stage).unwrap();
        assert!(f
            .signature
            .materialize_candidate_after_saved_s(store, &f.original.owner)
            .is_err());
        assert_eq!(observed(&calls), [0, 0]);
        let commit = commit_s(&f);
        f.signature.run_commit(commit).unwrap();
        let target = s_target(&f);
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let pool = store
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .clone();
        let charged = pool.charged_for_test();
        f.signature
            .materialize_candidate_after_saved_s(store, &f.original.owner)
            .unwrap();
        let digest = f.signature.verified_candidate_digest_for_test().unwrap();
        assert_eq!(observed(&calls), [1, 1]);
        for _ in 0..3 {
            f.signature
                .materialize_candidate_after_saved_s(store, &f.original.owner)
                .unwrap();
            assert_eq!(
                f.signature.verified_candidate_digest_for_test().unwrap(),
                digest
            );
            assert_eq!(observed(&calls), [1, 1]);
            assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
            assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
            assert_eq!(pool.charged_for_test(), charged);
        }
        if family == 0 {
            let owner = f.original.owner.borrow_current().unwrap();
            owner.warm_scope().unwrap().revocation().revoke();
            drop(owner);
            assert!(f
                .signature
                .materialize_candidate_after_saved_s(store, &f.original.owner)
                .is_err());
        }
        assert_eq!(
            f.signature.verified_candidate_digest_for_test().unwrap(),
            digest
        );
        assert_eq!(observed(&calls), [1, 1]);
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
        assert_eq!(pool.charged_for_test(), charged);
    }
}

#[test]
fn actual_post_candidate_cancel_keeps_owning_output_and_never_materializes_or_derives_again() {
    let mut f = selected_s(0, 0x7fc40);
    let target = s_target(&f);
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let calls = counts();
    let _notice = f
        .signature
        .observe_candidate_work_for_test(calls.clone(), true);
    let store = f.original.layout.checked_store().unwrap();
    assert!(f
        .signature
        .materialize_candidate_after_saved_s(store, &f.original.owner)
        .is_err());
    let digest = f.signature.verified_candidate_digest_for_test().unwrap();
    assert_eq!(observed(&calls), [1, 1]);
    // The physical selected S remains genuine: cancellation is of the SAME Core
    // projection origin, so this specifically exercises cache-before-late-negative.
    validate_saved_s(&f).unwrap();
    for _ in 0..3 {
        assert!(f
            .signature
            .materialize_candidate_after_saved_s(store, &f.original.owner)
            .is_err());
        assert_eq!(
            f.signature.verified_candidate_digest_for_test().unwrap(),
            digest
        );
        assert_eq!(observed(&calls), [1, 1]);
    }
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    assert!(!f.model_dropped.load(Ordering::SeqCst));
    assert!(!f.sdk_dropped.load(Ordering::SeqCst));
}

#[test]
fn actual_damaged_selected_s_refuses_before_header_or_candidate_and_same_owner_retries_after_repair(
) {
    let mut f = selected_s(0, 0x7fc50);
    let target = s_target(&f);
    let signature = named_artifact(&target, "s-");
    let original = std::fs::read(&signature).unwrap();
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&signature, corrupt).unwrap();
    let calls = counts();
    let _notice = f
        .signature
        .observe_candidate_work_for_test(calls.clone(), false);
    let store = f.original.layout.checked_store().unwrap();
    assert!(f
        .signature
        .materialize_candidate_after_saved_s(store, &f.original.owner)
        .is_err());
    assert_eq!(observed(&calls), [0, 0]);
    assert!(f.signature.verified_candidate_digest_for_test().is_err());
    // Restore only the test's in-place corruption of the original named object.
    // No new receipt, SDK plan, owner or raw signature is introduced by retry.
    std::fs::write(&signature, original).unwrap();
    f.signature
        .materialize_candidate_after_saved_s(store, &f.original.owner)
        .unwrap();
    assert_eq!(observed(&calls), [1, 1]);
    assert!(f.signature.verified_candidate_digest_for_test().is_ok());
}
