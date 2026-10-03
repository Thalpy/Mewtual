//! P2's successor classification, held to the handoff precondition it names.
//!
//! `overlay_successor_hold` is a classification over exactly the conditions
//! `check_overlay_successor` enforces. Every case here asserts both the reason AND that the hold is
//! `None` exactly when the check accepts, so the two cannot drift apart without a test noticing:
//! a hold that reported `None` where the handoff would refuse would tell a user their draft is
//! about to transfer when it never will.
use super::archive::branch as sealed_branch;
use super::handoff::branch as transferable_branch;
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
