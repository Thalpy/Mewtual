//! Genuine selected-B cuts: actual Core dispatch/model ownership, lost returns,
//! outside old-math destruction, distinct final proof and late original revocation.
//! No fixture constructs a guard, candidate, SavedB, phase or accepted basis.
use super::*;
use crate::store::persistence::PreparedV2UnsignedOriginalCustody;
use std::sync::atomic::AtomicUsize;

fn counts() -> Arc<[AtomicUsize; 2]> {
    Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)])
}
fn observed(counts: &[AtomicUsize; 2]) -> [usize; 2] {
    std::array::from_fn(|i| counts[i].load(Ordering::SeqCst))
}
fn selected_b(number: u64) -> AcceptedFixture {
    let mut f = accepted_fixture(0, number);
    let stage = stage_b(&f);
    f.accepted.run_stage(stage).unwrap();
    let commit = commit_b(&f);
    f.accepted.run_commit(commit).unwrap();
    validate_saved_b(&f).unwrap();
    f
}

#[test]
fn actual_lost_core_and_final_returns_keep_one_installed_header_and_require_fresh_final_b() {
    let mut f = selected_b(0x80000);
    let target = b_target(&f);
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let (parent, planned, retired, candidate_present, basis_present) =
        f.accepted.accepted_header_state_for_test().unwrap();
    assert_ne!(parent, planned);
    assert!(retired.is_none() && candidate_present && !basis_present);
    let calls = counts();
    let no_new_plan = counts();
    let _observation =
        f.accepted
            .observe_accepted_advance_work_for_test(calls.clone(), [true, true], None, None);
    let _plans = f
        .accepted
        .observe_candidate_work_for_test(no_new_plan.clone());
    let signs = Arc::new(AtomicUsize::new(0));
    let pool = f
        .original
        .layout
        .checked_store()
        .unwrap()
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let charged = pool.charged_for_test();
    PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        signs.clone(),
        None,
        || {
            // The injected error is AFTER the real Core take and BOTH owning caches.
            assert!(advance_b(&mut f).is_err());
            assert_eq!(observed(&calls), [1, 0]);
            assert_eq!(
                f.accepted.accepted_header_state_for_test().unwrap(),
                (planned, planned, Some(parent), false, false)
            );
            assert!(
                final_b(&mut f).is_err(),
                "the actual old math is still owned"
            );
            for _ in 0..3 {
                advance_b(&mut f).unwrap();
                assert_eq!(observed(&calls), [1, 0]);
                assert_eq!(
                    f.accepted.accepted_header_state_for_test().unwrap(),
                    (planned, planned, Some(parent), false, false)
                );
            }

            // There is no Current writer in this scope. The actual retired parent
            // observed above is destroyed, while its original model/SDK fees stay.
            f.accepted.drain_accepted_retired_math_if_present().unwrap();
            assert_eq!(
                f.accepted.accepted_header_state_for_test().unwrap(),
                (planned, planned, None, false, false)
            );
            assert!(!f.model_dropped.load(Ordering::SeqCst));
            assert!(!f.sdk_dropped.load(Ordering::SeqCst));

            // Damage a genuine NEW B object AFTER the drain. Its native identity
            // and length stay unchanged; final must independently reprove its bytes.
            let path = f
                .accepted
                .accepted_current_artifact_path_for_test()
                .unwrap();
            let original = std::fs::read(&path).unwrap();
            let native =
                catcoms_storage::kept::session_id(&std::fs::File::open(&path).unwrap()).unwrap();
            let mut corrupt = original.clone();
            *corrupt.last_mut().unwrap() ^= 1;
            std::fs::write(&path, &corrupt).unwrap();
            assert_eq!(
                catcoms_storage::kept::session_id(&std::fs::File::open(&path).unwrap()).unwrap(),
                native
            );
            assert_eq!(
                std::fs::metadata(&path).unwrap().len(),
                original.len() as u64
            );
            assert!(final_b(&mut f).is_err());
            assert_eq!(observed(&calls), [1, 0]);
            assert!(f.accepted.accepted_final_basis_data().is_err());
            std::fs::write(&path, original).unwrap();

            // A new genuine final proof succeeds, then its actual cached reply is
            // lost. The same historical basis survives and the next return reuses it.
            assert!(final_b(&mut f).is_err());
            assert_eq!(observed(&calls), [1, 1]);
            assert_eq!(
                f.accepted.accepted_header_state_for_test().unwrap(),
                (planned, planned, None, false, true)
            );
            let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
            basis.validate_negative().unwrap();
            for _ in 0..3 {
                final_b(&mut f).unwrap();
                advance_b(&mut f).unwrap();
                basis
                    .require_same_original(f.accepted.accepted_final_basis_data().unwrap())
                    .unwrap();
                assert_eq!(observed(&calls), [1, 1]);
                assert_eq!(observed(&no_new_plan), [0, 0]);
                assert_eq!(signs.load(Ordering::SeqCst), 0);
                assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                assert_eq!(pool.charged_for_test(), charged);
            }
        },
    );
}

#[test]
fn actual_post_core_original_domain_or_scope_close_keeps_installed_model_and_allows_only_real_drain(
) {
    for close_scope in [false, true] {
        let mut f = selected_b(0x80020 + u64::from(close_scope));
        let target = b_target(&f);
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let (parent, planned, _, _, _) = f.accepted.accepted_header_state_for_test().unwrap();
        let calls = counts();
        let no_new_plan = counts();
        let scope = close_scope.then(|| {
            f.original
                .owner
                .borrow_current()
                .unwrap()
                .warm_scope()
                .unwrap()
                .revocation()
        });
        let _observation = f.accepted.observe_accepted_advance_work_for_test(
            calls.clone(),
            [false, false],
            (!close_scope).then_some(0),
            scope.map(|scope| (0, scope)),
        );
        let _plans = f
            .accepted
            .observe_candidate_work_for_test(no_new_plan.clone());
        let signs = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            signs.clone(),
            None,
            || {
                assert!(advance_b(&mut f).is_err());
                assert_eq!(observed(&calls), [1, 0]);
                assert_eq!(
                    f.accepted.accepted_header_state_for_test().unwrap(),
                    (planned, planned, Some(parent), false, false)
                );
                // Scope close must really invalidate Current. Domain-only close
                // leaves real physical Current intact but revokes its Core origin.
                assert_eq!(validate_saved_b(&f).is_err(), close_scope);
                assert!(advance_b(&mut f).is_err());
                assert!(final_b(&mut f).is_err());
                f.accepted.drain_accepted_retired_math_if_present().unwrap();
                f.accepted.drain_accepted_retired_math_if_present().unwrap();
                assert_eq!(
                    f.accepted.accepted_header_state_for_test().unwrap(),
                    (planned, planned, None, false, false)
                );
                assert!(advance_b(&mut f).is_err());
                assert!(final_b(&mut f).is_err());
                assert!(f.accepted.accepted_final_basis_data().is_err());
                assert_eq!(observed(&calls), [1, 0]);
                assert_eq!(observed(&no_new_plan), [0, 0]);
                assert_eq!(signs.load(Ordering::SeqCst), 0);
                assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                assert!(!f.model_dropped.load(Ordering::SeqCst));
                assert!(!f.sdk_dropped.load(Ordering::SeqCst));
            },
        );
    }
}

#[test]
fn actual_post_final_original_domain_or_scope_close_preserves_basis_but_never_renews_its_origin() {
    for close_scope in [false, true] {
        let mut f = selected_b(0x80040 + u64::from(close_scope));
        let target = b_target(&f);
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let (parent, planned, _, _, _) = f.accepted.accepted_header_state_for_test().unwrap();
        let calls = counts();
        let no_new_plan = counts();
        let scope = close_scope.then(|| {
            f.original
                .owner
                .borrow_current()
                .unwrap()
                .warm_scope()
                .unwrap()
                .revocation()
        });
        let _observation = f.accepted.observe_accepted_advance_work_for_test(
            calls.clone(),
            [false, false],
            (!close_scope).then_some(1),
            scope.map(|scope| (1, scope)),
        );
        let _plans = f
            .accepted
            .observe_candidate_work_for_test(no_new_plan.clone());
        let signs = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            signs.clone(),
            None,
            || {
                advance_b(&mut f).unwrap();
                assert_eq!(
                    f.accepted.accepted_header_state_for_test().unwrap(),
                    (planned, planned, Some(parent), false, false)
                );
                assert!(
                    final_b(&mut f).is_err(),
                    "retired payload still blocks final"
                );
                f.accepted.drain_accepted_retired_math_if_present().unwrap();
                assert!(final_b(&mut f).is_err());
                assert_eq!(observed(&calls), [1, 1]);
                assert_eq!(
                    f.accepted.accepted_header_state_for_test().unwrap(),
                    (planned, planned, None, false, true)
                );
                let basis = f.accepted.accepted_final_basis_data().unwrap().clone();
                assert!(basis.validate_negative().is_err());
                assert_eq!(validate_saved_b(&f).is_err(), close_scope);
                f.accepted.drain_accepted_retired_math_if_present().unwrap();
                for _ in 0..3 {
                    assert!(advance_b(&mut f).is_err());
                    assert!(final_b(&mut f).is_err());
                    basis
                        .require_same_original(f.accepted.accepted_final_basis_data().unwrap())
                        .unwrap();
                    assert!(basis.validate_negative().is_err());
                    assert_eq!(observed(&calls), [1, 1]);
                    assert_eq!(observed(&no_new_plan), [0, 0]);
                    assert_eq!(signs.load(Ordering::SeqCst), 0);
                }
                assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
                assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
                assert!(!f.model_dropped.load(Ordering::SeqCst));
                assert!(!f.sdk_dropped.load(Ordering::SeqCst));
            },
        );
    }
}
