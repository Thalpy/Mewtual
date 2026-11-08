//! Two actual founder Creates separated by genuine first WC/lane retirement and
//! complete old debt disposal. Request two moves the SAME advanced model; no G
//! replay, raw model adopter, invented SavedA or token-row authority is used.
use super::*;
use crate::store::critical_namespaces::current_layout::{
    PreparedV2TextAcceptedQueryCapture, PreparedV2TextAcceptedQueryLease,
};
use crate::store::persistence::v2_text::{
    PreparedV2TextAcceptedQuery, PreparedV2TextRawSignature, PreparedV2TextUnsignedIntent,
    V2TextAcceptedQueryResultData,
};
use crate::store::persistence::{PreparedV2NextUnsignedAppend, PreparedV2UnsignedOriginalCustody};
use std::sync::atomic::AtomicUsize;

fn accepted_miss(t: &NextCreateFixture) -> PreparedV2TextAcceptedQuery {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let mut query = PreparedV2TextAcceptedQuery::prepare(
        store,
        owner.source(),
        t.transaction.as_ref().unwrap(),
    )
    .unwrap();
    let mut reader =
        PreparedV2TextAcceptedQueryLease::prepare(store, owner.source(), &query).unwrap();
    let mut capture =
        Some(PreparedV2TextAcceptedQueryCapture::prepare(store, owner.source(), &query).unwrap());
    let resources = query.resources().clone();
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_accepted_query_into(
                &mut query,
                &mut reader,
                &mut capture,
                &mut admitted,
            )
        })
        .unwrap();
    reader.drain_unused_outside_writers();
    query.run(admitted.take().unwrap()).unwrap();
    assert!(matches!(
        query.result_data().unwrap(),
        V2TextAcceptedQueryResultData::Absent
    ));
    query
}
fn prepare_unsigned(
    t: &mut NextCreateFixture,
    query: &PreparedV2TextAcceptedQuery,
    next: &mut Option<PreparedV2NextUnsignedAppend>,
) -> Result<(), AppError> {
    PreparedV2NextUnsignedAppend::prepare_after_saved_a_into(
        t.original.layout.checked_store()?,
        &t.original.owner,
        t.transaction.as_ref().unwrap(),
        query,
        &mut t.resident,
        next,
    )
}
fn select_i(t: &NextCreateFixture, intent: &mut PreparedV2TextUnsignedIntent) {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let resources = intent.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_intent_stage_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                intent,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    intent.run_stage(admitted.take().unwrap()).unwrap();
    let ready = intent.ready_selection().unwrap();
    let mut admitted = None;
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_intent_commit_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                ready,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    intent.run_commit(admitted.take().unwrap()).unwrap();
}
fn sign_i(t: &NextCreateFixture, intent: &mut PreparedV2TextUnsignedIntent) {
    let saved = intent.saved_unsigned_intent().unwrap().clone();
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let resources = intent.resources().clone();
    for _ in 0..3 {
        store
            .with_v2_text_current(&owner, |a| {
                let mut guard = store.lock_v2_text_current_group(a, &resources)?;
                let signing = guard.admit_unsigned_signing(&t.lane, &saved, intent)?;
                intent.sign_reserved_original(store, &t.original.owner, &signing)
            })
            .unwrap();
    }
    intent.assert_existing_raw_valid_for_test();
}
fn select_s(t: &NextCreateFixture, signature: &mut PreparedV2TextRawSignature) {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let resources = signature.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_raw_signature_stage_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                signature,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    signature.run_stage(admitted.take().unwrap()).unwrap();
    let ready = signature.ready_selection().unwrap();
    let mut admitted = None;
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_raw_signature_commit_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                ready,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    signature.run_commit(admitted.take().unwrap()).unwrap();
}
fn select_b(t: &NextCreateFixture, accepted: &mut PreparedV2TextAcceptedCreate) {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let resources = accepted.resources().clone();
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    let mut admitted = None;
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_accepted_create_stage_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                accepted,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    accepted.run_stage(admitted.take().unwrap()).unwrap();
    let ready = accepted.ready_selection().unwrap();
    let mut admitted = None;
    let mut physical =
        Some(PreparedV2TextPhysicalLease::prepare(store, owner.source(), &resources).unwrap());
    store
        .with_v2_text_current(&owner, |a| {
            let mut guard = store.lock_v2_text_current_group(a, &resources)?;
            guard.admit_text_completed_accepted_create_commit_into(
                &t.lane,
                t.predecessor.as_ref().unwrap(),
                ready,
                &mut physical,
                &mut admitted,
            )
        })
        .unwrap();
    accepted.run_commit(admitted.take().unwrap()).unwrap();
}

fn second_b(
    t: &mut NextCreateFixture,
    mut query: Option<PreparedV2TextAcceptedQuery>,
    mut unsigned: Option<PreparedV2UnsignedOriginalCustody>,
    number: u64,
) -> Box<PreparedV2TextAcceptedCreate> {
    let store = t.original.layout.checked_store().unwrap();
    let owner = t.original.owner.borrow_current().unwrap();
    let mut intent = None;
    PreparedV2TextUnsignedIntent::take_into(
        store,
        owner.source(),
        &mut t.transaction,
        &mut unsigned,
        &mut query,
        &mut intent,
    )
    .unwrap();
    assert!(t.transaction.is_none() && unsigned.is_none() && query.is_none());
    let mut intent = intent.unwrap();
    intent
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 3),
        )
        .unwrap();
    drop(owner);
    select_i(t, &mut intent);
    sign_i(t, &mut intent);
    let owner = t.original.owner.borrow_current().unwrap();
    let mut intent = Some(intent);
    let mut signature = None;
    PreparedV2TextRawSignature::take_into(store, owner.source(), &mut intent, &mut signature)
        .unwrap();
    let mut signature = signature.unwrap();
    signature
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 4),
        )
        .unwrap();
    drop(owner);
    select_s(t, &mut signature);
    let candidate_counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let observation = signature.observe_candidate_work_for_test(candidate_counts.clone(), false);
    for _ in 0..3 {
        signature
            .materialize_candidate_after_saved_s(store, &t.original.owner)
            .unwrap();
    }
    drop(observation);
    assert_eq!(candidate_counts[0].load(Ordering::SeqCst), 1);
    assert_eq!(candidate_counts[1].load(Ordering::SeqCst), 1);
    let owner = t.original.owner.borrow_current().unwrap();
    let mut input = Some(signature);
    let mut accepted = None;
    PreparedV2TextAcceptedCreate::take_completed_into(
        store,
        owner.source(),
        &mut input,
        &mut accepted,
    )
    .unwrap();
    assert!(input.is_none());
    let mut accepted = accepted.unwrap();
    accepted
        .prepare_candidates(
            store,
            owner.source(),
            &mut ChaCha20Rng::seed_from_u64(number + 5),
        )
        .unwrap();
    drop(owner);
    select_b(t, &mut accepted);
    accepted
}

#[test]
fn actual_three_family_second_create_reuses_same_verifier_and_one_new_sdk_fee_without_extra_key_or_core(
) {
    for family in 0..3 {
        let core = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
        let key = Arc::new(AtomicUsize::new(0));
        PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
            core.clone(),
            || {
                PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
                    key.clone(),
                    None,
                    || {
                        let number = 0x82d00 + family as u64;
                        let mut t = next_saved_a_fixture(family, number);
                        assert_eq!(core[0].load(Ordering::SeqCst), 1);
                        assert_eq!(core[1].load(Ordering::SeqCst), 1);
                        assert_eq!(key.load(Ordering::SeqCst), 1);
                        let original = t.resident.as_ref().unwrap().observe_original();
                        let genesis = {
                            let owner = t.original.owner.borrow_current().unwrap();
                            let header = owner.source().data().header;
                            assert_eq!(header.spec().sequence, 1);
                            *header.id().as_bytes()
                        };
                        let first_header = t
                            .resident
                            .as_ref()
                            .unwrap()
                            .resident_header_state_for_test();
                        assert_eq!(first_header.2, 2);
                        assert_ne!(first_header.0, genesis);
                        let query = accepted_miss(&t);
                        let store = t.original.layout.checked_store().unwrap();
                        let before = store
                            .archive_bootstrap_policy
                            .as_ref()
                            .unwrap()
                            .pending
                            .pool
                            .charged_for_test();
                        let sdk = Arc::new(AtomicBool::new(false));
                        let mut next = None;
                        PreparedV2NextUnsignedAppend::with_sdk_fee_notice_for_test(
                            sdk.clone(),
                            || prepare_unsigned(&mut t, &query, &mut next),
                        )
                        .unwrap();
                        assert!(t.resident.is_none());
                        assert!(!t.model_dropped.load(Ordering::SeqCst));
                        let digest = next
                            .as_ref()
                            .unwrap()
                            .complete_digest_for_test(t.transaction.as_ref().unwrap())
                            .unwrap();
                        assert_eq!(
                            t.original
                                .layout
                                .checked_store()
                                .unwrap()
                                .archive_bootstrap_policy
                                .as_ref()
                                .unwrap()
                                .pending
                                .pool
                                .charged_for_test(),
                            before + catcoms_sync::owner_v2::APPEND_DATA_BUFFER_BYTES
                        );
                        for _ in 0..3 {
                            prepare_unsigned(&mut t, &query, &mut next).unwrap();
                            assert_eq!(
                                next.as_ref()
                                    .unwrap()
                                    .complete_digest_for_test(t.transaction.as_ref().unwrap())
                                    .unwrap(),
                                digest
                            );
                        }
                        let mut unsigned = None;
                        PreparedV2NextUnsignedAppend::take_complete_into(
                            t.transaction.as_ref().unwrap(),
                            &mut next,
                            &mut unsigned,
                        )
                        .unwrap();
                        assert!(next.is_none());
                        assert_eq!(
                            unsigned
                                .as_ref()
                                .unwrap()
                                .next_model_state_for_test(&original)
                                .unwrap(),
                            (first_header.0, 2)
                        );
                        // Repeating a completed DATA handoff cannot take another owner.
                        PreparedV2NextUnsignedAppend::take_complete_into(
                            t.transaction.as_ref().unwrap(),
                            &mut next,
                            &mut unsigned,
                        )
                        .unwrap();
                        let mut b = second_b(&mut t, Some(query), unsigned, number);
                        let saved = b.saved_accepted_create().unwrap().clone();
                        let resources = saved.resources().clone();
                        let store = t.original.layout.checked_store().unwrap();
                        let owner = t.original.owner.borrow_current().unwrap();
                        for _ in 0..3 {
                            store
                                .with_v2_text_current(&owner, |a| {
                                    let mut guard =
                                        store.lock_v2_text_current_group(a, &resources)?;
                                    let advance =
                                        guard.admit_accepted_advance(&t.lane, &saved, &b)?;
                                    b.advance_accepted_under_guard(&advance)
                                })
                                .unwrap();
                        }
                        let header = b.accepted_header_state_for_test().unwrap();
                        assert_eq!(
                            header.2,
                            Some(first_header.0),
                            "actual retired parent is the first advanced model"
                        );
                        assert_eq!(
                            b.next_model_state_for_test(&original).unwrap(),
                            (header.0, 3)
                        );
                        assert_ne!(header.0, first_header.0);
                        b.drain_accepted_retired_math_if_present().unwrap(); // OUTSIDE every writer
                        for _ in 0..3 {
                            store
                                .with_v2_text_current(&owner, |a| {
                                    let mut guard =
                                        store.lock_v2_text_current_group(a, &resources)?;
                                    let final_guard =
                                        guard.admit_accepted_final(&t.lane, &saved, &b)?;
                                    b.confirm_accepted_final_under_guard(&final_guard)
                                })
                                .unwrap();
                        }
                        drop(owner);
                        drop(resources);
                        drop(saved);
                        let selected = selected_capture(&t.original);
                        let public = selected.selected_data().unwrap().archive_data().spec_data();
                        assert_eq!(public.header.spec().sequence, 3);
                        assert_eq!(*public.header.spec().parent.as_bytes(), first_header.0);
                        // These are authenticated structural token DATA, not fabricated
                        // SavedA/canonical-hit authority or permission to create again.
                        let old = completed_query(&t.original, &selected, [31; 32]);
                        let new = completed_query(&t.original, &selected, NEXT_TOKEN);
                        assert_eq!(
                            old.accepted_header_data_for_test()
                                .unwrap()
                                .map(|h| *h.as_bytes()),
                            Some(first_header.0)
                        );
                        assert_eq!(
                            new.accepted_header_data_for_test()
                                .unwrap()
                                .map(|h| *h.as_bytes()),
                            Some(header.0)
                        );
                        assert_eq!(core[0].load(Ordering::SeqCst), 2);
                        assert_eq!(core[1].load(Ordering::SeqCst), 2);
                        assert_eq!(key.load(Ordering::SeqCst), 2);
                        assert!(!sdk.load(Ordering::SeqCst));
                        assert!(!t.model_dropped.load(Ordering::SeqCst));
                        drop(old);
                        drop(new);
                        drop(selected);
                        drop(b);
                        assert!(sdk.load(Ordering::SeqCst));
                        assert!(t.model_dropped.load(Ordering::SeqCst));
                        assert_eq!(
                            text_query_control_io_for_test(
                                t.original.layout.checked_store().unwrap()
                            ),
                            0
                        );
                    },
                )
            },
        );
    }
}

#[test]
fn actual_next_unsigned_refuses_foreign_resident_and_damaged_saved_a_before_taking_the_owner() {
    let mut a = next_saved_a_fixture(0, 0x82d10);
    let mut b = next_saved_a_fixture(1, 0x82d11);
    let query = accepted_miss(&a);
    let own = a.resident.as_ref().unwrap().observe_original();
    let other = b.resident.as_ref().unwrap().observe_original();
    let mut next = None;
    std::mem::swap(&mut a.resident, &mut b.resident);
    assert!(prepare_unsigned(&mut a, &query, &mut next).is_err());
    assert!(next.is_none() && a.resident.is_some() && b.resident.is_some());
    a.resident
        .as_ref()
        .unwrap()
        .observe_original()
        .require_same_original(&other)
        .unwrap();
    b.resident
        .as_ref()
        .unwrap()
        .observe_original()
        .require_same_original(&own)
        .unwrap();
    std::mem::swap(&mut a.resident, &mut b.resident);
    let target = a
        .original
        .owner
        .borrow_current()
        .unwrap()
        .source()
        .source()
        .capture
        .target
        .to_path_buf();
    let path = target.join("head.bin");
    let saved = std::fs::read(&path).unwrap();
    let mut damaged = saved.clone();
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    std::fs::write(&path, &damaged).unwrap();
    let before = a
        .original
        .layout
        .checked_store()
        .unwrap()
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .charged_for_test();
    assert!(prepare_unsigned(&mut a, &query, &mut next).is_err());
    assert_eq!(
        a.original
            .layout
            .checked_store()
            .unwrap()
            .archive_bootstrap_policy
            .as_ref()
            .unwrap()
            .pending
            .pool
            .charged_for_test(),
        before
    );
    assert!(next.is_none() && a.resident.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), damaged);
    std::fs::write(&path, &saved).unwrap();
    prepare_unsigned(&mut a, &query, &mut next).unwrap();
    let digest = next
        .as_ref()
        .unwrap()
        .complete_digest_for_test(a.transaction.as_ref().unwrap())
        .unwrap();
    let owner = a.original.owner.borrow_current().unwrap();
    owner.warm_scope().unwrap().revocation().revoke();
    drop(owner);
    assert!(prepare_unsigned(&mut a, &query, &mut next).is_err());
    assert!(a.resident.is_none() && next.is_some());
    assert_eq!(
        next.as_ref()
            .unwrap()
            .complete_digest_for_test(a.transaction.as_ref().unwrap())
            .unwrap(),
        digest
    );
    // Historical completed DATA can move for finite cleanup after Stop. It does
    // not renew I/current/sign authority or fabricate another successful Save.
    let mut custody = None;
    PreparedV2NextUnsignedAppend::take_complete_into(
        a.transaction.as_ref().unwrap(),
        &mut next,
        &mut custody,
    )
    .unwrap();
    assert!(next.is_none() && custody.is_some());
    assert_eq!(
        custody
            .as_ref()
            .unwrap()
            .next_model_state_for_test(&own)
            .unwrap()
            .1,
        2
    );
    assert!(!a.model_dropped.load(Ordering::SeqCst));
    drop(custody);
    assert!(a.model_dropped.load(Ordering::SeqCst));
    assert!(!b.model_dropped.load(Ordering::SeqCst));
}

#[test]
fn actual_domain_stop_after_next_owner_cache_preserves_same_model_and_new_sdk_payment_without_sdk_work(
) {
    let core = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let key = Arc::new(AtomicUsize::new(0));
    PreparedV2UnsignedOriginalCustody::with_accepted_advance_observation_for_test(
        core.clone(),
        || {
            PreparedV2UnsignedOriginalCustody::with_sign_dispatch_observation_for_test(
                key.clone(),
                None,
                || {
                    let mut t = next_saved_a_fixture(0, 0x82d20);
                    let original = t.resident.as_ref().unwrap().observe_original();
                    let header = t
                        .resident
                        .as_ref()
                        .unwrap()
                        .resident_header_state_for_test();
                    let query = accepted_miss(&t);
                    let stop = t
                        .resident
                        .as_ref()
                        .unwrap()
                        .observe_next_owner_cache_domain_stop_for_test();
                    let sdk = Arc::new(AtomicBool::new(false));
                    let mut next = None;
                    assert!(PreparedV2NextUnsignedAppend::with_sdk_fee_notice_for_test(
                        sdk.clone(),
                        || prepare_unsigned(&mut t, &query, &mut next)
                    )
                    .is_err());
                    drop(stop);
                    assert!(t.resident.is_none() && next.is_some());
                    assert_eq!(
                        next.as_ref()
                            .unwrap()
                            .cached_owner_state_for_test(&original)
                            .unwrap(),
                        (header.0, 2, false, false)
                    );
                    assert!(!sdk.load(Ordering::SeqCst));
                    assert!(!t.model_dropped.load(Ordering::SeqCst));
                    let store = t.original.layout.checked_store().unwrap();
                    let owner = t.original.owner.borrow_current().unwrap();
                    store.with_v2_text_current(&owner, |_| Ok(())).unwrap(); // real native Current stays live
                    drop(owner);
                    for _ in 0..3 {
                        assert!(prepare_unsigned(&mut t, &query, &mut next).is_err());
                    }
                    let mut custody = None;
                    assert!(PreparedV2NextUnsignedAppend::take_complete_into(
                        t.transaction.as_ref().unwrap(),
                        &mut next,
                        &mut custody
                    )
                    .is_err());
                    assert!(next.is_some() && custody.is_none());
                    assert_eq!(
                        next.as_ref()
                            .unwrap()
                            .cached_owner_state_for_test(&original)
                            .unwrap(),
                        (header.0, 2, false, false)
                    );
                    assert_eq!(core[0].load(Ordering::SeqCst), 1);
                    assert_eq!(core[1].load(Ordering::SeqCst), 1);
                    assert_eq!(key.load(Ordering::SeqCst), 1);
                    drop(next); // finite negative destruction outside every writer
                    assert!(sdk.load(Ordering::SeqCst));
                    assert!(t.model_dropped.load(Ordering::SeqCst));
                    assert_eq!(
                        text_query_control_io_for_test(t.original.layout.checked_store().unwrap()),
                        0
                    );
                },
            )
        },
    );
}

mod semantic;
