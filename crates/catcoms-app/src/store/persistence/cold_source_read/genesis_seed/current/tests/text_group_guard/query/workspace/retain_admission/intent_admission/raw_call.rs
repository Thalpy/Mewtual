//! Actual selected-I -> closed Physical delegate -> Root -> bounded SDK key path.
//! The counter observes real dispatch; returned raw is independently verified by the
//! owning assertion helper. No test supplies a signature or calls an unguarded signer.
use super::*;
use crate::store::persistence::v2_text::SavedV2UnsignedAppendIntent;
use std::sync::atomic::AtomicUsize;

pub(super) fn selected_i_fixture(
    family: usize,
    number: u64,
) -> (IntentFixture, Arc<SavedV2UnsignedAppendIntent>) {
    let mut f = intent_fixture(family, number);
    let stage = stage_i(&f);
    f.intent.run_stage(stage).unwrap();
    let commit = commit_i(&f);
    f.intent.run_commit(commit).unwrap();
    let saved = Arc::clone(f.intent.saved_unsigned_intent().unwrap());
    (f, saved)
}

pub(super) fn sign_i(
    f: &mut IntentFixture,
    saved: &SavedV2UnsignedAppendIntent,
) -> Result<(), AppError> {
    let store = f.original.layout.checked_store()?;
    let owner = f.original.owner.borrow_current()?;
    let resources = f.intent.resources().clone();
    store.with_v2_text_current(&owner, |admission| {
        let mut current = store.lock_v2_text_current_group(admission, &resources)?;
        // The real SavedI Arc is outside the mutable intent. The temporary PreparedI
        // loan ends at guard construction, before the whole-custody raw cache changes.
        let signing = current.admit_unsigned_signing(&f.lane, saved, &f.intent)?;
        f.intent
            .sign_reserved_original(store, &f.original.owner, &signing)
    })
}

#[test]
fn actual_three_family_raw_key_dispatch_occurs_once_and_repeat_reuses_same_valid_original() {
    for family in 0..3 {
        let (mut f, saved) = selected_i_fixture(family, 0x7fa00 + family as u64 * 16);
        let target = i_target(&f);
        let head = std::fs::read(target.join("head.bin")).unwrap();
        let wal = std::fs::read(target.join("text.wal")).unwrap();
        let dispatches = Arc::new(AtomicUsize::new(0));
        assert!(f.intent.existing_raw_signature_data().is_err());
        PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
            dispatches.clone(),
            None,
            || {
                // Discard this reply as a caller with an uncertain/lost response may.
                // Recovery re-enters the same real guarded path, not an SDK getter.
                sign_i(&mut f, &saved).unwrap();
                f.intent.assert_existing_raw_valid_for_test();
                let original = *f.intent.existing_raw_signature_data().unwrap();
                for _ in 0..3 {
                    sign_i(&mut f, &saved).unwrap();
                    assert_eq!(f.intent.existing_raw_signature_data().unwrap(), &original);
                }
            },
        );
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
        assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
        assert!(!f.model_dropped.load(Ordering::SeqCst));
        assert!(!f.sdk_dropped.load(Ordering::SeqCst));
    }
}

#[test]
fn actual_raw_key_refuses_same_file_wi_corruption_and_reserved_stages_before_any_dispatch() {
    let (mut f, saved) = selected_i_fixture(0, 0x7fa40);
    let target = i_target(&f);
    let wal_path = target.join("text.wal");
    let wal = std::fs::read(&wal_path).unwrap();
    let native =
        catcoms_storage::kept::session_id(&std::fs::File::open(&wal_path).unwrap()).unwrap();
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let dispatches = Arc::new(AtomicUsize::new(0));
    PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        dispatches.clone(),
        None,
        || {
            let mut damaged = wal.clone();
            let middle = damaged.len() / 2;
            damaged[middle] ^= 0x20;
            std::fs::write(&wal_path, &damaged).unwrap();
            assert_eq!(
                catcoms_storage::kept::session_id(&std::fs::File::open(&wal_path).unwrap())
                    .unwrap(),
                native,
            );
            assert_eq!(
                std::fs::metadata(&wal_path).unwrap().len(),
                wal.len() as u64
            );
            assert!(sign_i(&mut f, &saved).is_err());
            assert!(f.intent.existing_raw_signature_data().is_err());
            assert_eq!(dispatches.load(Ordering::SeqCst), 0);
            assert_eq!(std::fs::read(&wal_path).unwrap(), damaged);
            std::fs::write(&wal_path, &wal).unwrap();
            for name in ["text.wal.stage.v1", "head.stage", "text.seed.stage.v1"] {
                let path = target.join(name);
                std::fs::write(&path, b"foreign pending work").unwrap();
                assert!(sign_i(&mut f, &saved).is_err());
                assert!(f.intent.existing_raw_signature_data().is_err());
                assert_eq!(dispatches.load(Ordering::SeqCst), 0);
                assert_eq!(std::fs::read(&path).unwrap(), b"foreign pending work");
                std::fs::remove_file(path).unwrap();
            }
            sign_i(&mut f, &saved).unwrap();
            f.intent.assert_existing_raw_valid_for_test();
        },
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(&wal_path).unwrap(), wal);
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert!(!f.lane.workspace_active_for_test());
}

#[test]
fn actual_raw_key_refuses_other_genuine_saved_i_and_independent_domain_stop_before_dispatch() {
    let (mut a, saved_a) = selected_i_fixture(0, 0x7fa50);
    let (mut b, saved_b) = selected_i_fixture(0, 0x7fa60);
    let dispatches = Arc::new(AtomicUsize::new(0));
    PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
        dispatches.clone(),
        None,
        || {
            assert!(sign_i(&mut a, &saved_b).is_err());
            assert!(sign_i(&mut b, &saved_a).is_err());
            assert!(a.intent.existing_raw_signature_data().is_err());
            assert!(b.intent.existing_raw_signature_data().is_err());
            assert_eq!(dispatches.load(Ordering::SeqCst), 0);
            a.intent.cancel_original_projection_domain_for_test();
            assert!(sign_i(&mut a, &saved_a).is_err());
            assert!(a.intent.existing_raw_signature_data().is_err());
            assert_eq!(dispatches.load(Ordering::SeqCst), 0);
            // This is domain-only Stop: SAME actual Current remains live.
            let store = a.original.layout.checked_store().unwrap();
            let owner = a.original.owner.borrow_current().unwrap();
            store.with_v2_text_current(&owner, |_| Ok(())).unwrap();
            sign_i(&mut b, &saved_b).unwrap();
            b.intent.assert_existing_raw_valid_for_test();
        },
    );
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
}

#[test]
fn actual_post_key_domain_cancel_keeps_same_valid_raw_and_never_redispatches() {
    let (mut f, saved) = selected_i_fixture(0, 0x7fa70);
    let target = i_target(&f);
    let head = std::fs::read(target.join("head.bin")).unwrap();
    let wal = std::fs::read(target.join("text.wal")).unwrap();
    let dispatches = Arc::new(AtomicUsize::new(0));
    {
        // The actual owning family installs SAME-domain cancellation after SDK has
        // cache-stored raw and before Root's post-key negatives. No domain escapes.
        let _notice = f
            .intent
            .observe_domain_cancel_after_sign_for_test(dispatches.clone());
        assert!(sign_i(&mut f, &saved).is_err());
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        f.intent.assert_existing_raw_valid_for_test();
        let original = *f.intent.existing_raw_signature_data().unwrap();
        assert!(sign_i(&mut f, &saved).is_err());
        assert_eq!(f.intent.existing_raw_signature_data().unwrap(), &original);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }
    let store = f.original.layout.checked_store().unwrap();
    let owner = f.original.owner.borrow_current().unwrap();
    store.with_v2_text_current(&owner, |_| Ok(())).unwrap();
    assert_eq!(std::fs::read(target.join("head.bin")).unwrap(), head);
    assert_eq!(std::fs::read(target.join("text.wal")).unwrap(), wal);
    assert!(!f.model_dropped.load(Ordering::SeqCst));
    assert!(!f.sdk_dropped.load(Ordering::SeqCst));
}
