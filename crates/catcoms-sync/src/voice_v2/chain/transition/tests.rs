//! These are original production custody owners, with no synthetic readiness/SDP capability.
use super::*;
use crate::voice_v2::chain::tests::{binding, prepare_chain};
use crate::voice_v2::runtime::{Runtime, View};
use catcoms_rt::ManualClock;

fn fixture(
    deadline: u64,
) -> (
    Arc<Runtime>,
    ActorOwner,
    Arc<PreparedChain>,
    Arc<Workflow>,
    Context,
) {
    let runtime = Runtime::new(Arc::new(ManualClock::new(1_000)));
    let mut binding = binding(1);
    binding.deadline = deadline;
    let context = Context {
        scope: binding.scope,
        local: binding.local,
        epoch: binding.epoch,
        now: 1_000,
        indirect: true,
    };
    let ticket = runtime.session_ticket();
    runtime.activate_session(&ticket).unwrap();
    let (mut actor, registration) = runtime
        .prepare_actor(binding.scope.group, binding.local, &ticket)
        .unwrap();
    registration.register().unwrap();
    actor
        .sync_view(View {
            epoch: binding.epoch,
            members: [binding.local, binding.peer].into(),
            policy: binding.scope.policy,
            eligible: true,
            indirect: true,
        })
        .unwrap();
    let chain = prepare_chain(
        runtime.resources(),
        binding,
        Arc::new(slots::Slots::default()),
    )
    .unwrap();
    let workflow = chain.workflow().unwrap();
    (runtime, actor, chain, workflow, context)
}
fn identity() -> FinishIdentity {
    FinishIdentity {
        message: [3; 32],
        hash: [4; 32],
    }
}

#[test]
fn await_ready_stale_expiry_claim_cannot_close_a_winning_finish_in_either_collector() {
    for process_first in [false, true] {
        let (runtime, actor, chain, workflow, context) = fixture(16_000);
        let window = chain.prepare_finish_window(&actor, &context).unwrap();
        // Park the ACTUAL shared collector decision after its Open/expired observation,
        // before CAS. Finish wins there; the production predicate must reload and return false.
        assert!(!chain
            .binding
            .transition
            .claim_expired_after_observation(16_000, 16_000, || {
                assert_eq!(chain.binding.transition.phase.load(Ordering::Acquire), OPEN);
                window.publish_at(15_999, identity()).unwrap();
            }));
        // Both actual collector entry points must reload the new deadline. No Clock is
        // rewound; these explicit observations reproduce the parked decision deterministically.
        if process_first {
            runtime.resources().collect_workflows(16_000);
            chain.collect_expired(16_000);
        } else {
            chain.collect_expired(16_000);
            runtime.resources().collect_workflows(16_000);
        }
        assert!(workflow.is_live());
        assert!(chain.lease().is_live());
        assert_eq!(chain.finish_window_for_test(), Some((identity(), 45_999)));
        // Generic confirmation matching keeps its ORIGINAL deadline despite custody surviving.
        assert!(!chain.matches(
            &chain.binding.value,
            &runtime.resources(),
            &chain.history.pool,
            16_000
        ));
        assert_eq!(chain.binding.deadline, 16_000);
        chain.collect_expired(45_998);
        assert!(workflow.is_live());
        runtime.resources().collect_workflows(45_999);
        assert!(!workflow.is_live());
        assert_eq!(
            window.publish_at(15_999, identity()),
            Err(VoiceError::Unauthorized)
        );
        assert_eq!(chain.finish_window_for_test(), Some((identity(), 45_999)));
    }
}

#[test]
fn await_ready_expiry_or_explicit_close_wins_before_arm_and_cannot_reopen() {
    for explicit in [false, true] {
        let (_runtime, actor, chain, workflow, context) = fixture(16_000);
        let window = chain.prepare_finish_window(&actor, &context).unwrap();
        if explicit {
            chain.close();
        } else {
            workflow.collect_expired(16_000);
        }
        assert_eq!(
            window.publish_at(15_999, identity()),
            Err(VoiceError::Unauthorized)
        );
        assert!(chain.finish_window_for_test().is_none());
        assert!(!workflow.is_live());
        assert!(chain.prepare_finish_window(&actor, &context).is_err());
    }
}

#[test]
fn await_ready_original_deadline_and_overflow_refuse_without_changing_phase() {
    for (deadline, mono) in [
        (16_000, 16_000),
        (u64::MAX, u64::MAX - AWAIT_READY_MS),
        (u64::MAX, u64::MAX - 1),
    ] {
        let (_runtime, actor, chain, _, context) = fixture(deadline);
        let window = chain.prepare_finish_window(&actor, &context).unwrap();
        assert!(window.publish_at(mono, identity()).is_err());
        assert_eq!(chain.binding.transition.phase.load(Ordering::Acquire), OPEN);
        assert!(chain.finish_window_for_test().is_none());
    }
}

#[test]
fn await_ready_arm_uses_fixed_payment_even_when_ordinary_pool_and_crypto_slots_are_full() {
    let (runtime, actor, chain, workflow, context) = fixture(16_000);
    let window = chain.prepare_finish_window(&actor, &context).unwrap();
    let resources = runtime.resources();
    let group = context.scope.group;
    let slots: Vec<_> = (0..3)
        .map(|_| resources.charge(group, 0, ResourceKind::Crypto).unwrap())
        .collect();
    let fill = resources
        .charge(
            group,
            super::super::super::resources::GROUP_BYTES - resources.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = resources.counts();
    window.publish_at(15_999, identity()).unwrap();
    assert_eq!(resources.counts(), before);
    assert!(workflow.is_live());
    drop((fill, slots));
}

#[test]
fn await_ready_epoch_policy_session_and_pair_revocation_close_only_original_custody() {
    for change in ["epoch", "policy", "indirect", "session", "pair"] {
        let (runtime, mut actor, chain, workflow, context) = fixture(16_000);
        let window = chain.prepare_finish_window(&actor, &context).unwrap();
        window.publish_at(1_000, identity()).unwrap();
        match change {
            "session" => runtime.revoke_session(),
            "pair" => chain.binding.pair.revoke(),
            _ => {
                actor
                    .sync_view(View {
                        epoch: context.epoch + u64::from(change == "epoch"),
                        members: [chain.binding.local, chain.binding.peer].into(),
                        policy: if change == "policy" {
                            Some([8; 32])
                        } else {
                            context.scope.policy
                        },
                        eligible: true,
                        indirect: change != "indirect",
                    })
                    .unwrap();
            }
        }
        runtime.resources().collect_workflows(1_001);
        assert!(!workflow.is_live(), "{change}");
        assert!(!chain.lease().is_live(), "{change}");
        assert_eq!(
            chain.finish_window_for_test(),
            Some((identity(), 31_000)),
            "{change}"
        );
    }
}

#[test]
fn await_ready_expiry_preserves_real_lower_and_handoff_payment_until_actual_drop() {
    let (runtime, actor, chain, workflow, context) = fixture(16_000);
    let window = chain.prepare_finish_window(&actor, &context).unwrap();
    let lower = workflow
        .charge(
            &runtime.resources(),
            context.scope.group,
            1_024,
            ResourceKind::Forward,
        )
        .unwrap();
    let handoff = workflow.borrow_handoff(16 * 1024).unwrap();
    let control = workflow.confirmation_control();
    window.publish_at(15_999, identity()).unwrap();
    chain.collect_expired(45_999);
    assert!(!control.is_drained());
    let after_close = runtime.resources().counts();
    drop(workflow);
    assert!(!control.is_drained());
    assert_eq!(runtime.resources().counts(), after_close);
    drop(handoff);
    assert!(!control.is_drained());
    drop(lower);
    assert!(control.is_drained());
    let history = control.confirmed_finish().unwrap();
    assert_eq!(
        (
            history.message(),
            history.object_hash(),
            history.transition_until()
        ),
        ([3; 32], [4; 32], 45_999)
    );
}

#[test]
fn await_ready_finish_history_after_actor_runtime_drop_pays_complete_small_observation_graph() {
    let (runtime, actor, chain, workflow, context) = fixture(16_000);
    let window = chain.prepare_finish_window(&actor, &context).unwrap();
    window.publish_at(1_000, identity()).unwrap();
    let resources = runtime.resources();
    let control = workflow.confirmation_control();
    // Neither the actor's1024-byte body nor its Policy/watch graph is retained by this history.
    // The complete small observation cells are included in the structural2KiB growth guard.
    drop((window, chain, workflow, actor, runtime));
    assert!(control.is_drained());
    assert_eq!(resources.counts(), (METADATA_BYTES, 0));
    assert!(control.confirmed_finish().is_some());
    drop(control);
    assert_eq!(resources.counts(), (0, 0));
}

#[test]
fn await_ready_prepared_arm_cannot_reopen_confirmation_after_manual_capacity_transfer() {
    let (_runtime, actor, chain, workflow, context) = fixture(16_000);
    let window = chain.prepare_finish_window(&actor, &context).unwrap();
    let control = workflow.confirmation_control();
    let capacity = control.prepare_sdp_capacity().unwrap();
    let ticket = control.transfer_sdp_capacity(capacity).unwrap();
    assert_eq!(
        window.publish_at(1_000, identity()),
        Err(VoiceError::Unauthorized)
    );
    assert!(control.confirmed_finish().is_none());
    assert!(!workflow.is_live());
    drop(ticket);
}
