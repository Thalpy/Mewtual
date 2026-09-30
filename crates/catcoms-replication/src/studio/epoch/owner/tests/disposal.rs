//! The terminal disposal manifest and the v3 extension arm, against real branches built through
//! the production basis path, for the same reason the archive tests do: a synthetic basis would
//! need a test-only constructor on `StudioClosingOverlayBasis`, which is the back door the overlay
//! design forbids.
use super::archive::{branch, unreplayable_branch};
use super::handoff::{branch as transferable_branch, signing};
use super::*;
use crate::studio::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioDisposalMode, StudioOverlayProvenance,
};
use crate::IntentLedger;

/// Dispose the fixture's branch, returning the rebuilt state and the ids it retired.
#[allow(clippy::type_complexity)]
fn dispose(
    metadata: &StudioOverlayState,
    ledger: &IntentLedger,
    decision: StudioDisposalDecision,
) -> Result<(StudioOverlayState, std::collections::BTreeSet<[u8; 32]>), ReplError> {
    let content = metadata.branch_content(ledger)?;
    metadata.dispose(
        ledger,
        decision,
        [0x5b; 32],
        content,
        3,
        StudioOverlayProvenance::Closing,
        9,
        1_700_000_000_000,
    )
}

fn confirmation() -> StudioDiscardConfirmation {
    StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap()
}

/// The branch's own accepted ids, derived from the authored operations rather than from an
/// accessor added for the test: `DomainOp::id` is how the ledger derived them in the first place.
fn authored_ids(f: &Fixture, ordered: &[(DomainOp, u64)]) -> std::collections::BTreeSet<[u8; 32]> {
    ordered
        .iter()
        .map(|(op, _)| op.id(&f.owner.device_id()))
        .collect()
}

#[test]
fn a_disposal_round_trips_and_retires_exactly_the_branch_ids() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered, _basis) = branch(&mut f, 3);
    let live = authored_ids(&f, &ordered);
    assert_eq!(live.len(), ordered.len());

    let (next, removed) = dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Preserve {
            archive: [0xa7; 32],
        },
    )
    .unwrap();

    assert_eq!(
        removed, live,
        "the retired set must be exactly the branch's own ids, read once rather than recomputed"
    );
    assert!(
        next.overlay().is_none(),
        "the branch must be gone: clearing it is the whole transition"
    );
    let disposal = next.disposed().expect("the manifest must be retained");
    assert_eq!(disposal.accepted, ordered.len());
    assert_eq!(disposal.generation, 3);
    assert_eq!(disposal.sequence, 9);
    assert_eq!(disposal.at, 1_700_000_000_000);
    assert_eq!(disposal.author, f.owner.device_id());
    assert_eq!(
        disposal.mode,
        StudioDisposalMode::Preserved {
            archive: [0xa7; 32]
        }
    );

    // Through the real extension codec, which also runs the canonical re-encode check.
    let bytes = next.encode_vault(&ledger).unwrap();
    let read = StudioOverlayState::decode_vault(&bytes, &ledger).unwrap();
    assert_eq!(read.disposed(), next.disposed());
    assert_eq!(read.encode_vault(&ledger).unwrap(), bytes);
}

/// The version byte is evidence, not a build stamp. A vault that has never held a disposal must
/// re-encode byte-identically to before this field existed, or every existing record would fail
/// its own canonical re-encode check on upgrade.
#[test]
fn the_extension_version_is_two_until_a_disposal_exists_and_three_after() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, _ordered, _basis) = branch(&mut f, 2);

    let before = metadata.encode_vault(&ledger).unwrap();
    assert_eq!(
        before.first(),
        Some(&2),
        "a branch with no disposal must still encode as v2: this is the byte that proves adding \
         the field did not rewrite every existing record"
    );

    let (next, _) = dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Discard(
            StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
        ),
    )
    .unwrap();
    let after = next.encode_vault(&ledger).unwrap();
    assert_eq!(
        after.first(),
        Some(&3),
        "a retained disposal must raise the version, or a reader cannot know it is there"
    );
}

/// D5's type-level half. Only the exact literal constructs a confirmation: a near miss is a caller
/// that built the string itself instead of echoing what the user typed, and the release token is
/// the specific confusion worth refusing by name.
#[test]
fn only_the_exact_discard_literal_constructs_a_confirmation() {
    assert!(StudioDiscardConfirmation::parse("destroy-local-draft").is_some());
    for wrong in [
        "",
        " destroy-local-draft",
        "destroy-local-draft ",
        "Destroy-Local-Draft",
        "DESTROY-LOCAL-DRAFT",
        "destroy-local-drafts",
        "destroy-local-draf",
        "release-local-archive",
        "yes",
        "true",
    ] {
        assert!(
            StudioDiscardConfirmation::parse(wrong).is_none(),
            "{wrong:?} must not construct a discard confirmation"
        );
    }
}

/// The two modes must be distinguishable in the bytes. A preserved disposal names the archive that
/// holds the bodies; a discarded one names nothing, because there is nothing to name. If these
/// encoded alike, a reader could not tell "the work is somewhere else" from "the user destroyed
/// it", which is the single most important thing this record says.
#[test]
fn preserved_and_discarded_disposals_are_distinguishable_on_disk() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, _ordered, _basis) = branch(&mut f, 2);

    let (preserved, _) = dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Preserve {
            archive: [0xa7; 32],
        },
    )
    .unwrap();
    let (discarded, _) = dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Discard(
            StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
        ),
    )
    .unwrap();

    let a = preserved.encode_vault(&ledger).unwrap();
    let b = discarded.encode_vault(&ledger).unwrap();
    assert_ne!(a, b, "the two modes must not encode identically");

    assert_eq!(
        StudioOverlayState::decode_vault(&a, &ledger)
            .unwrap()
            .disposed()
            .unwrap()
            .mode,
        StudioDisposalMode::Preserved {
            archive: [0xa7; 32]
        }
    );
    assert_eq!(
        StudioOverlayState::decode_vault(&b, &ledger)
            .unwrap()
            .disposed()
            .unwrap()
            .mode,
        StudioDisposalMode::Discarded
    );
}

/// D2 at this layer. A Prepared branch carries a live transfer hold: someone else may be about to
/// accept this work, and dropping it here would race that acceptance.
#[test]
fn a_prepared_branch_refuses_disposal() {
    let mut f = Fixture::new(true);
    // This case needs the fixture that advances to a successor epoch: without one there is
    // nothing to hand off into, `handoff_authority` refuses, and the test would fail while
    // building its own transfer hold rather than while disposing.
    let (metadata, ledger, _ordered) = transferable_branch(&mut f, 2);

    // A real transfer hold through the real preparation path, not a hand-set flag.
    let mut batch = signing(&mut f, &metadata, &ledger);
    while batch.remaining() > 0 {
        batch.sign_next(&f.owner, &f.group, 0).unwrap();
    }
    let (_source, prepared) = batch.finish().unwrap().into_parts();

    let content = prepared.branch_content(&ledger).unwrap();
    let refused = prepared.dispose(
        &ledger,
        StudioDisposalDecision::Discard(confirmation()),
        [0x5b; 32],
        content,
        3,
        StudioOverlayProvenance::Closing,
        9,
        1,
    );
    assert!(
        refused.is_err(),
        "a branch under a transfer hold must refuse disposal outright"
    );
    // Guard the guard: the same call on the un-prepared state succeeds, so the refusal is
    // attributable to the hold and not to the arguments.
    assert!(dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Discard(confirmation())
    )
    .is_ok());
}

/// D3's content half, enforced where the branch can actually be hashed. A UI that has not
/// re-inspected since the branch changed is naming work the user did not see, and the whole point
/// of carrying `content` into the request is to refuse that rather than dispose of it.
#[test]
fn a_disposal_naming_the_wrong_branch_content_refuses() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, _ordered, _basis) = branch(&mut f, 2);
    let right = metadata.branch_content(&ledger).unwrap();
    let mut wrong = right;
    wrong[0] ^= 0xff;

    let refused = metadata.dispose(
        &ledger,
        StudioDisposalDecision::Discard(
            StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
        ),
        [0x5b; 32],
        wrong,
        3,
        StudioOverlayProvenance::Closing,
        9,
        1,
    );
    assert!(
        refused.is_err(),
        "a stale content hash must refuse, or a disposal destroys work the user never saw"
    );
    // The right one still works, so the refusal was about the hash and not a blanket failure.
    assert!(dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Discard(
            StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap(),
        ),
    )
    .is_ok());
}

/// A branch that survives structural validation but cannot be replayed must still be disposable.
/// Refusing here would leave exactly the drafts most in need of disposal undisposable, which is the
/// same reasoning that makes the archive build from structural entries.
///
/// Stated on the manifest builder rather than on `dispose`, because wrapping a bare overlay into a
/// `StudioOverlayState` would need a test-only constructor, and that is the back door the overlay
/// design forbids. The builder is where the property actually lives: it takes `checked_entries`,
/// which is structural, so nothing in the manifest path can depend on a replay succeeding.
#[test]
fn an_unreplayable_branch_still_yields_a_complete_disposal_manifest() {
    let mut f = Fixture::new(true);
    let (overlay, ledger) = unreplayable_branch(&mut f);

    // Guard the guard: the fixture really must be structurally valid but unreplayable.
    assert!(
        overlay.read(&ledger).is_err(),
        "the fixture must not be replayable, or this proves nothing"
    );
    // Structural validity needs no separate assertion: `from_branch` goes through
    // `checked_entries`, so the successful build below is that evidence. Asserting it here would
    // also need a private method, and reaching for one is how a test starts proving the code's
    // internals instead of its behaviour.

    let manifest = crate::studio::overlay::disposal::StudioOverlayDisposal::from_branch(
        &overlay,
        &ledger,
        StudioOverlayProvenance::Closing,
        [0x5b; 32],
        [0x5c; 32],
        3,
        &StudioDisposalDecision::Discard(confirmation()),
        9,
        1,
    )
    .expect("an unreplayable branch must still produce a complete acknowledgement");
    assert_eq!(manifest.accepted, manifest.removed_ids().len());
    assert!(manifest.accepted > 0);
}
