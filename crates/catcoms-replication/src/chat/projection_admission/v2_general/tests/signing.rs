//! Actual production plan issuer. These HOST selections are explicit Core trust fixtures,
//! not real native Source/UI admission or evidence of the later Sync signer implementation.
use super::*;
use crate::chat::ActionDraft;

fn unsigned_draft(
    key: &DeviceKeypair,
    admission: &V2AuthenticatedAdmission,
    nonce: u8,
    body: Body,
    mut parents: Vec<ActionId>,
) -> ActionDraft {
    let realm = Realm::new(GROUP, 42).unwrap();
    let labels = admission.admission_data().ordinary_labels(&realm).unwrap();
    parents.sort();
    parents.dedup();
    ActionDraft::new(
        ActionSpec {
            realm,
            basis: labels.basis,
            grant: labels.grant,
            nonce: [nonce; 16],
            presented_ms: u64::MAX,
            parents,
            body,
        },
        key.verifying_key().to_bytes(),
    )
    .unwrap()
}
fn create_draft(key: &DeviceKeypair, a: &V2AuthenticatedAdmission, n: u8) -> ActionDraft {
    unsigned_draft(
        key,
        a,
        n,
        Body::Create {
            text: format!("draft {n}"),
            reply_to: None,
        },
        vec![],
    )
}

#[test]
fn original_admission_id_remains_historical_data_after_remove_and_source_stop() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[175; 32]);
    let admission = f.add(&key, 75);
    let draft = create_draft(&key, &admission, 1);
    let (_ledger, view) = f.ledger();
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
        .unwrap();
    let expected = admission.admission_data().admission_id();
    assert_eq!(plan.admission_id_data(), expected);
    let mut removed = f.remove(&admission);
    assert!(removed.take_close_completion().unwrap().try_complete());
    assert_eq!(plan.admission_id_data(), expected);
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .is_err());
    f.negative.0.store(false, Ordering::Release);
    assert_eq!(plan.admission_id_data(), expected);
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .is_err());
}

#[test]
fn actual_general_prekey_issuer_supports_reply_own_edit_delete_and_join_with_real_dependencies() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[130; 32]);
    let admission = f.add(&key, 60);
    let (mut ledger, view) = f.ledger();
    let target = create(&key, &admission, 10);
    retain(&f, &mut ledger, &target, &admission, &[]);
    install(&f, &mut ledger, &view, &target, &admission, &[]);
    let owner_calls = f.owner.calls.get();
    let mut signer_calls = 0;
    for (index, body) in [
        Body::Create {
            text: "reply from member".into(),
            reply_to: Some(target.id()),
        },
        Body::Edit {
            create: target.id(),
            text: "member edit".into(),
        },
        Body::Delete {
            create: target.id(),
        },
        Body::Join {
            create: target.id(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let draft = unsigned_draft(&key, &admission, index as u8, body, vec![target.id()]);
        let deps = [V2OrdinaryDependency::new(&target, &admission)];
        let mut plan = f
            .verifier
            .prepare_ordinary_signing_plan(&draft, &admission, &deps, &view)
            .unwrap();
        let original = plan.observe_original();
        let call = plan
            .admit_signing(&draft, &key.verifying_key().to_bytes())
            .unwrap()
            .begin_key()
            .unwrap();
        signer_calls += 1;
        let raw = key.sign(call.signing_bytes_data());
        drop(call.cache_raw64(raw));
        let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
        retain(&f, &mut ledger, &signed, &admission, &deps);
        install(&f, &mut ledger, &view, &signed, &admission, &deps);
        assert_eq!(signed.semantic_bytes(), draft.signing_bytes_data());
    }
    assert_eq!(signer_calls, 4);
    assert_eq!(f.owner.calls.get(), owner_calls);
}

#[test]
fn actual_maximum_parent_set_checks_sixty_five_real_signed_dependencies() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[129; 32]);
    let admission = f.add(&key, 59);
    let (_ledger, view) = f.ledger();
    let target = create(&key, &admission, 1);
    let owner_calls = f.owner.calls.get();
    let joins: Vec<_> = (0..MAX_PARENTS)
        .map(|i| {
            action(
                &key,
                &admission,
                (i + 2) as u8,
                Body::Join {
                    create: target.id(),
                },
                vec![target.id()],
            )
        })
        .collect();
    let mut parents = Vec::with_capacity(MAX_PARENTS);
    parents.extend(joins.iter().map(SignedAction::id));
    parents.sort();
    let draft = unsigned_draft(
        &key,
        &admission,
        0,
        Body::Edit {
            create: target.id(),
            text: "wide exact dependency set".into(),
        },
        parents,
    );
    let mut dependencies: Vec<_> = joins.iter().map(|join| (join, &admission)).collect();
    dependencies.push((&target, &admission));
    let dependencies = ordered_dependencies(dependencies);
    assert_eq!(dependencies.len(), MAX_PARENTS + 1);
    // Exercise the actual shared clone+insert geometry that the transient invoice names.
    let required = crate::chat::action::dependencies_spec(draft.spec_data());
    assert_eq!(required.len(), MAX_PARENTS + 1);
    assert!(required.capacity() <= 2 * (MAX_PARENTS + 1));
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &admission, &dependencies, &view)
        .unwrap();
    let original = plan.observe_original();
    let call = plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .unwrap();
    let raw = key.sign(call.signing_bytes_data());
    drop(call.cache_raw64(raw));
    let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert!(f
        .verifier
        .verify_action_semantics(&signed, &admission, &dependencies)
        .is_ok());
    assert_eq!(f.owner.calls.get(), owner_calls);
}

#[test]
fn actual_member_plan_caches_one_real_signature_and_retries_never_enter_key_or_owner_again() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[131; 32]);
    let admission = f.add(&key, 61);
    let (mut ledger, view) = f.ledger();
    let draft = create_draft(&key, &admission, 1);
    let owner_calls = f.owner.calls.get();
    let mut signer_calls = 0;
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
        .unwrap();
    let original = plan.observe_original();
    assert!(
        f.verifier
            .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
            .is_err(),
        "one original draft, one non-Clone plan"
    );
    let guard = plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .unwrap();
    assert!(guard.cached_raw64_data().is_none());
    let call = guard.begin_key().unwrap();
    signer_calls += 1;
    let raw = key.sign(call.signing_bytes_data());
    let guard = call.cache_raw64(raw);
    assert_eq!(guard.cached_raw64_data(), Some(&raw));
    drop(guard);
    // Simulated lost outer return: retry sees the actual cached result, not a success flag.
    for _ in 0..3 {
        let guard = plan
            .admit_signing(&draft, &key.verifying_key().to_bytes())
            .unwrap();
        assert_eq!(guard.cached_raw64_data(), Some(&raw));
        assert!(guard.begin_key().is_err());
    }
    let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert_eq!(signed.id(), draft.action_id_data());
    assert!(signed.verify());
    retain(&f, &mut ledger, &signed, &admission, &[]);
    install(&f, &mut ledger, &view, &signed, &admission, &[]);
    assert_eq!(signer_calls, 1);
    assert_eq!(f.owner.calls.get(), owner_calls);
    drop(plan);
    assert!(
        f.verifier
            .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
            .is_err(),
        "dropping original plan cannot renew a key slot"
    );
}

#[test]
fn actual_plan_wrong_draft_full_key_group_view_and_signed_dependency_refuse_before_entry() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[132; 32]);
    let a = f.add(&key, 62);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &a, 1);
    let equal = create_draft(&key, &a, 1);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &a, &[], &view)
        .unwrap();
    assert!(plan
        .admit_signing(&equal, &key.verifying_key().to_bytes())
        .is_err());
    assert!(plan.admit_signing(&draft, &[0; 32]).is_err());
    assert!(plan
        .borrow_cached_raw64_data(&plan.observe_original(), &draft)
        .unwrap()
        .is_none());
    let foreign = Fixture::new();
    let (_foreign_ledger, foreign_view) = foreign.ledger();
    let other = create_draft(&key, &a, 2);
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&other, &a, &[], &foreign_view)
        .is_err());
    let mut wrong_spec = other.spec_data().clone();
    wrong_spec.grant = GrantId::from_bytes([0; 32]);
    let wrong = ActionDraft::new(wrong_spec, key.verifying_key().to_bytes()).unwrap();
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&wrong, &a, &[], &view)
        .is_err());
    let target = create(&key, &a, 3);
    let edit = unsigned_draft(
        &key,
        &a,
        4,
        Body::Edit {
            create: target.id(),
            text: "edit".into(),
        },
        vec![target.id()],
    );
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&edit, &a, &[], &view)
        .is_err());
    let mut encoded = target.encode();
    *encoded.last_mut().unwrap() ^= 1;
    let bad = SignedAction::decode(&encoded).unwrap();
    assert!(!bad.verify());
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&edit, &a, &[V2OrdinaryDependency::new(&bad, &a)], &view)
        .is_err());
    let deps = [V2OrdinaryDependency::new(&target, &a)];
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&edit, &a, &deps, &view)
        .is_ok());
    let extra = create(&key, &a, 5);
    let extras = ordered_dependencies(vec![(&target, &a), (&extra, &a)]);
    let next_edit = unsigned_draft(
        &key,
        &a,
        6,
        Body::Edit {
            create: target.id(),
            text: "edit".into(),
        },
        vec![target.id()],
    );
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&next_edit, &a, &extras, &view)
        .is_err());
}

#[test]
fn actual_signing_reader_delays_remove_completion_and_late_raw_stays_historical_data() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[133; 32]);
    let a1 = f.add(&key, 63);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &a1, 1);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &a1, &[], &view)
        .unwrap();
    let original = plan.observe_original();
    let call = plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .unwrap();
    let mut removed = f.remove(&a1);
    let mut completion = removed.take_close_completion().unwrap();
    assert!(
        !completion.try_complete(),
        "Remove selection proceeds but actual signing reader still holds group drain"
    );
    let raw = key.sign(call.signing_bytes_data());
    let guard = call.cache_raw64(raw);
    assert_eq!(guard.cached_raw64_data(), Some(&raw));
    assert!(!completion.try_complete());
    drop(guard);
    assert!(completion.try_complete());
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .is_err());
    let signed = plan.complete_cached_raw64_data(&original, &draft).unwrap();
    assert!(signed.verify());
    assert!(f
        .verifier
        .verify_action_semantics(&signed, &a1, &[])
        .is_ok());
    assert!(f
        .verifier
        .provisional_action(&signed, &a1, &[], &view)
        .is_err());
    drop(removed);
    let a2 = f.add(&key, 64);
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &a2, &[], &view)
        .is_err());
    let fresh = create_draft(&key, &a2, 2);
    assert!(f
        .verifier
        .prepare_ordinary_signing_plan(&fresh, &a2, &[], &view)
        .is_ok());
}

#[test]
fn actual_remove_before_signing_and_original_source_or_view_close_prevent_any_key_entry() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[134; 32]);
    let a = f.add(&key, 65);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &a, 1);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &a, &[], &view)
        .unwrap();
    let mut removed = f.remove(&a);
    assert!(removed.take_close_completion().unwrap().try_complete());
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .is_err());
    assert!(plan
        .borrow_cached_raw64_data(&plan.observe_original(), &draft)
        .unwrap()
        .is_none());
    drop(removed);
    let key2 = DeviceKeypair::from_seed(&[135; 32]);
    let a2 = f.add(&key2, 66);
    let draft2 = create_draft(&key2, &a2, 2);
    let mut plan2 = f
        .verifier
        .prepare_ordinary_signing_plan(&draft2, &a2, &[], &view)
        .unwrap();
    f.negative.0.store(false, Ordering::Release);
    assert!(plan2
        .admit_signing(&draft2, &key2.verifying_key().to_bytes())
        .is_err());
    assert!(plan2
        .borrow_cached_raw64_data(&plan2.observe_original(), &draft2)
        .unwrap()
        .is_none());
    let mut other = Fixture::new();
    let a3 = other.add(&key2, 67);
    let (_other_ledger, view3) = other.ledger();
    let draft3 = create_draft(&key2, &a3, 3);
    let mut plan3 = other
        .verifier
        .prepare_ordinary_signing_plan(&draft3, &a3, &[], &view3)
        .unwrap();
    let mut close = view3.revoker().close();
    assert!(close.try_complete());
    assert!(plan3
        .admit_signing(&draft3, &key2.verifying_key().to_bytes())
        .is_err());
}

#[test]
fn actual_lost_key_return_is_terminal_and_fork_close_waits_for_existing_key_cache() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[136; 32]);
    let a = f.add(&key, 68);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &a, 1);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &a, &[], &view)
        .unwrap();
    let call = plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .unwrap();
    let _lost_raw = key.sign(call.signing_bytes_data());
    drop(call);
    assert!(plan
        .admit_signing(&draft, &key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .is_err());
    assert!(plan
        .complete_cached_raw64_data(&plan.observe_original(), &draft)
        .is_err());
    let next = create_draft(&key, &a, 2);
    let mut pending = f
        .verifier
        .prepare_ordinary_signing_plan(&next, &a, &[], &view)
        .unwrap();
    let original = pending.observe_original();
    let call = pending
        .admit_signing(&next, &key.verifying_key().to_bytes())
        .unwrap()
        .begin_key()
        .unwrap();
    let base = f.verifier.replay_data();
    let mut spec = base.header().spec().clone();
    spec.parent = base.header().id();
    spec.sequence += 1;
    spec.body = OwnerBodyId::from_bytes([70; 32]);
    let first = SignedOwnerHeader::sign_using(spec.clone(), |b| Ok(f.owner.key.sign(b))).unwrap();
    f.verifier.observe_current_header(&first).unwrap();
    spec.body = OwnerBodyId::from_bytes([71; 32]);
    let second = SignedOwnerHeader::sign_using(spec, |b| Ok(f.owner.key.sign(b))).unwrap();
    assert!(f.verifier.observe_current_header(&second).is_err());
    assert!(!f
        .verifier
        .fork_close_completion_mut()
        .unwrap()
        .try_complete());
    let raw = key.sign(call.signing_bytes_data());
    let cached = call.cache_raw64(raw);
    drop(cached);
    assert!(f
        .verifier
        .fork_close_completion_mut()
        .unwrap()
        .try_complete());
    assert!(pending
        .complete_cached_raw64_data(&original, &next)
        .unwrap()
        .verify());
    assert!(pending
        .admit_signing(&next, &key.verifying_key().to_bytes())
        .is_err());
}

#[test]
fn admitted_guard_names_the_exact_draft_signer_and_dropping_it_keeps_the_key_entry_unused() {
    let mut f = Fixture::new();
    let key = DeviceKeypair::from_seed(&[176; 32]);
    let admission = f.add(&key, 76);
    let (_ledger, view) = f.ledger();
    let draft = create_draft(&key, &admission, 9);
    let mut plan = f
        .verifier
        .prepare_ordinary_signing_plan(&draft, &admission, &[], &view)
        .unwrap();
    let original = plan.observe_original();
    let full = key.verifying_key().to_bytes();
    {
        // A Sync signer compares this getter with its retained key before begin_key. A
        // refusal at that point drops the guard; the Plan must remain Prepared.
        let guard = plan.admit_signing(&draft, &full).unwrap();
        assert_eq!(guard.signer_data(), &full);
        assert_eq!(guard.signer_data(), draft.signer_data());
        assert!(guard.spec_data() == draft.spec_data());
        assert!(guard.cached_raw64_data().is_none());
    }
    let call = plan
        .admit_signing(&draft, &full)
        .unwrap()
        .begin_key()
        .unwrap();
    let cached = call.cache_raw64(key.sign(draft.signing_bytes_data()));
    // After caching, the same getter is still the original signer and raw is present.
    assert_eq!(cached.signer_data(), &full);
    assert!(cached.cached_raw64_data().is_some());
    drop(cached);
    assert!(plan
        .complete_cached_raw64_data(&original, &draft)
        .unwrap()
        .verify());
}
