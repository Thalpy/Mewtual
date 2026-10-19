use super::*;

fn action_coordinator() -> Coordinator {
    Coordinator::with_runtime(Runtime::new(Arc::new(catcoms_rt::ManualClock::new(1000))))
}

/// Only the desktop lifecycle grant is a fixture here. Pair derivation still requires the real
/// signed announcement -> watch challenge -> join proof -> LocalJoin pair factories. This tests
/// the native engine registry, not actual MLS/native WebView/transport room acceptance.
fn proven_action(
    coordinator: &Coordinator,
    owner_id: &str,
    pool: Arc<Resources>,
) -> (
    catcoms_sync::voice_v2::presence::LocalJoin,
    Arc<CryptoAction>,
) {
    use catcoms_crypto::DeviceKeypair;
    use catcoms_sync::voice_v2::limits::OriginLimits;
    use catcoms_sync::voice_v2::packets::Authority;
    use catcoms_sync::voice_v2::presence::{LocalJoin, WatchAuthority, Watcher};
    use catcoms_sync::voice_v2::resources::ByteBudget;
    use catcoms_sync::voice_v2::{Context, JoinKey, WatchLease};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    let owner = coordinator.owner(owner_id).unwrap();
    let clock = coordinator.runtime.clock();
    let mut rng = ChaCha20Rng::from_seed([99; 32]);
    let signer = DeviceKeypair::from_seed(&[3; 32]);
    let remote_signer = DeviceKeypair::from_seed(&[4; 32]);
    let context = Context {
        scope: owner.binding.scope,
        local: signer.device_id(),
        now: clock.now_ms(),
        epoch: 1,
        indirect: true,
    };
    let remote_context = Context {
        local: remote_signer.device_id(),
        ..context
    };
    let members = |_| true;
    let authority = Authority {
        context: &context,
        lease: &owner.lease,
        member: &members,
    };
    let remote_lease = CallLease::default();
    let remote_authority = Authority {
        context: &remote_context,
        lease: &remote_lease,
        member: &members,
    };
    let point: [u8; 65] = hex::decode("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5").unwrap().try_into().unwrap();
    let mut join = LocalJoin::new(
        &context,
        JoinKey::new(owner.join, point).unwrap(),
        owner.lease.clone(),
        pool.clone(),
    )
    .unwrap();
    let mut remote = LocalJoin::new(
        &remote_context,
        JoinKey::new([7; 32], point).unwrap(),
        remote_lease.clone(),
        pool.clone(),
    )
    .unwrap();
    join.announce(&signer, &authority, clock.as_ref(), &mut rng)
        .unwrap();
    let announcement = remote
        .announce(&remote_signer, &remote_authority, clock.as_ref(), &mut rng)
        .unwrap();
    let watch = WatchLease::default();
    let wa = WatchAuthority {
        context: &context,
        lease: &watch,
        member: &members,
    };
    let mut watcher = Watcher::new(&context, watch.clone(), pool.clone(), &mut rng).unwrap();
    let mut limits = OriginLimits::new(context.scope.group, pool);
    let challenge = watcher
        .challenge(
            &announcement,
            &signer,
            &wa,
            &mut limits,
            clock.as_ref(),
            &mut rng,
        )
        .unwrap();
    let proof = remote
        .answer(
            &challenge,
            &remote_signer,
            &remote_authority,
            clock.as_ref(),
            &mut rng,
        )
        .unwrap();
    let fresh = watcher.prove(&proof, &wa, clock.as_ref()).unwrap();
    let setup = join
        .begin_pair(&fresh, &authority, &mut limits, clock.as_ref())
        .unwrap();
    let action = join
        .derivation_action(setup.handshake, &authority, clock, ByteBudget::Ordinary)
        .unwrap();
    (join, action)
}

/// Commit the genuine native derivation action without a JSON result cache or a browser key.
/// The returned metadata is used only to exercise native history ownership below.
fn derivation_receipt(
    coordinator: &Coordinator,
    owner_id: &str,
    join: &mut catcoms_sync::voice_v2::presence::LocalJoin,
    action: &CryptoAction,
) -> CryptoResultReceipt {
    use catcoms_crypto::DeviceKeypair;
    use catcoms_sync::voice_v2::packets::Authority;
    use catcoms_sync::voice_v2::Context;
    let owner = coordinator.owner(owner_id).unwrap();
    let clock = coordinator.runtime.clock();
    let context = Context {
        scope: owner.binding.scope,
        local: DeviceKeypair::from_seed(&[3; 32]).device_id(),
        now: clock.now_ms(),
        epoch: 1,
        indirect: true,
    };
    let members = |_| true;
    join.commit_derivation_result(
        action,
        action.output_tag(&[]).unwrap(),
        &Authority {
            context: &context,
            lease: &owner.lease,
            member: &members,
        },
        clock.as_ref(),
    )
    .unwrap()
}

#[test]
fn registered_terminal_history_survives_end_settlement_and_plaintext_loss_until_exact_ack() {
    use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let owner = call(&mut c, &controller_id, 1, 1);
    let (mut join, action) = proven_action(&c, &owner, c.resources.clone());
    let identity = c.register_action(&owner, action.clone()).unwrap();
    let pending = c.action_status(&identity).unwrap();
    assert!(!pending.settled && pending.receipt.is_none());
    c.admit_action(&identity, action.engine_bytes()).unwrap();
    let receipt = derivation_receipt(&c, &owner, &mut join, &action);
    assert_eq!(receipt.outcome, CryptoOutcome::Derived);
    assert!(!receipt.retired);
    drop(join);
    c.end(&owner);
    assert!(c.settle_action(&identity).unwrap());
    let held = c.resources.counts();
    assert_eq!(
        c.actions.len(),
        1,
        "End and engine settlement cannot acknowledge a lost reply"
    );
    let status = c.action_status(&identity).unwrap();
    assert!(status.settled);
    assert!(status.receipt.as_ref().unwrap().retired);
    assert!(receipt.same_transition(status.receipt.as_ref().unwrap()));
    assert_eq!(
        c.resources.counts(),
        held,
        "metadata status performs no new work"
    );
    assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
    assert!(c.acknowledge_action(&identity, &receipt).unwrap());
    assert!(c.actions.is_empty());
    assert!(
        !c.acknowledge_action(&identity, &receipt).unwrap(),
        "lost ACK reply is idempotent"
    );
    assert!(!c.settle_action(&identity).unwrap());
}

#[test]
fn registered_terminal_ack_latches_before_settlement_and_never_recreates_root_action() {
    use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let owner = call(&mut c, &controller_id, 1, 1);
    let (mut join, action) = proven_action(&c, &owner, c.resources.clone());
    let identity = c.register_action(&owner, action.clone()).unwrap();
    c.admit_action(&identity, action.engine_bytes()).unwrap();
    let receipt = derivation_receipt(&c, &owner, &mut join, &action);
    for changed in [
        CryptoResultReceipt {
            output_tag: [9; 32],
            ..receipt.clone()
        },
        CryptoResultReceipt {
            outcome: CryptoOutcome::Opened,
            ..receipt.clone()
        },
        CryptoResultReceipt {
            input_tag: [9; 32],
            ..receipt.clone()
        },
        CryptoResultReceipt {
            action: receipt.action + 1,
            ..receipt.clone()
        },
    ] {
        assert!(c.acknowledge_action(&identity, &changed).is_err());
    }
    assert!(c.acknowledge_action(&identity, &receipt).unwrap());
    assert!(c.acknowledge_action(&identity, &receipt).unwrap());
    assert_eq!(c.actions.len(), 1);
    assert_eq!(
        c.resources.counts().1,
        1,
        "ACK is not actual engine settlement"
    );
    assert!(c.settle_action(&identity).unwrap());
    assert!(c.actions.is_empty());
    assert!(
        c.register_action(&owner, action.clone()).is_err(),
        "Root's retained terminal Arc cannot recreate an acknowledged row"
    );
    assert!(action.admit_engine(action.engine_bytes()).is_err());
    assert!(!c.acknowledge_action(&identity, &receipt).unwrap());
}

#[test]
fn registered_terminal_capacity_is_shared_across_replacement_owners_without_ttl_eviction() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let mut retained = Vec::new();
    for n in 1..=REGISTERED_ACTIONS {
        let owner = call(&mut c, &controller_id, n as u8, n as u64);
        let (mut join, action) = proven_action(&c, &owner, c.resources.clone());
        let identity = c.register_action(&owner, action.clone()).unwrap();
        c.admit_action(&identity, action.engine_bytes()).unwrap();
        let receipt = derivation_receipt(&c, &owner, &mut join, &action);
        c.settle_action(&identity).unwrap();
        retained.push((identity, receipt));
    }
    assert_eq!(c.actions.len(), REGISTERED_ACTIONS);
    let next = call(&mut c, &controller_id, 63, 63);
    let (_join, action) = proven_action(&c, &next, c.resources.clone());
    assert!(c.register_action(&next, action.clone()).is_err());
    let (old, receipt) = &retained[0];
    assert!(c.action_status(old).unwrap().receipt.is_some());
    assert!(c.acknowledge_action(old, receipt).unwrap());
    assert_eq!(c.actions.len(), REGISTERED_ACTIONS - 1);
    assert!(c.register_action(&next, action).is_ok());
}

#[test]
fn registered_action_lost_admission_reuses_permit_and_unknown_settlement_creates_no_fence() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let (_join, action) = proven_action(&c, &owner, c.resources.clone());
    let unknown = ActionIdentity {
        owner: owner.clone(),
        action: action.serial(),
        version: 1,
        input_tag: action.input_tag(),
    };
    assert!(c.admit_action(&unknown, action.engine_bytes()).is_err());
    assert!(!c.settle_action(&unknown).unwrap());
    assert!(c.actions.is_empty());
    for invalid in [
        ActionIdentity {
            owner: "x".repeat(129),
            ..unknown.clone()
        },
        ActionIdentity {
            owner: String::new(),
            ..unknown.clone()
        },
        ActionIdentity {
            action: 0,
            ..unknown.clone()
        },
        ActionIdentity {
            version: 0,
            ..unknown.clone()
        },
    ] {
        assert!(c.settle_action(&invalid).is_err());
    }
    assert!(c.actions.is_empty());
    let identity = c.register_action(&owner, action.clone()).unwrap();
    assert!(c.register_action(&owner, action.clone()).unwrap() == identity);
    let before = c.resources.counts();
    let permit = c.admit_action(&identity, action.engine_bytes()).unwrap();
    let reader = action.input().unwrap();
    let catcoms_sync::voice_v2::crypto_actions::CryptoInput::Derive(setup) = reader.as_input()
    else {
        panic!("derive input")
    };
    // The native action contract includes256 bytes of fixed derive-input metadata.
    let payload_bytes = setup.transcript.len() + 256;
    assert_eq!(
        c.admit_action(&identity, action.engine_bytes()).unwrap(),
        permit
    );
    assert_eq!(c.resources.counts(), (before.0 + action.engine_bytes(), 1));
    for bad in [
        ActionIdentity {
            input_tag: [0; 32],
            ..identity.clone()
        },
        ActionIdentity {
            version: 2,
            ..identity.clone()
        },
        ActionIdentity {
            owner: "another-native-owner".into(),
            ..identity.clone()
        },
    ] {
        assert!(c.admit_action(&bad, action.engine_bytes()).is_err());
        if bad.owner == identity.owner {
            assert!(c.settle_action(&bad).is_err());
        }
    }
    assert_eq!(c.resources.counts().1, 1);
    assert!(c.settle_action(&identity).unwrap());
    assert!(c.settle_action(&identity).unwrap());
    assert_eq!(c.resources.counts(), before);
    assert!(action.input().is_err());
    drop(reader);
    assert_eq!(c.resources.counts(), (before.0 - payload_bytes, 0));
    assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
}

#[test]
fn registered_action_settlement_before_admission_is_permanent_for_delayed_requests() {
    let mut c = action_coordinator();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let (_join, action) = proven_action(&c, &owner, c.resources.clone());
    let identity = c.register_action(&owner, action.clone()).unwrap();
    let before = c.resources.counts();
    let reader = action.input().unwrap();
    let catcoms_sync::voice_v2::crypto_actions::CryptoInput::Derive(setup) = reader.as_input()
    else {
        panic!("derive input")
    };
    let payload_bytes = setup.transcript.len() + 256;
    assert!(c.settle_action(&identity).unwrap());
    assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
    assert!(c.register_action(&owner, action.clone()).unwrap() == identity);
    assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
    assert_eq!(c.resources.counts(), before);
    assert!(action.input().is_err());
    drop(reader);
    assert_eq!(c.resources.counts(), (before.0 - payload_bytes, 0));
}

#[test]
fn registered_action_old_owner_engine_survives_lock_replacement_and_root_drop() {
    let mut c = action_coordinator();
    let controller_id = controller(&mut c);
    let owner = call(&mut c, &controller_id, 1, 1);
    let (join, action) = proven_action(&c, &owner, c.resources.clone());
    let identity = c.register_action(&owner, action.clone()).unwrap();
    c.admit_action(&identity, action.engine_bytes()).unwrap();
    drop(join);
    c.revoke_all();
    assert_eq!(c.resources.counts().1, 1);
    assert_eq!(c.actions.len(), 1);
    assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
    let controller_id = controller(&mut c);
    let replacement = call(&mut c, &controller_id, 1, 2);
    assert!(c.register_action(&replacement, action.clone()).is_err());
    assert!(c.settle_action(&identity).unwrap());
    assert_eq!(c.resources.counts().1, 0);
    assert!(c.actions.is_empty());
    assert!(!c.settle_action(&identity).unwrap());
    assert!(c.register_action(&owner, action).is_err());
}

#[test]
fn registered_action_wrong_process_pool_refuses_and_unstarted_owner_retirement_closes_fence() {
    for foreign_pool in [false, true] {
        let mut c = action_coordinator();
        let controller = controller(&mut c);
        let owner = call(&mut c, &controller, 1, 1);
        let pool = if foreign_pool {
            Arc::new(Resources::default())
        } else {
            c.resources.clone()
        };
        let (_join, action) = proven_action(&c, &owner, pool);
        if foreign_pool {
            assert!(c.register_action(&owner, action).is_err());
            assert!(c.actions.is_empty());
            assert_eq!(c.resources.counts(), (0, 0));
        } else {
            let identity = c.register_action(&owner, action.clone()).unwrap();
            c.end(&owner);
            assert!(c.actions.is_empty());
            assert!(action.is_settled());
            assert!(c.admit_action(&identity, action.engine_bytes()).is_err());
            assert!(action.admit_engine(action.engine_bytes()).is_err());
        }
    }
}

fn binding(group: u8, instance: u64) -> Binding {
    Binding {
        generation: 1,
        server: 7,
        instance,
        group: vec![group],
        scope: Scope::for_group(&[group], Some([8; 32]), 12),
    }
}
fn controller(coordinator: &mut Coordinator) -> String {
    let id = coordinator
        .prepare(CandidateKind::Controller(1), 0)
        .unwrap();
    coordinator.activate(&id, [0; 32], 0, None).unwrap();
    id
}
fn call(coordinator: &mut Coordinator, controller: &str, group: u8, instance: u64) -> String {
    let id = coordinator
        .prepare(
            CandidateKind::Call {
                controller: controller.into(),
                binding: binding(group, instance),
            },
            0,
        )
        .unwrap();
    let grant = CallGrant::from_lease(CallLease::default(), binding(group, instance).scope);
    coordinator
        .activate(&id, [instance as u8; 32], 0, Some(grant))
        .unwrap();
    id
}

#[test]
fn stale_activation_and_end_cannot_retire_a_replacement_call() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let old_candidate = c
        .prepare(
            CandidateKind::Call {
                controller: controller.clone(),
                binding: binding(1, 1),
            },
            0,
        )
        .unwrap();
    let current = call(&mut c, &controller, 1, 2);
    let lease = c.owner(&current).unwrap().lease;
    assert!(c.activate(&old_candidate, [1; 32], 0, None).is_err());
    c.end(&old_candidate);
    assert!(lease.is_live());
    assert!(c.owner(&current).is_some());
    let replacement = call(&mut c, &controller, 1, 3);
    assert!(!lease.is_live());
    c.end(&current);
    assert!(c.owner(&replacement).is_some());
}

#[test]
fn ending_current_call_preserves_controller_and_explicit_watch_parent() {
    use catcoms_sync::voice_v2::WatchLease;
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let parent = c.controller_parent(&controller, 1).unwrap();
    let watch = WatchLease::default().with_parent(parent.clone()).unwrap();
    let owner = call(&mut c, &controller, 1, 1);
    let lease = c.owner(&owner).unwrap().lease;
    c.end(&owner);
    assert!(!lease.is_live());
    assert!(parent.is_live() && watch.is_live());
    assert!(c.controller_parent(&controller, 1).is_ok());
}

#[test]
fn native_collection_uses_injected_clock_while_locked_preserves_engine_uncertainty_and_stops() {
    use catcoms_app::DeviceId;
    use catcoms_rt::{Clock, ManualClock};
    use catcoms_sync::voice_v2::runtime::View;
    use libp2p::futures::FutureExt;
    let clock = Arc::new(ManualClock::new(1000));
    let runtime = Runtime::new(clock.clone());
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let device = DeviceId::from_bytes([1; 32]);
    let (mut actor, registration) = runtime.prepare_actor([1; 32], device, &ticket).unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: 1,
            members: [device].into_iter().collect(),
            policy: Some([8; 32]),
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let mut c = Coordinator::with_runtime(runtime.clone());
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    let charge = c.admit(&owner, Operation::Generate, 128).unwrap();
    let baseline = runtime.resources().counts();
    actor
        .with_limits(|limits| limits.requester(device, clock.monotonic_ms()))
        .unwrap();
    let (stop, cancelled) = tokio::sync::oneshot::channel();
    let mut collection = Box::pin(collect_until_stopped(
        Arc::downgrade(&runtime),
        runtime.clock(),
        cancelled,
    ));
    assert!(collection.as_mut().now_or_never().is_none());
    c.revoke_all();
    runtime.revoke_session();
    clock.advance_ms(60_000);
    assert!(collection.as_mut().now_or_never().is_none());
    assert_eq!(runtime.resources().counts(), baseline);
    assert_eq!(
        c.operations.len(),
        1,
        "lost WebView settlement has no TTL refund"
    );
    stop.send(()).unwrap();
    assert!(collection.as_mut().now_or_never().is_some());
    assert!(c.settle(&charge));
}

#[test]
fn controller_parent_closes_on_replace_end_lock_and_drop_without_refunding_engine_work() {
    use catcoms_sync::voice_v2::WatchLease;
    for transition in 0..4 {
        let mut c = Coordinator::default();
        let id = controller(&mut c);
        let parent = c.controller_parent(&id, 1).unwrap();
        let watch = WatchLease::default().with_parent(parent.clone()).unwrap();
        assert!(c.controller_parent(&id, 2).is_err());
        let owner = call(&mut c, &id, 1, 1);
        let charge = c.admit(&owner, Operation::Generate, 128).unwrap();
        assert!(
            parent.is_live(),
            "starting a call preserves explicit observation consent"
        );
        match transition {
            0 => {
                let next = controller(&mut c);
                c.end(&id);
                assert!(c.controller_parent(&next, 1).unwrap().is_live());
            }
            1 => c.end(&id),
            2 => c.revoke_all(),
            _ => {
                drop(c);
                assert!(!parent.is_live() && !watch.is_live());
                continue;
            }
        }
        assert!(
            !parent.is_live() && !watch.is_live(),
            "transition {transition}"
        );
        assert!(c.controller_parent(&id, 1).is_err());
        assert_eq!(
            c.operations.len(),
            1,
            "retiring consent never refunds actual unsettled crypto"
        );
        assert!(c.settle(&charge));
    }
}

#[test]
fn remount_lock_and_exact_actor_retirement_revoke_shared_leases_without_refunding_work() {
    let mut c = Coordinator::default();
    let first_controller = controller(&mut c);
    let first = call(&mut c, &first_controller, 1, 1);
    let owner = c.owner(&first).unwrap();
    let charge = c.admit(&first, Operation::Generate, 128).unwrap();
    c.retire_server(7, 9);
    assert!(owner.lease.is_live());
    let new_controller = controller(&mut c);
    assert!(!owner.lease.is_live());
    assert_eq!(c.operations.len(), 1);
    let second = call(&mut c, &new_controller, 1, 2);
    let lease = c.owner(&second).unwrap().lease;
    c.retire_server(7, 2);
    assert!(!lease.is_live());
    assert_eq!(c.operations.len(), 1);
    let third = call(&mut c, &new_controller, 1, 3);
    let lease = c.owner(&third).unwrap().lease;
    c.revoke_all();
    assert!(!lease.is_live());
    assert!(c.admit(&third, Operation::Generate, 1).is_err());
    assert!(c.settle(&charge));
    assert!(!c.settle(&charge));
}

#[test]
fn unsettled_work_remains_group_charged_across_new_controllers_and_calls() {
    let mut c = Coordinator::default();
    let mut controller_id = controller(&mut c);
    let owner = call(&mut c, &controller_id, 1, 1);
    let charges = (0..4)
        .map(|_| c.admit(&owner, Operation::Decrypt, 1024).unwrap())
        .collect::<Vec<_>>();
    c.revoke_all();
    controller_id = controller(&mut c);
    let replacement = call(&mut c, &controller_id, 1, 2);
    assert!(c.admit(&replacement, Operation::Generate, 128).is_err());
    assert!(c.settle(&charges[0]));
    let new = c.admit(&replacement, Operation::Generate, 128).unwrap();
    assert_ne!(new, charges[0]);
    assert!(!c.settle(&charges[0]));
    assert_eq!(c.operations.len(), 4);
}

#[test]
fn process_bound_survives_group_account_and_owner_changes() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    for group in [1, 2] {
        let owner = call(&mut c, &controller, group, group as u64);
        for _ in 0..4 {
            c.admit(&owner, Operation::Encrypt, 1024).unwrap();
        }
    }
    let owner = call(&mut c, &controller, 3, 3);
    assert!(c.admit(&owner, Operation::Generate, 128).is_err());
    let charge = c.operations.keys().next().unwrap().clone();
    assert!(c.settle(&charge));
    assert!(c.admit(&owner, Operation::Generate, 128).is_ok());
}

#[test]
fn byte_operation_and_identity_boundaries_refuse_without_charge() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let owner = call(&mut c, &controller, 1, 1);
    assert!(Operation::parse("arbitrary-sign").is_err());
    assert!(Operation::parse("derive").is_ok());
    assert!(c.admit("not-an-owner", Operation::Generate, 128).is_err());
    assert!(c.admit(&owner, Operation::Generate, 129).is_err());
    assert!(c
        .admit(&owner, Operation::Encrypt, 4096 + 32 * 1024 + 1)
        .is_err());
    assert!(c.operations.is_empty());
    c.serial = u64::MAX;
    assert!(c.admit(&owner, Operation::Generate, 1).is_err());
    assert!(c.operations.is_empty());
}

#[test]
fn candidates_are_bounded_expire_and_never_revoke_until_successful_cas() {
    let mut c = Coordinator::default();
    let controller_id = controller(&mut c);
    let owner = call(&mut c, &controller_id, 1, 1);
    let lease = c.owner(&owner).unwrap().lease;
    let candidates = (0..8)
        .map(|_| c.prepare(CandidateKind::Controller(1), 0).unwrap())
        .collect::<Vec<_>>();
    assert!(c.prepare(CandidateKind::Controller(1), 0).is_err());
    assert!(lease.is_live());
    assert!(c.candidate(&candidates[0], 30_000).is_none());
    assert!(c.activate(&candidates[0], [0; 32], 30_000, None).is_err());
    assert!(lease.is_live());
    assert!(c
        .prepare(
            CandidateKind::Call {
                controller: controller_id,
                binding: Binding {
                    generation: 2,
                    ..binding(1, 1)
                }
            },
            30_000
        )
        .is_err());
    let new_controller = c.prepare(CandidateKind::Controller(1), 30_000).unwrap();
    c.activate(&new_controller, [0; 32], 30_000, None).unwrap();
    assert!(!lease.is_live());
}

#[test]
fn lease_revocation_is_shared_and_irreversible() {
    let first = CallLease::default();
    let copy = first.clone();
    let other = CallLease::default();
    assert!(first.is_same(&copy));
    assert!(!first.is_same(&other));
    copy.revoke();
    assert!(!first.is_live());
    first.revoke();
    assert!(!copy.is_live());
    assert!(other.is_live());
}

#[test]
fn activation_requires_exact_live_actor_grant_and_failed_candidates_preserve_current_owner() {
    let mut c = Coordinator::default();
    let controller = controller(&mut c);
    let current = call(&mut c, &controller, 1, 1);
    let old = c.owner(&current).unwrap();
    for mode in 0..3 {
        let id = c
            .prepare(
                CandidateKind::Call {
                    controller: controller.clone(),
                    binding: binding(1, 2),
                },
                0,
            )
            .unwrap();
        let lease = CallLease::default();
        let grant = match mode {
            0 => None,
            1 => Some(CallGrant::from_lease(lease.clone(), binding(2, 2).scope)),
            _ => {
                lease.revoke();
                Some(CallGrant::from_lease(lease.clone(), binding(1, 2).scope))
            }
        };
        assert!(c.activate(&id, [2; 32], 0, grant).is_err());
        assert!(old.lease.is_live());
        assert!(c.owner(&current).is_some());
        if mode != 0 {
            assert!(!lease.is_live(), "failed activation drops its affine grant");
        }
    }
    c.end(&current);
    assert!(
        !old.lease.is_live(),
        "a retained Owner clone cannot postpone End"
    );
}

#[test]
fn native_pool_is_shared_with_actor_runtime_and_drop_revokes_retained_owner_clones() {
    let mut c = Coordinator::default();
    let runtime = c.runtime();
    assert!(Arc::ptr_eq(&c.resources, &runtime.resources()));
    let controller = controller(&mut c);
    let current = call(&mut c, &controller, 1, 1);
    let retained = c.owner(&current).unwrap();
    drop(c);
    assert!(!retained.lease.is_live());
}
