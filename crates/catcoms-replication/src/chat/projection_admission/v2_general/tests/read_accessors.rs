//! The three joined-source read accessors are OBSERVATIONS of existing private state. These
//! tests pin that each one follows the actual production verifier/Plan state it reads, that a
//! closed life never reads as open again, and that reading admits or changes nothing. HOST
//! selections here are the fixture's explicit trust decisions, as in the sibling tests.
use super::*;
use crate::chat::ActionDraft;

fn create_draft(key: &DeviceKeypair, admission: &V2AuthenticatedAdmission, n: u8) -> ActionDraft {
    let realm = Realm::new(GROUP, 42).unwrap();
    let labels = admission.admission_data().ordinary_labels(&realm).unwrap();
    ActionDraft::new(
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce: [n; 16],
            presented_ms: u64::MAX,
            parents: Vec::new(),
            body: Body::Create {
                text: format!("accessor {n}"),
                reply_to: None,
            },
        },
        key.verifying_key().to_bytes(),
    )
    .unwrap()
}

#[test]
fn mls_epoch_data_follows_only_selected_membership_steps() {
    let mut f = Fixture::new();
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), 0);
    let key = DeviceKeypair::from_seed(&[181; 32]);
    let admission = f.add(&key, 81);
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), 1);
    // The accessor reports exactly the selected image's own encoded epoch.
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), f.epoch());
    // An Append is not a membership transition: the epoch stays where it was.
    let accepted = create(&key, &admission, 11);
    drop(f.append_one(&admission, &accepted));
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), 1);
    // A Removal advances by exactly one, like an Add.
    drop(f.remove(&admission));
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), 2);
    assert_eq!(f.verifier.replay_data().mls_epoch_data(), f.epoch());
}

#[test]
fn is_open_data_tracks_removal_and_source_stop_and_never_reopens() {
    let mut f = Fixture::new();
    let removed_key = DeviceKeypair::from_seed(&[182; 32]);
    let survivor_key = DeviceKeypair::from_seed(&[183; 32]);
    let removed = f.add(&removed_key, 82);
    let survivor = f.add(&survivor_key, 84);
    assert!(removed.is_open_data());
    assert!(survivor.is_open_data());
    // A selected Remove closes exactly that Admission's life, immediately.
    drop(f.remove(&removed));
    assert!(!removed.is_open_data());
    assert!(survivor.is_open_data());
    // A historical handle fetched before removal and the verifier's own lookup agree.
    assert!(f
        .verifier
        .admission(removed.admission_data().admission_id())
        .is_err());
    // Stopping the original host negative closes every remaining Admission...
    f.negative.0.store(false, Ordering::Release);
    assert!(!survivor.is_open_data());
    // ...and a later `true` from the host cannot reopen either one: the origin latched.
    f.negative.0.store(true, Ordering::Release);
    assert!(!survivor.is_open_data());
    assert!(!removed.is_open_data());
}

#[test]
fn require_open_data_reads_the_plans_original_lifetimes_and_admits_nothing() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[184; 32]);
    let other_key = DeviceKeypair::from_seed(&[185; 32]);
    let admission = f.add(&key, 86);
    let survivor = f.add(&other_key, 88);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &admission, 4);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
        .unwrap();
    let original = plan.observe_original();
    let survivor_draft = create_draft(&other_key, &survivor, 5);
    let survivor_plan = f
        .verifier
        .prepare_ordinary_signing_plan(&survivor_draft, &survivor, &[], &view)
        .unwrap();
    assert!(plan.require_open_data().is_ok());
    // Reading entered nothing: no raw exists and the one key entry is still available.
    assert!(plan
        .borrow_cached_raw64_data(&original, &draft)
        .unwrap()
        .is_none());
    drop(
        plan.admit_signing(&draft, &key.verifying_key().to_bytes())
            .unwrap(),
    );
    // Removing the Plan's own Admission closes this Plan's OPEN life only.
    drop(f.remove(&admission));
    assert!(plan.require_open_data().is_err());
    assert!(survivor_plan.require_open_data().is_ok());
    // The read agrees with the real admission predicate.
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .is_err());
    // The Plan's historical label survives; only its present lifetime closed.
    assert_eq!(
        plan.admission_id_data(),
        admission.admission_data().admission_id()
    );
    // A source stop closes the surviving Plan too, and it stays closed.
    f.negative.0.store(false, Ordering::Release);
    assert!(survivor_plan.require_open_data().is_err());
    f.negative.0.store(true, Ordering::Release);
    assert!(survivor_plan.require_open_data().is_err());
}
