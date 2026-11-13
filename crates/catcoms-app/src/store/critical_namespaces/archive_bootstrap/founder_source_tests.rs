//! Real original debit/SDK/FI -> actual staged-source producers. All authored UNRUN while
//! compilation is paused. No decoder or fake Saved flag constructs a source owner here.
use super::super::tests::{created, fixture, NeverRng};
use super::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use catcoms_sync::owner_v2::open_bootstrap_source_data;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
type Owner = ArchiveFounderSourcePreparation<MemNetwork, ChaCha20Rng>;

fn source(store: &ServerStore, rng: &mut ChaCha20Rng, invocation: u8) -> Owner {
    let creator = created(store, rng, invocation);
    let mut planning = creator.into_ram_plan(store).unwrap();
    planning.save_founding_intent(store, rng).unwrap();
    planning.into_source_preparation(store).unwrap()
}
fn output(
    owner: &Owner,
    store: &ServerStore,
) -> (BootstrapSourceContextData, BootstrapSourceId, Vec<u8>) {
    let SourcePhase::Bootstrap(source) = &owner.phase else {
        panic!("actual cached Bootstrap expected")
    };
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let node = &owner.original.creator.server.as_ref().unwrap().sync;
    let frame = source.borrow_prepared_for_handoff(node, &key).unwrap();
    (
        frame.context_data().clone(),
        frame.id(),
        frame.as_bytes().to_vec(),
    )
}

#[test]
fn only_actual_saved_intent_consumes_into_source_and_caches_real_s0_genesis_exactly() {
    let (path, store, mut rng) = fixture(0xfd01);
    let creator = created(&store, &mut rng, 41);
    let planning = creator.into_ram_plan(&store).unwrap();
    let (mut planning, _) = planning
        .into_source_preparation(&store)
        .unwrap_err()
        .into_parts();
    assert!(planning
        .creator
        .server
        .as_ref()
        .unwrap()
        .sync
        .chat_scope_label()
        .is_none());
    planning.save_founding_intent(&store, &mut rng).unwrap();
    let original = planning.plan.as_ref().unwrap().data().unwrap().clone();
    let rows = fs::read(path.path().join("chat-admissions.bin")).unwrap();
    let pins = fs::read(path.path().join("chat-owner-pins.bin")).unwrap();
    let mut owner = planning.into_source_preparation(&store).unwrap();
    assert!(matches!(owner.phase, SourcePhase::PendingPrivateStage));
    assert_eq!(
        owner.prepare_private_source(&store, &mut rng).unwrap(),
        ArchiveFounderSourceProgress::BootstrapSourcePreparedUnexposed
    );
    let (context, id, frame) = output(&owner, &store);
    let context_view = context.spec_data();
    assert_eq!(context_view.group, original.group);
    assert_eq!(context_view.signer, original.signer);
    assert_eq!(context_view.fresh_attempt, original.fresh_attempt);
    assert_eq!(context_view.scope_nonce, original.nonce);
    assert_eq!(context_view.planned_revision, 1);
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let opened = open_bootstrap_source_data(&key, &context, id, &frame).unwrap();
    assert_eq!(opened.view_data().context, &context);
    let node = &owner.original.creator.server.as_ref().unwrap().sync;
    assert!(!node.chat_scope_label().unwrap().initializing());
    assert_eq!(
        node.chat_scope_revision(),
        2,
        "durable planned revision remains1"
    );
    owner.prepare_private_source(&store, &mut NeverRng).unwrap();
    assert_eq!(output(&owner, &store), (context, id, frame));
    assert_eq!(
        fs::read(path.path().join("chat-admissions.bin")).unwrap(),
        rows
    );
    assert_eq!(
        fs::read(path.path().join("chat-owner-pins.bin")).unwrap(),
        pins
    );
}

#[test]
fn pending_private_initializing_stage_retries_same_nonce_and_foreign_stage_stays_pending() {
    for foreign in [false, true] {
        let (_path, store, mut rng) = fixture(0xfd10 + foreign as u64);
        let mut owner = source(&store, &mut rng, 42);
        let facts = owner
            .original
            .plan
            .as_ref()
            .unwrap()
            .data()
            .unwrap()
            .clone();
        let nonce = if foreign { [99; 32] } else { facts.nonce };
        owner
            .original
            .creator
            .server
            .as_mut()
            .unwrap()
            .sync
            .reserve_managed_chat_scope(facts.scope_domain, nonce)
            .unwrap();
        assert!(matches!(owner.phase, SourcePhase::PendingPrivateStage));
        if foreign {
            assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
            assert!(matches!(owner.phase, SourcePhase::PendingPrivateStage));
            assert_eq!(
                owner
                    .original
                    .creator
                    .server
                    .as_ref()
                    .unwrap()
                    .sync
                    .chat_scope_label()
                    .unwrap()
                    .nonce(),
                nonce
            );
        } else {
            owner.prepare_private_source(&store, &mut rng).unwrap();
            assert_eq!(
                output(&owner, &store).0.spec_data().scope_nonce,
                facts.nonce
            );
        }
    }
}

#[test]
fn retained_bootstrap_freeze_refusal_keeps_raw_custody_without_reentering_a_retired_source() {
    let (_path, store, mut rng) = fixture(0xfd20);
    let mut owner = source(&store, &mut rng, 43);
    FOUNDERS_PREPARED.with(|count| count.set(0));
    let revoker = owner.original.creator.original.revoker();
    owner.before_freeze = Some(Box::new(move || revoker.revoke()));
    assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
    // Intentional contract change: consuming freeze loses only temporary DATA. Keep the
    // original signature cache, while the retired source STILL refuses all fresh work.
    assert!(matches!(owner.phase, SourcePhase::Signing { .. }));
    assert_eq!(FOUNDERS_PREPARED.with(|count| count.get()), 1);
    assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
    assert_eq!(FOUNDERS_PREPARED.with(|count| count.get()), 1);
}

#[test]
fn retained_bootstrap_freeze_unwind_keeps_the_original_signer_and_can_finish() {
    let (_path, store, mut rng) = fixture(0xfd21);
    let mut owner = source(&store, &mut rng, 46);
    let original_group = owner.original.creator.group_data().unwrap().to_vec();
    owner.before_freeze = Some(Box::new(|| panic!("injected before consuming freeze")));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = owner.prepare_private_source(&store, &mut NeverRng);
    }))
    .is_err());
    assert!(matches!(owner.phase, SourcePhase::Signing { .. }));
    assert_eq!(owner.original.creator.group_data().unwrap(), original_group);
    owner.prepare_private_source(&store, &mut rng).unwrap();
    let (context, id, bytes) = output(&owner, &store);
    owner.prepare_private_source(&store, &mut NeverRng).unwrap();
    let (again_context, again_id, again) = output(&owner, &store);
    assert_eq!(again_context, context);
    assert_eq!(again_id, id);
    assert_eq!(again, bytes);
}

#[test]
fn retained_bootstrap_cache_payment_refusal_can_finish_on_the_same_staged_owner() {
    let (_path, store, mut rng) = fixture(0xfd22);
    let mut owner = source(&store, &mut rng, 47);
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let free = pool.capacity() - pool.charged_for_test();
    let held = pool
        .try_reserve(free - (RETAINED_FOUNDER_SIGNING_INVOICE - 1))
        .unwrap();
    FOUNDERS_PREPARED.with(|count| count.set(0));
    assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
    assert!(matches!(owner.phase, SourcePhase::PendingPrivateStage));
    assert_eq!(FOUNDERS_PREPARED.with(|count| count.get()), 0);
    // Scope staging preceded the cache reservation. Its exact original reservation is
    // retained, so pressure cannot strand that same owner or force another SDK admission.
    // Staging and attaching the genuine reservation are TWO existing transitions.
    // Retrying the cache payment must not repeat either transition.
    let revision = owner
        .original
        .creator
        .server
        .as_ref()
        .unwrap()
        .sync
        .chat_scope_revision();
    assert_eq!(revision, 2);
    let group = owner.original.creator.group_data().unwrap().to_vec();
    drop(held);
    owner.prepare_private_source(&store, &mut rng).unwrap();
    assert_eq!(FOUNDERS_PREPARED.with(|count| count.get()), 1);
    assert_eq!(owner.original.creator.group_data().unwrap(), group);
    assert_eq!(
        owner
            .original
            .creator
            .server
            .as_ref()
            .unwrap()
            .sync
            .chat_scope_revision(),
        revision
    );
    assert!(matches!(owner.phase, SourcePhase::Bootstrap(_)));
}

#[test]
fn separate_phase_fee_refuses_before_mutation_and_same_planning_can_resume() {
    let (_path, store, mut rng) = fixture(0xfd30);
    let creator = created(&store, &mut rng, 44);
    let mut planning = creator.into_ram_plan(&store).unwrap();
    planning.save_founding_intent(&store, &mut rng).unwrap();
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let original = pool.charged_for_test();
    let held = pool.try_reserve(1024 * 1024 * 1024 - original).unwrap();
    let (planning, _) = planning
        .into_source_preparation(&store)
        .unwrap_err()
        .into_parts();
    assert!(planning
        .creator
        .server
        .as_ref()
        .unwrap()
        .sync
        .chat_scope_label()
        .is_none());
    drop(held);
    let mut owner = planning.into_source_preparation(&store).unwrap();
    assert_eq!(pool.charged_for_test(), original + PHASE_METADATA_FEE);
    owner.prepare_private_source(&store, &mut rng).unwrap();
}

#[test]
fn current_metadata_mismatch_and_equal_restored_core_never_replace_original_source() {
    let (_path, store, mut rng) = fixture(0xfd40);
    let mut owner = source(&store, &mut rng, 45);
    owner.prepare_private_source(&store, &mut rng).unwrap();
    let (context, id, frame) = output(&owner, &store);
    owner
        .original
        .creator
        .server
        .as_mut()
        .unwrap()
        .sync
        .set_rendezvous_nodes(vec![("changed".into(), vec![1])]);
    assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
    owner
        .original
        .creator
        .server
        .as_mut()
        .unwrap()
        .sync
        .set_rendezvous_nodes(Vec::new());
    owner.prepare_private_source(&store, &mut NeverRng).unwrap();
    assert_eq!(output(&owner, &store), (context.clone(), id, frame.clone()));
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    let opened = open_bootstrap_source_data(&key, &context, id, &frame).unwrap();
    let restored = catcoms_sync::ChannelSync::restore(
        opened.view_data().snapshot,
        Hub::new().join(PeerId::from_u64(991)),
        ChaCha20Rng::seed_from_u64(991),
        Box::new(ManualClock::new(10)),
    )
    .unwrap();
    owner.original.creator.server.as_mut().unwrap().sync = restored;
    assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
}

#[test]
fn actual_transfer_once_keeps_original_fee_output_and_requires_exact_transferred_phase() {
    let (_path, store, mut rng) = fixture(0xfd50);
    let mut owner = source(&store, &mut rng, 46);
    owner.prepare_private_source(&store, &mut rng).unwrap();
    let (context, id, frame) = output(&owner, &store);
    let key = Zeroizing::new(store.keys.db_key().unwrap());
    {
        let SourcePhase::Bootstrap(bootstrap) = &owner.phase else {
            panic!()
        };
        let actual = ActualFounderSource {
            creator: &owner.original.creator,
            phase: ActualSourcePhase::Bootstrap {
                source: bootstrap,
                key: &key,
            },
        };
        let plan = owner.original.plan.as_ref().unwrap();
        store
            .with_original_archive_founder(&actual, plan, |admission| {
                let guard = store.lock_archive_vacant_owner_namespace(admission)?;
                assert!(guard
                    .validate_transferred_source(&owner.original.creator.original, &context, id)
                    .is_err());
                Ok(())
            })
            .unwrap();
    }
    owner.detach_for_root(&store, &mut NeverRng).unwrap();
    assert!(owner.source_fee.is_some());
    assert_eq!(owner.detached.as_ref().unwrap().as_bytes(), frame);
    owner.detach_for_root(&store, &mut NeverRng).unwrap();
    let SourcePhase::Transferred {
        source,
        context: original,
        id: original_id,
    } = &owner.phase
    else {
        panic!()
    };
    assert_eq!(original.as_ref(), &context);
    assert_eq!(*original_id, id);
    let actual = ActualFounderSource {
        creator: &owner.original.creator,
        phase: ActualSourcePhase::Transferred {
            source,
            key: &key,
            context: original,
            id,
        },
    };
    store
        .with_original_archive_founder(
            &actual,
            owner.original.plan.as_ref().unwrap(),
            |admission| {
                let guard = store.lock_archive_vacant_owner_namespace(admission)?;
                guard.validate_transferred_source(
                    &owner.original.creator.original,
                    &context,
                    id,
                )?;
                assert!(guard.require_founding_phase().is_err());
                assert!(guard
                    .validate_transferred_source(
                        &owner.original.creator.original,
                        &context,
                        BootstrapSourceId::from_bytes([99; 32])
                    )
                    .is_err());
                let mut spec = context.spec_data();
                spec.scope_nonce = [99; 32];
                let wrong = BootstrapSourceContextData::from_data(spec).unwrap();
                assert!(guard
                    .validate_transferred_source(&owner.original.creator.original, &wrong, id)
                    .is_err());
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn actual_root_registration_holds_complete_cipher_and_retries_same_original_cell() {
    let (_path, store, mut rng) = fixture(0xfd60);
    let mut owner = source(&store, &mut rng, 47);
    owner.prepare_private_source(&store, &mut rng).unwrap();
    assert_eq!(
        owner
            .register_private_source(&store, &mut NeverRng)
            .unwrap(),
        ArchiveFounderSourceProgress::BootstrapCipherRetainedUnexposed
    );
    assert!(owner.registered.is_some());
    assert!(owner.detached.is_none());
    assert!(owner.source_fee.is_none());
    owner
        .register_private_source(&store, &mut NeverRng)
        .unwrap();
    // Lose ONLY the observed local handle: Root's complete inert cell and actual fee survive.
    owner.registered = None;
    owner
        .register_private_source(&store, &mut NeverRng)
        .unwrap();
    assert!(owner.registered.is_some());
    owner.close();
    assert!(owner
        .register_private_source(&store, &mut NeverRng)
        .is_err());
}

#[test]
fn last_full_source_comparison_cannot_outlive_original_root_or_creator_return_check() {
    for transferred in [false, true] {
        for cancel in [false, true] {
            let (path, store, mut rng) = fixture(0xfd70 + 2 * transferred as u64 + cancel as u64);
            let mut owner = source(&store, &mut rng, 48);
            owner.prepare_private_source(&store, &mut rng).unwrap();
            if transferred {
                owner.detach_for_root(&store, &mut NeverRng).unwrap();
            }
            let marker = path.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(ROOT_MARKER);
            let original = fs::read(&marker).unwrap();
            if cancel {
                let revoker = owner.original.creator.original.revoker();
                owner.before_source_return = Some(Box::new(move || revoker.revoke()));
            } else {
                let target = marker.clone();
                let mut changed = original.clone();
                changed[0] ^= 1;
                owner.before_source_return =
                    Some(Box::new(move || fs::write(target, changed).unwrap()));
            }
            assert!(owner.prepare_private_source(&store, &mut NeverRng).is_err());
            assert!(
                matches!(
                    owner.phase,
                    SourcePhase::Bootstrap(_) | SourcePhase::Transferred { .. }
                ),
                "exact source is retained after failed observation"
            );
            if !cancel {
                fs::write(&marker, original).unwrap();
                owner.prepare_private_source(&store, &mut NeverRng).unwrap();
            }
        }
    }
}

#[test]
fn final_root_registration_races_leave_complete_original_output_and_fee_untouched() {
    for cancel in [false, true] {
        let (path, store, mut rng) = fixture(0xfd80 + cancel as u64);
        let mut owner = source(&store, &mut rng, 49);
        owner.prepare_private_source(&store, &mut rng).unwrap();
        let (_, _, frame) = output(&owner, &store);
        let marker = path.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(ROOT_MARKER);
        let original = fs::read(&marker).unwrap();
        if cancel {
            let revoker = owner.original.creator.original.revoker();
            *store.founder_before_source_admission.lock().unwrap() =
                Some(Box::new(move || revoker.revoke()));
        } else {
            let target = marker.clone();
            let mut changed = original.clone();
            changed[0] ^= 1;
            *store.founder_before_source_admission.lock().unwrap() =
                Some(Box::new(move || fs::write(target, changed).unwrap()));
        }
        assert!(owner
            .register_private_source(&store, &mut NeverRng)
            .is_err());
        assert_eq!(owner.detached.as_ref().unwrap().as_bytes(), frame);
        assert!(owner.source_fee.is_some());
        assert!(owner.registered.is_none());
        if !cancel {
            fs::write(marker, original).unwrap();
            owner
                .register_private_source(&store, &mut NeverRng)
                .unwrap();
            assert!(owner.detached.is_none());
            assert!(owner.source_fee.is_none());
        }
    }
}

#[test]
fn final_root_revalidation_and_lost_handle_lookup_repeat_same_negative_after_comparers() {
    let (path, store, mut rng) = fixture(0xfd90);
    let mut owner = source(&store, &mut rng, 50);
    owner.register_private_source(&store, &mut rng).unwrap();
    let marker = path.path().join(ARCHIVE_BOOTSTRAP_ROOT).join(ROOT_MARKER);
    let original = fs::read(&marker).unwrap();
    let target = marker.clone();
    let mut changed = original.clone();
    changed[0] ^= 1;
    *store.founder_before_source_admission.lock().unwrap() =
        Some(Box::new(move || fs::write(target, changed).unwrap()));
    assert!(owner
        .register_private_source(&store, &mut NeverRng)
        .is_err());
    assert!(owner.registered.is_some());
    assert!(owner.detached.is_none());
    assert!(owner.source_fee.is_none());
    fs::write(marker, original).unwrap();
    owner
        .register_private_source(&store, &mut NeverRng)
        .unwrap();
    owner.registered = None;
    let revoker = owner.original.creator.original.revoker();
    *store.founder_before_source_admission.lock().unwrap() =
        Some(Box::new(move || revoker.revoke()));
    assert!(owner
        .register_private_source(&store, &mut NeverRng)
        .is_err());
    assert!(owner.registered.is_none());
    assert!(owner.detached.is_none());
    assert!(owner.source_fee.is_none());
}

#[test]
fn owner_drop_cancels_original_but_store_keeps_complete_cipher_control_and_source_credit() {
    let (_path, store, mut rng) = fixture(0xfda0);
    let pool = store
        .archive_bootstrap_policy
        .as_ref()
        .unwrap()
        .pending
        .pool
        .clone();
    let baseline = pool.charged_for_test();
    let mut owner = source(&store, &mut rng, 51);
    owner.register_private_source(&store, &mut rng).unwrap();
    let observer = owner.original.creator.original.observer();
    let SourcePhase::Transferred { id, .. } = &owner.phase else {
        panic!("actual one-time transfer expected")
    };
    let expected_id = *id.as_bytes();
    let retained = store.retained_bootstrap_source_for_test([51; 32]).unwrap();
    assert!(retained.0 > 0);
    assert_eq!(retained.1, expected_id);
    assert!(!observer.is_cancelled());
    // DROP is intentional: closing while all buffers/fees stay in an owned value would not
    // prove that inert Store custody survives payload/provider destruction without its sender.
    drop(owner);
    assert!(observer.is_cancelled());
    assert_eq!(
        store.retained_bootstrap_source_for_test([51; 32]).unwrap(),
        retained,
        "actual complete cipher hash/length and original ID survive creator destruction"
    );
    assert_eq!(
        pool.charged_for_test(),
        baseline + codec::CONTROL_FEE + BOOTSTRAP_SOURCE_CUSTODY_FEE,
        "node and phase credits drain, but complete debt keeps source+same control credit"
    );
    drop(store);
    assert_eq!(
        pool.charged_for_test(),
        0,
        "the last actual Store/cipher/control owners refund after drain"
    );
}
