//! P2's successor classification, held to the handoff precondition it names.
//!
//! `overlay_successor_hold` is a classification over exactly the conditions
//! `check_overlay_successor` enforces. Every case here asserts both the reason AND that the hold is
//! `None` exactly when the check accepts, so the two cannot drift apart without a test noticing:
//! a hold that reported `None` where the handoff would refuse would tell a user their draft is
//! about to transfer when it never will.
use super::archive::branch as sealed_branch;
use super::handoff::{branch as transferable_branch, repair_bound_branch};
use super::*;
use crate::studio::{StudioOverlayManualReason as R, StudioOverlayState};
use crate::IntentLedger;

/// Classify three ways and demand they agree: the handoff's own check, the full hold over the
/// restored source, and the structural hold over the vault bytes that production actually calls.
///
/// The structural form does not check authorship (the record does not store the local actor; the
/// store compares author and device first), so for `NotCurrentAuthor` only the first two are held
/// together.
fn classify(
    source: &mut StudioEpoch,
    group: &ServerGroup,
    metadata: &StudioOverlayState,
    ledger: &IntentLedger,
) -> Option<R> {
    let overlay = metadata.overlay().unwrap();
    let hold = source.overlay_successor_hold(overlay, ledger).unwrap();
    assert_eq!(
        hold.is_none(),
        source.check_overlay_successor(overlay, ledger).is_ok(),
        "the hold ({hold:?}) must be None exactly when the handoff precondition accepts"
    );
    if hold != Some(R::NotCurrentAuthor) {
        let structural = StudioEpoch::overlay_successor_hold_in_vault(
            &source.snapshot().unwrap(),
            source.target,
            group.designated_committer(),
            overlay,
        )
        .unwrap();
        assert_eq!(
            structural, hold,
            "the structural hold production calls must name the same reason as the full one"
        );
    }
    hold
}

#[test]
fn a_pristine_successor_is_transferable_and_anything_else_names_why_not() {
    // Successor not installed yet: the source is still Closing at the closed epoch.
    let mut f = Fixture::new(true);
    let (metadata, ledger, _, _) = sealed_branch(&mut f, 1);
    assert_eq!(
        classify(&mut f.source, &f.group, &metadata, &ledger),
        Some(R::SuccessorMissing)
    );

    // The installed, untouched successor: the one state the handoff accepts.
    let mut f = Fixture::new(true);
    let (metadata, ledger, _) = transferable_branch(&mut f, 2);
    assert_eq!(classify(&mut f.source, &f.group, &metadata, &ledger), None);

    // Another device's view of the same source: only the branch's author may transfer it.
    let mut other = StudioEpoch::restore(
        &f.source.snapshot().unwrap(),
        &f.group,
        f.source.target,
        MlsDevice::generate().unwrap().device_id(),
    )
    .unwrap();
    assert_eq!(
        classify(&mut other, &f.group, &metadata, &ledger),
        Some(R::NotCurrentAuthor)
    );

    // The same pristine successor, but someone other than the receipt's owner now holds office:
    // the branch's receipt is no longer the current owner's. A single-member fixture cannot change
    // its committer, so the live owner is supplied directly, as the store supplies it.
    assert_eq!(
        StudioEpoch::overlay_successor_hold_in_vault(
            &f.source.snapshot().unwrap(),
            f.source.target,
            Some(MlsDevice::generate().unwrap().device_id()),
            metadata.overlay().unwrap(),
        )
        .unwrap(),
        Some(R::ReceiptChanged)
    );

    // Work landed on the successor: no longer the base this branch was built on.
    f.edit(f.title_body("someone else's edit on the successor"));
    assert_eq!(
        classify(&mut f.source, &f.group, &metadata, &ledger),
        Some(R::SuccessorNotPristine)
    );
}

/// Historical loser evidence retained by a repair is not a live competing opening.
///
/// The branch is based on the selected receipt and the installed successor is otherwise pristine.
/// All three gates must therefore accept the repair-bound (vault form 3) source exactly as they
/// accept form 2. This records the Agent 3 policy answer for the older review question: a repair
/// resolves the loser; its retained evidence alone is not a handoff refusal.
#[test]
fn a_repair_bound_successor_of_the_selected_receipt_is_transferable() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = repair_bound_branch(&mut f, 1);
        assert_eq!(classify(&mut f.source, &f.group, &metadata, &ledger), None);
    }
}

/// A cross-tenure repair does not turn its retained loser label into a successor hold.
///
/// This is the exact A -> B -> A' shape raised during Agent 1/3 integration. A's branch is based
/// on the receipt whose successor was installed. A later conflicting receipt faults that source;
/// when the same device becomes owner again, its repair selects the rival and deliberately leaves
/// the already-installed opening in place. Ordinary edits can continue there, so all three
/// successor predicates must keep accepting it. The separate live-authority check still refuses
/// the old receipt under A's *new* tenure; retained history is not allowed to erase the anti-replay
/// tenure boundary or mint current authority.
#[test]
fn cross_tenure_repair_keeps_a_loser_successor_eligible_without_reauthorizing_its_receipt() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let returning = f.rejoining_owner.take().unwrap();
        f.fill();
        let losing_decision = f.decide(None);
        let losing = losing_decision.receipt().clone();
        f.edit(f.title_body("late work outside the losing close"));
        let selected_decision = f.decide(None);
        let selected = selected_decision.receipt().clone();
        assert_ne!(selected.hash(), losing.hash());

        let losing_plan = f.plan(&losing_decision);
        let basis = f
            .source
            .prepare_closing_overlay(losing_decision.close(), &f.group, 0)
            .unwrap();
        let mut ledger = IntentLedger::new(f.source.document().clone());
        let mut metadata = StudioOverlayState::new(&basis);
        let op = f.domain(f.title_body("retained work on the losing close"));
        let id = ledger.prepare(f.owner.device_id(), op).unwrap();
        metadata.append(&basis, &ledger, id, 500).unwrap();
        f.source = f
            .source
            .checkpoint_successor(&losing_plan, &f.group, 0)
            .unwrap();
        assert_eq!(f.source.opening(), Some(&losing));
        assert_eq!(
            f.source.seal(selected.clone(), &f.group, 0).unwrap(),
            ReceiptIngest::Fault
        );

        // A -> B, then the same A identity rejoins as A' in a later observed tenure.
        let next = MlsDevice::generate().unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, next.key_package().unwrap())
            .unwrap()
            .welcome;
        let mut next_group = ServerGroup::join(&next, &welcome).unwrap();
        next_group
            .remove_member(&next, &f.owner.device_id())
            .unwrap();
        let welcome = next_group
            .add_member(&next, returning.key_package().unwrap())
            .unwrap()
            .welcome;
        f.group = ServerGroup::join(&returning, &welcome).unwrap();
        f.owner = returning;
        let returned_tenure = f.group.epoch();
        assert!(returned_tenure > 0);
        assert_eq!(f.group.designated_committer(), Some(f.owner.device_id()));
        f.restart();

        let repair = crate::ReceiptRepair::sign_in_tenure(
            selected.document.clone(),
            selected.tenure_id,
            [selected.hash(), losing.hash()],
            selected.hash(),
            1,
            returned_tenure,
            &f.owner,
        )
        .unwrap();
        f.source
            .apply_receipt_repair(&repair, &selected, &losing, &f.group, returned_tenure)
            .unwrap();
        f.restart();
        assert_eq!(f.source.phase(), EpochPhase::Open);
        assert_eq!(f.source.opening(), Some(&losing));
        assert_eq!(
            f.source.repair_state().unwrap().disposition,
            crate::RepairDisposition::Transitioned
        );
        assert_eq!(f.source.snapshot().unwrap()[0], 3);
        assert_eq!(
            classify(&mut f.source, &f.group, &metadata, &ledger),
            None,
            "the historical loser label alone became a successor hold"
        );
        assert!(matches!(
            metadata.handoff_authority(&f.owner, &f.group, returned_tenure),
            Err(ReplError::EpochAuthority)
        ));
    }
}

#[test]
fn a_source_that_moved_past_the_successor_is_replaced() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, _) = transferable_branch(&mut f, 1);
    let first = metadata.overlay().unwrap().receipt().clone();
    f.fill();
    let decision = f.decide(Some(&first));
    let plan = f.plan(&decision);
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    assert_eq!(f.source.epoch(), 2);
    assert_eq!(
        classify(&mut f.source, &f.group, &metadata, &ledger),
        Some(R::SourceReplaced)
    );
}

#[test]
fn a_source_at_the_closed_epoch_that_is_not_closing_is_named_as_such() {
    let mut f = Fixture::new(true);
    f.fill();
    // The same source before its seal: Open at the epoch the branch's receipt will close.
    let mut unsealed = StudioEpoch::restore(
        &f.source.snapshot().unwrap(),
        &f.group,
        f.source.target,
        f.owner.device_id(),
    )
    .unwrap();
    let decision = f.decide(None);
    let _plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut metadata = StudioOverlayState::new(&basis);
    let op = f.domain(f.title_body("work on the closing basis"));
    let id = ledger.prepare(f.owner.device_id(), op).unwrap();
    metadata.append(&basis, &ledger, id, 1).unwrap();

    assert_eq!(unsealed.phase(), EpochPhase::Open);
    assert_eq!(
        unsealed.epoch(),
        metadata.overlay().unwrap().receipt().closed_epoch
    );
    assert_eq!(
        classify(&mut unsealed, &f.group, &metadata, &ledger),
        Some(R::SourceNotClosing)
    );
}

#[test]
fn a_source_from_before_the_closed_epoch_is_rewound() {
    let mut f = Fixture::new(true);
    // Keep the epoch-0 source, then rotate once and base a branch on the epoch-1 close.
    let mut early = StudioEpoch::restore(
        &f.source.snapshot().unwrap(),
        &f.group,
        f.source.target,
        f.owner.device_id(),
    )
    .unwrap();
    f.fill();
    let first = f.decide(None);
    let plan = f.plan(&first);
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    f.fill();
    let second = f.decide(Some(first.receipt()));
    let _plan = f.plan(&second);
    let basis = f
        .source
        .prepare_closing_overlay(second.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut metadata = StudioOverlayState::new(&basis);
    let op = f.domain(f.title_body("work on the epoch-1 close"));
    let id = ledger.prepare(f.owner.device_id(), op).unwrap();
    metadata.append(&basis, &ledger, id, 1).unwrap();
    assert_eq!(metadata.overlay().unwrap().receipt().closed_epoch, 1);

    assert_eq!(early.epoch(), 0);
    assert_eq!(
        classify(&mut early, &f.group, &metadata, &ledger),
        Some(R::SourceRewound)
    );
}
