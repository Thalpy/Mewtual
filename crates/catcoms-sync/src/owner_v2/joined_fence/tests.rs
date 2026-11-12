//! Real founder and invite-joined nodes, real Add/Remove commits taken from the founder's own
//! commit log (the exact broadcast bytes), and the real contest window. These pin the fence's
//! ordering and recording contract. `OWN` is an explicit test label for the source's own
//! Admission bytes; nothing here establishes governance, selection or signing.
use super::*;
use crate::*;
use catcoms_rt::{Hub, ManualClock, MemNetwork, PeerId};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

type Node = ChannelSync<MemNetwork, ChaCha20Rng>;
const OWN: [u8; 32] = [0x5a; 32];

fn founder(peer: u64) -> Node {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    ChannelSync::new(
        Hub::new().join(PeerId::from_u64(peer)),
        group,
        owner,
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(ManualClock::new(1_000)),
    )
}

/// A genuinely invite-joined node, its fence (taken with its join facts) and its clock.
fn join(founder: &mut Node, peer: u64, nonce: u8) -> (Node, V2JoinedSourceFence, ManualClock) {
    let joiner = MlsDevice::generate().unwrap();
    let invite = founder
        .mint_invite([nonce; 16], 10_000, Vec::new())
        .unwrap();
    let package = joiner
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    let expected = joiner.key_package_reference(&package).unwrap();
    let (welcome, routing, signature) = founder
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    let (group, transferred) =
        finish_join(&joiner, &invite, &welcome, &signature, &routing, &expected).unwrap();
    let clock = ManualClock::new(1_000);
    let mut node = ChannelSync::new_joined(
        Hub::new().join(PeerId::from_u64(peer)),
        group,
        joiner,
        ChaCha20Rng::seed_from_u64(peer),
        Box::new(clock.clone()),
        transferred,
    );
    let instance = node.registry_instance();
    let (_facts, fence) = node.take_v2_joined_join_facts(&instance).unwrap();
    (node, fence, clock)
}

/// The founder admits a fresh device; returns the exact commit record it broadcast and the new
/// member's `DeviceId` bytes.
fn founder_adds(founder: &mut Node, nonce: u8) -> (CommitRecord, [u8; 32]) {
    let device = MlsDevice::generate().unwrap();
    let id = *device.device_id().as_bytes();
    let invite = founder
        .mint_invite([nonce; 16], 10_000, Vec::new())
        .unwrap();
    let package = device
        .key_package_for_invite(&invite.group_id, invite.invite_nonce)
        .unwrap();
    founder
        .admit_now(&invite, &serialize_key_package(&package).unwrap(), 1_000)
        .unwrap();
    (founder.commit_log.back().unwrap().clone(), id)
}

fn digest(record: &CommitRecord) -> Option<[u8; 32]> {
    Some(*blake3::hash(&record.mls_commit).as_bytes())
}

fn settle_at_join(node: &Node, fence: &V2JoinedSourceFence) {
    fence
        .install_selected(node.epoch(), node.group.epoch_authenticator_id(), OWN)
        .unwrap();
}

#[test]
fn an_in_order_add_is_recorded_with_its_exact_commit_and_pauses_until_covered() {
    let mut f = founder(0xd101);
    let (mut b, fence, _clock) = join(&mut f, 0xd102, 0xd1);
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Unselected),
        "no governance installed yet"
    );
    settle_at_join(&b, &fence);
    fence.require_authorized(&OWN).unwrap();
    assert_eq!(
        fence.require_authorized(&[0; 32]),
        Err(V2JoinedFenceRefusal::NotOwn)
    );

    let before = b.epoch();
    let (record, added) = founder_adds(&mut f, 0xd2);
    assert!(b.apply_commit_in_order(&record));
    let transition = fence.transition_data(before).unwrap().unwrap();
    assert_eq!(transition.before_epoch_data(), before);
    assert_eq!(transition.after_epoch_data(), b.epoch());
    assert_eq!(transition.commit_data(), digest(&record));
    assert_eq!(
        transition.authenticator_data(),
        &b.group.epoch_authenticator_id()
    );
    assert_eq!(transition.delta_data(), V2TransitionDelta::Added(added));
    assert!(transition.own_active_data());
    assert_eq!(
        format!("{:?}", transition.delta_data()),
        "Added(..)",
        "member keys never reach logs"
    );

    // The node moved past the installed governance: authorization pauses until covered.
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Unselected)
    );
    // A state this node never finalized cannot be installed.
    assert_eq!(
        fence.install_selected(b.epoch(), [9; 32], OWN),
        Err(V2JoinedFenceRefusal::Unknown)
    );
    fence
        .install_selected(
            transition.after_epoch_data(),
            *transition.authenticator_data(),
            OWN,
        )
        .unwrap();
    fence.require_authorized(&OWN).unwrap();
    assert_eq!(fence.transition_data(before).unwrap(), None);
    assert_eq!(fence.observe_data().unwrap().unconsumed, 0);
    // The ring no longer carries this transition. Exact retry and later Appends at this
    // same MLS state must still recognize the previously established selection.
    fence
        .install_selected(b.epoch(), b.group.epoch_authenticator_id(), OWN)
        .unwrap();
    assert_eq!(
        fence.install_selected(b.epoch(), [9; 32], OWN),
        Err(V2JoinedFenceRefusal::Unknown)
    );
    assert_eq!(
        fence.install_selected(b.epoch(), b.group.epoch_authenticator_id(), [1; 32]),
        Err(V2JoinedFenceRefusal::Unknown)
    );
    fence.require_authorized(&OWN).unwrap();
}

#[test]
fn a_current_remove_refusal_closes_before_the_caller_can_handle_the_error() {
    for direct in [false, true] {
        let mut f = founder(0xd201);
        let (mut b, fence, _clock) = join(&mut f, 0xd202, 0xe3);
        settle_at_join(&b, &fence);
        let epoch = b.epoch();
        let authenticator = b.group.epoch_authenticator_id();
        f.commit_remove_now(&b.device.device_id());
        let record = f.commit_log.back().unwrap().clone();
        assert!(b.authorize_committer(&record));
        assert!(b.commit_record_refusal(&record).is_none());
        let _fault = super::faults::arm(&b, &record.mls_commit, super::faults::Cut::RefuseCommit);
        if direct {
            // Check the settlement boundary BEFORE the in-order caller's Err arm exists.
            // Moving revocation into that caller would leave this path authorized.
            let result = b.with_observed_mls_commit(&record.mls_commit, |node| {
                node.group
                    .apply_commit(&node.device, &record.mls_commit, record.commit_epoch)
            });
            assert!(result.is_err());
            assert_eq!(
                fence.admit_final(&OWN).unwrap_err(),
                V2JoinedFenceRefusal::Closed
            );
        } else {
            assert!(!b.apply_commit_in_order(&record));
        }
        assert_eq!(b.epoch(), epoch);
        assert_eq!(b.group.epoch_authenticator_id(), authenticator);
        assert!(
            b.group.is_active(),
            "the fault prevented the real SDK apply"
        );
        let observed = fence.observe_data().unwrap();
        assert!(!observed.merging && !observed.contested);
        assert_eq!(
            observed.terminal,
            Some(V2JoinedFenceTerminal::Diverged { epoch })
        );
        assert_eq!(fence.transition_data(epoch).unwrap(), None);
        assert_eq!(
            fence.require_authorized(&OWN),
            Err(V2JoinedFenceRefusal::Closed)
        );
        assert_eq!(
            fence.admit_final(&OWN).unwrap_err(),
            V2JoinedFenceRefusal::Closed
        );
        assert_eq!(
            fence.install_selected(epoch, authenticator, OWN),
            Err(V2JoinedFenceRefusal::Closed)
        );
    }
}

#[test]
fn a_failed_real_contest_winner_cannot_reopen_its_previous_selected_authority() {
    let mut f = founder(0xd211);
    let (mut b, fence, clock) = join(&mut f, 0xd212, 0xe4);
    settle_at_join(&b, &fence);
    let epoch = b.epoch();
    f.commit_remove_now(&b.device.device_id());
    let record = f.commit_log.back().unwrap().clone();
    b.config.stage_decision_window_ms = 10;
    b.contest_commit(record.clone());
    assert!(fence.observe_data().unwrap().contested);
    let _fault = super::faults::arm(&b, &record.mls_commit, super::faults::Cut::RefuseCommit);
    clock.set_wall_ms(2_000);
    assert!(b.resolve_pending_if_expired());
    assert_eq!(b.epoch(), epoch);
    let observed = fence.observe_data().unwrap();
    assert!(!observed.merging && !observed.contested);
    assert_eq!(
        observed.terminal,
        Some(V2JoinedFenceTerminal::Diverged { epoch })
    );
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Closed
    );
    assert_eq!(fence.transition_data(epoch).unwrap(), None);
}

#[test]
fn an_error_after_actual_apply_retains_the_exact_transition_and_closes_admission() {
    for remove_self in [false, true] {
        let mut f = founder(0xd251);
        let (mut b, fence, _clock) = join(&mut f, 0xd252, 0xea);
        settle_at_join(&b, &fence);
        let epoch = b.epoch();
        let (record, expected_delta) = if remove_self {
            let own = *b.device.device_id().as_bytes();
            f.commit_remove_now(&b.device.device_id());
            (
                f.commit_log.back().unwrap().clone(),
                V2TransitionDelta::Removed(own),
            )
        } else {
            let (record, added) = founder_adds(&mut f, 0xeb);
            (record, V2TransitionDelta::Added(added))
        };
        let result: Result<(), MlsError> = b.with_observed_mls_commit(&record.mls_commit, |node| {
            node.group
                .apply_commit(&node.device, &record.mls_commit, record.commit_epoch)?;
            Err(MlsError::Internal("test-only failure after actual apply"))
        });
        assert!(result.is_err());
        assert_eq!(b.epoch(), epoch + 1);
        let transition = fence.transition_data(epoch).unwrap().unwrap();
        assert_eq!(transition.commit_data(), digest(&record));
        assert_eq!(transition.delta_data(), expected_delta);
        assert_eq!(transition.own_active_data(), !remove_self);
        assert_eq!(
            fence.observe_data().unwrap().terminal,
            Some(if remove_self {
                V2JoinedFenceTerminal::Removed
            } else {
                V2JoinedFenceTerminal::Diverged { epoch }
            })
        );
        assert_eq!(
            fence.admit_final(&OWN).unwrap_err(),
            V2JoinedFenceRefusal::Closed
        );
    }
}

#[test]
fn an_uncertain_merge_panic_closes_without_inventing_an_applied_transition() {
    let mut f = founder(0xd221);
    let (mut b, fence, _clock) = join(&mut f, 0xd222, 0xe5);
    settle_at_join(&b, &fence);
    let epoch = b.epoch();
    let (record, _) = founder_adds(&mut f, 0xe6);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), MlsError> = b.with_observed_mls_commit(&record.mls_commit, |_node| {
            panic!("merge work unwound before a result");
        });
    }));
    assert!(failed.is_err());
    assert_eq!(b.epoch(), epoch);
    assert_eq!(fence.transition_data(epoch).unwrap(), None);
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Closed
    );
}

#[test]
fn an_observer_hook_panic_keeps_actual_transition_data_but_closes_authority() {
    let mut f = founder(0xd231);
    let (mut b, fence, _clock) = join(&mut f, 0xd232, 0xe7);
    settle_at_join(&b, &fence);
    let epoch = b.epoch();
    let (record, _) = founder_adds(&mut f, 0xe8);
    let _fault = super::faults::arm(
        &b,
        &record.mls_commit,
        super::faults::Cut::PanicAfterSettlement,
    );
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        b.apply_commit_in_order(&record);
    }));
    assert!(failed.is_err());
    assert_eq!(b.epoch(), epoch + 1);
    assert_eq!(
        fence.transition_data(epoch).unwrap().unwrap().commit_data(),
        digest(&record)
    );
    assert!(fence.is_closed());
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Closed
    );
    assert_eq!(
        fence.install_selected(b.epoch(), b.group.epoch_authenticator_id(), OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
}

#[test]
fn a_section_entered_during_unwind_cannot_leave_atomic_and_positive_gates_in_disagreement() {
    struct EnterDuringUnwind<'a> {
        fence: &'a V2JoinedSourceFence,
        epoch: u64,
        authenticator: [u8; 32],
    }
    impl Drop for EnterDuringUnwind<'_> {
        fn drop(&mut self) {
            self.fence
                .install_selected(self.epoch, self.authenticator, OWN)
                .unwrap();
        }
    }
    let mut f = founder(0xd241);
    let (b, fence, _clock) = join(&mut f, 0xd242, 0xe9);
    settle_at_join(&b, &fence);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _inside = EnterDuringUnwind {
            fence: &fence,
            epoch: b.epoch(),
            authenticator: b.group.epoch_authenticator_id(),
        };
        panic!("outer unwind");
    }));
    assert!(failed.is_err());
    assert!(
        !fence.inner.state.is_poisoned(),
        "std need not poison this acquisition"
    );
    assert!(fence.is_closed());
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Closed
    );
    assert_eq!(
        fence.install_selected(b.epoch(), b.group.epoch_authenticator_id(), OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
}

#[test]
fn final_admission_holds_the_fence_and_a_second_edge_never_waits() {
    let mut f = founder(0xd111);
    let (b, fence, _clock) = join(&mut f, 0xd112, 0xd3);
    settle_at_join(&b, &fence);
    let admission = fence.admit_final(&OWN).unwrap();
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Busy
    );
    // The negative probe is an atomic load and never touches the held mutex.
    assert!(!fence.is_closed());
    drop(admission);
    assert_eq!(
        fence.admit_final(&[1; 32]).unwrap_err(),
        V2JoinedFenceRefusal::NotOwn
    );
    drop(fence.admit_final(&OWN).unwrap());
}

#[test]
fn self_removal_closes_the_fence_and_keeps_its_transition_readable() {
    let mut f = founder(0xd121);
    let (mut b, fence, _clock) = join(&mut f, 0xd122, 0xd4);
    let own_id = *b.device.device_id().as_bytes();
    settle_at_join(&b, &fence);
    let before = b.epoch();
    f.commit_remove_now(&b.device.device_id());
    let record = f.commit_log.back().unwrap().clone();
    assert!(b.apply_commit_in_order(&record));
    assert!(fence.is_closed());
    assert_eq!(
        fence.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::Removed)
    );
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Closed
    );
    // Historical DATA for replaying the Remove(self) record stays readable.
    let transition = fence.transition_data(before).unwrap().unwrap();
    assert_eq!(transition.delta_data(), V2TransitionDelta::Removed(own_id));
    assert!(!transition.own_active_data());
    assert_eq!(transition.commit_data(), digest(&record));
    // Removal is final: no governed install reopens it.
    assert_eq!(
        fence.install_selected(
            transition.after_epoch_data(),
            *transition.authenticator_data(),
            OWN
        ),
        Err(V2JoinedFenceRefusal::Closed)
    );
}

#[test]
fn a_change_without_its_commit_bytes_is_an_unaccountable_history_gap() {
    let mut f = founder(0xd131);
    let (mut b, fence, _clock) = join(&mut f, 0xd132, 0xd5);
    settle_at_join(&b, &fence);
    let before = b.epoch();
    let (record, _) = founder_adds(&mut f, 0xd6);
    // The same real commit, merged through the bytes-less seam.
    b.with_observed_mls_transition(|node| {
        node.group
            .apply_commit(&node.device, &record.mls_commit, record.commit_epoch)
    })
    .unwrap();
    assert_eq!(b.epoch(), before + 1);
    assert!(fence.is_closed());
    assert_eq!(
        fence.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::HistoryGap { epoch: before })
    );
    assert_eq!(fence.transition_data(before).unwrap(), None);
}

#[test]
fn a_failed_merge_records_nothing_and_leaves_authority_exactly_as_before() {
    let mut f = founder(0xd141);
    let (mut b, fence, _clock) = join(&mut f, 0xd142, 0xd7);
    settle_at_join(&b, &fence);
    let epoch = b.epoch();
    assert!(b
        .with_observed_mls_commit(b"not a commit", |node| {
            node.group
                .apply_commit(&node.device, b"not a commit", epoch)
        })
        .is_err());
    assert_eq!(b.epoch(), epoch);
    fence.require_authorized(&OWN).unwrap();
    let observed = fence.observe_data().unwrap();
    assert!(!observed.merging);
    assert_eq!(observed.unconsumed, 0);
    assert_eq!(observed.terminal, None);
}

#[test]
fn a_pending_contest_pauses_authorization_until_its_merge_settles() {
    let mut f = founder(0xd151);
    let (mut b, fence, clock) = join(&mut f, 0xd152, 0xd8);
    settle_at_join(&b, &fence);
    let before = b.epoch();
    let (record, added) = founder_adds(&mut f, 0xd9);
    b.config.stage_decision_window_ms = 10;
    b.contest_commit(record.clone());
    assert!(b.pending.is_some(), "a contest is open, nothing applied");
    assert_eq!(b.epoch(), before);
    assert!(fence.observe_data().unwrap().contested);
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Transitioning)
    );
    assert_eq!(
        fence.admit_final(&OWN).unwrap_err(),
        V2JoinedFenceRefusal::Transitioning
    );
    clock.set_wall_ms(2_000);
    assert!(b.resolve_pending_if_expired());
    let observed = fence.observe_data().unwrap();
    assert!(!observed.contested && !observed.merging);
    let transition = fence.transition_data(before).unwrap().unwrap();
    assert_eq!(transition.commit_data(), digest(&record));
    assert_eq!(transition.delta_data(), V2TransitionDelta::Added(added));
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Unselected)
    );
    fence
        .install_selected(
            transition.after_epoch_data(),
            *transition.authenticator_data(),
            OWN,
        )
        .unwrap();
    fence.require_authorized(&OWN).unwrap();
}

#[test]
fn a_panic_while_final_admission_is_held_closes_the_fence_for_lock_free_negatives() {
    let mut f = founder(0xd171);
    let (b, fence, _clock) = join(&mut f, 0xd172, 0xdd);
    settle_at_join(&b, &fence);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _admission = fence.admit_final(&OWN).unwrap();
        panic!("issuing the save-source end failed");
    }));
    assert!(unwound.is_err());
    // Closed by the admission's own unwind guard, before the lock poisoned.
    assert!(fence.is_closed());
    assert_eq!(
        fence.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
    drop(b);
}

#[test]
fn a_joined_nodes_own_add_is_recorded_with_the_commit_from_its_result() {
    let mut f = founder(0xd181);
    let (mut b, fence, _clock) = join(&mut f, 0xd182, 0xde);
    let before = b.epoch();
    let device = MlsDevice::generate().unwrap();
    let added = *device.device_id().as_bytes();
    let package = device
        .key_package_for_invite(&b.group_id(), [0xdf; 16])
        .unwrap();
    let outcome = b
        .with_observed_mls_add(|node| node.group.add_member(&node.device, package))
        .unwrap();
    let transition = fence.transition_data(before).unwrap().unwrap();
    assert_eq!(
        transition.commit_data(),
        Some(*blake3::hash(&outcome.commit).as_bytes())
    );
    assert_eq!(transition.delta_data(), V2TransitionDelta::Added(added));
    // Recorded, not a gap: whether a member's own Add is governed is the source's decision.
    assert!(!fence.is_closed());
}

#[test]
fn a_mutation_outside_the_observer_makes_the_next_bracket_a_history_gap() {
    let mut f = founder(0xd191);
    let (mut b, fence, _clock) = join(&mut f, 0xd192, 0xe1);
    settle_at_join(&b, &fence);
    let (record, _) = founder_adds(&mut f, 0xe2);
    // Bypass the observer entirely.
    b.group
        .apply_commit(&b.device, &record.mls_commit, record.commit_epoch)
        .unwrap();
    let epoch = b.epoch();
    // Even a merge that changes nothing now finds the node moved since its last settlement.
    assert!(b
        .with_observed_mls_commit(b"not a commit", |node| {
            node.group
                .apply_commit(&node.device, b"not a commit", epoch)
        })
        .is_err());
    assert!(fence.is_closed());
    assert!(matches!(
        fence.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::HistoryGap { .. })
    ));
}

#[test]
fn drop_cancel_and_fork_each_close_for_good() {
    let mut f = founder(0xd161);
    let (b, dropped, _clock) = join(&mut f, 0xd162, 0xda);
    drop(b);
    assert!(dropped.is_closed());
    assert_eq!(
        dropped.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::Dropped)
    );

    let (c, cancelled, _clock) = join(&mut f, 0xd163, 0xdb);
    settle_at_join(&c, &cancelled);
    cancelled.cancel();
    assert!(cancelled.is_closed());
    assert_eq!(
        cancelled.install_selected(c.epoch(), c.group.epoch_authenticator_id(), OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );

    let (d, forked, _clock) = join(&mut f, 0xd164, 0xdc);
    settle_at_join(&d, &forked);
    forked.record_fork();
    // A later cancel does not rewrite the first terminal reason.
    forked.cancel();
    assert_eq!(
        forked.observe_data().unwrap().terminal,
        Some(V2JoinedFenceTerminal::Forked)
    );
    assert_eq!(
        forked.require_authorized(&OWN),
        Err(V2JoinedFenceRefusal::Closed)
    );
}
