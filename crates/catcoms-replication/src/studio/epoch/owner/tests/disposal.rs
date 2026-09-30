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

    // A pending intent that was NEVER appended to the branch. Without it the ledger contains
    // exactly the branch's ids and nothing else, so "the branch's ids" and "every pending id" are
    // the same set: the review proved that swapping `removed_ids()` for all pending ids passed.
    // This is what makes the assertion below about the branch rather than about the ledger.
    let mut ledger = ledger;
    let foreign = ledger
        .prepare(
            f.owner.device_id(),
            f.domain(f.title_body("never appended")),
        )
        .unwrap();
    assert!(
        !live.contains(&foreign),
        "the foreign intent must not be one of the branch's own"
    );

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
        "the retired set must be exactly the branch's own ids, not every pending id"
    );
    assert!(
        !removed.contains(&foreign),
        "a disposal must not retire an intent the branch never accepted"
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

    // Assembled, not sampled. An earlier version asserted only the leading byte, and the review
    // proved that insufficient: adding a trailing marker to the v2 layout and consuming it on
    // decode left every test green. The whole point of the claim is that the byte *sequence* of a
    // disposal-free vault is unchanged, so the expectation has to be the whole sequence, built here
    // from the documented v2 layout rather than read back from the encoder under test.
    let before = metadata.encode_vault(&ledger).unwrap();
    let mut expected = Encoder::new();
    expected.put_u8(2);
    expected.put_u8(1); // Flipnote target tag
    expected.put_bytes(&metadata.target().channel()).unwrap();
    if let StudioTarget::Flipnote { object, .. } = metadata.target() {
        expected.put_bytes(&object).unwrap();
    }
    expected.put_u64(0); // minimum_new_basis_closed_epoch
    expected.put_u8(1); // active present, no prepared
    expected
        .put_bytes(&metadata.overlay().unwrap().encode_vault(&ledger).unwrap())
        .unwrap();
    expected.put_u8(0); // no completed
    assert_eq!(
        before,
        expected.finish(),
        "a disposal-free vault must encode to exactly the bytes it did before this field existed"
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

/// A retained manifest must not be able to describe a body other than the one the ledger holds
/// under that id.
///
/// `DomainOp::id` hashes the nonce and author and **not the body**, so a second operation with the
/// same nonce and different content takes the same id. The entry's envelope is the only thing that
/// notices, and without that rule a retained acknowledgement could be made to describe work it
/// never recorded.
///
/// This one is reachable entirely through production calls: dispose, then ask the real encoder to
/// accept the result against a ledger that disagrees.
///
/// **The two collision rules beside it are NOT tested here, and deliberately so.** A state holding
/// `disposed` next to a live `active`, or next to a `completed` sharing an id, has exactly two
/// producers: `dispose`, which clears `active`, and the decoder. So today those guards defend the
/// **decode path** against a corrupt or crafted record, and the only way to exercise them is to
/// hand-assemble bytes, which tests the byte layout more than the rule. `admit_new_branch` is what
/// legitimately creates a new branch beside a retained disposal; the rules get their natural test
/// in that slice, and it is recorded as owed there rather than left to be rediscovered.
#[test]
fn a_retained_disposal_cannot_be_made_to_describe_a_different_body() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered, _basis) = branch(&mut f, 2);
    let (disposed, _removed) = dispose(
        &metadata,
        &ledger,
        StudioDisposalDecision::Discard(confirmation()),
    )
    .expect("dispose");

    // Baseline: against its own ledger the manifest encodes, so the refusal below is about the
    // disagreement and not about the manifest.
    assert!(disposed.encode_vault(&ledger).is_ok());

    // The same ids, under different bodies.
    let mut retitled = IntentLedger::new(ledger.document().clone());
    for (op, _) in &ordered {
        let mut other = op.clone();
        other.body = f.title_body("a different body under the same nonce");
        let id = retitled.prepare(f.owner.device_id(), other).unwrap();
        assert!(
            _removed.contains(&id),
            "the fixture must reuse the disposed ids, or the envelope rule is never consulted"
        );
    }

    assert!(
        disposed.encode_vault(&retitled).is_err(),
        "a retained manifest whose entry envelope disagrees with the ledger must be refused"
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
/// Driven through `dispose()` itself, not through the manifest builder. An earlier version of this
/// test used the builder and justified it by claiming a `StudioOverlayState` around a bare overlay
/// would need a test-only constructor. **That was false**, and the review was right to check:
/// `decode_vault_structural` on the overlay's own v1 bytes takes the legacy path and yields exactly
/// such a state, through production code. Going through the builder meant a replay guard added to
/// `dispose` would not have been caught here, and that is the guard the property is about.
#[test]
fn an_unreplayable_branch_is_still_disposable() {
    let mut f = Fixture::new(true);
    let (overlay, ledger) = unreplayable_branch(&mut f);

    // Guard the guard: the fixture really must be structurally valid but unreplayable.
    assert!(
        overlay.read(&ledger).is_err(),
        "the fixture must not be replayable, or this proves nothing"
    );

    // A real state holding the unreplayable branch, via the legacy v1 decode path.
    let state = StudioOverlayState::decode_vault_structural(
        &overlay.encode_vault(&ledger).unwrap(),
        &ledger,
    )
    .expect("a structurally valid branch must decode structurally");

    let content = state.branch_content(&ledger).unwrap();
    let (next, removed) = state
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            [0x5b; 32],
            content,
            3,
            StudioOverlayProvenance::Closing,
            9,
            1,
        )
        .expect("an unreplayable branch must still be disposable");

    assert!(!removed.is_empty());
    assert_eq!(next.disposed().unwrap().accepted, removed.len());
    assert!(next.overlay().is_none());
    // And the result is a real v3 record: a legacy-origin disposal must round trip like any other.
    let bytes = next.encode_vault(&ledger).unwrap();
    assert_eq!(bytes.first(), Some(&3));
    assert_eq!(
        StudioOverlayState::decode_vault_structural(&bytes, &ledger)
            .unwrap()
            .disposed(),
        next.disposed()
    );
}
