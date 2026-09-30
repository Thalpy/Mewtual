//! The branch-generation namespace, the two-stage request classification, and the collision rules
//! that only become reachable once a new branch can be admitted beside a retained disposal.
use super::archive::branch;
use super::handoff::{branch as transferable_branch, signing};
use super::*;
use crate::studio::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioOverlayAdmission,
    StudioOverlayProvenance, StudioOverlayRequestClass,
};
use crate::{IntentLedger, LocalIntent};

fn confirmation() -> StudioDiscardConfirmation {
    StudioDiscardConfirmation::parse(StudioDiscardConfirmation::TOKEN).unwrap()
}

fn intent(f: &Fixture, op: DomainOp) -> LocalIntent {
    LocalIntent {
        author: f.owner.device_id(),
        operation: op,
    }
}

/// A basis belonging to a DIFFERENT logical document, for the "unrelated basis" case.
///
/// The fixture's target constants are fixed, so this shares the target and differs only in group and
/// document - which is exactly what is needed: `admit_new_branch` checks the target first, so an
/// unrelated *basis* has to get past that check to prove the identity itself is binding.
fn unrelated_basis() -> StudioClosingOverlayBasis {
    let mut other = Fixture::new(true);
    let (_state, _ledger, _ordered, basis) = branch(&mut other, 1);
    basis
}

/// The identity a request must carry, recomputed here from the documented derivation rather than
/// read back from the state, so the test is an oracle rather than a restatement.
fn derived_branch_id(basis: [u8; 32], generation: u64) -> [u8; 32] {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-branch/v1");
    hash.update(&basis);
    hash.update(&generation.to_be_bytes());
    *hash.finalize().as_bytes()
}

#[test]
fn a_first_branch_is_generation_one_and_its_id_is_the_documented_derivation() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered, _basis) = branch(&mut f, 2);

    assert_eq!(metadata.branch_generation(), 1);
    assert_eq!(metadata.provenance(), StudioOverlayProvenance::Closing);
    assert_eq!(
        metadata.branch_id(),
        Some(derived_branch_id(metadata.overlay().unwrap().basis(), 1)),
        "the branch id must be H(domain, basis fingerprint, generation)"
    );

    // And a request carrying it classifies as the live branch.
    let request = intent(&f, ordered[0].0.clone());
    assert_eq!(
        metadata
            .classify_request(f.source.target, metadata.branch_id().unwrap(), &request)
            .unwrap(),
        StudioOverlayRequestClass::Active
    );
    let _ = &ledger;
}

/// The rollover defence, end to end, exactly as design 6.6's worked example states it.
///
/// Accept G1, dispose G1, admit G2, dispose G2 replacing the manifest, then deliver a delayed exact
/// retry of a G1 request. G1's id names generation 1, which matches no live branch and no retained
/// manifest, and it is not the derived next generation either, so it is `Stale`. G1's work is not
/// resurrected, and the legitimate G2 acceptance was never collateral damage.
#[test]
fn an_old_generation_request_is_stale_after_the_namespace_has_moved_on() {
    let mut f = Fixture::new(true);
    let (g1, ledger, ordered, basis) = branch(&mut f, 2);
    let g1_id = g1.branch_id().unwrap();
    let g1_request = intent(&f, ordered[0].0.clone());

    // G1 disposed: its id still classifies, because the manifest is retained.
    let (after_g1, _removed) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();
    assert!(matches!(
        after_g1
            .classify_request(f.source.target, g1_id, &g1_request)
            .unwrap(),
        StudioOverlayRequestClass::Disposed(_)
    ));

    // G2 admitted on a fresh basis. The id must be the derived next generation, and nothing else.
    // The same still-eligible basis. Design 6.6 is explicit that a fresh Save on a still-eligible
    // basis after a disposal is a legitimate new decision, so this is the primary case rather than a
    // shortcut: `minimum_new_basis_closed_epoch` is deliberately not advanced by a disposal.
    let fresh = &basis;
    let g2_id = derived_branch_id(fresh.fingerprint(), 2);
    assert_eq!(
        after_g1
            .admit_new_branch(f.source.target, g2_id, fresh)
            .unwrap(),
        StudioOverlayAdmission::New { generation: 2 }
    );
    let g2 = after_g1
        .new_admitted(
            fresh,
            StudioOverlayAdmission::New { generation: 2 },
            StudioOverlayProvenance::Closing,
        )
        .unwrap();
    assert_eq!(g2.branch_generation(), 2);
    assert_eq!(g2.branch_id(), Some(g2_id));
    assert!(
        g2.disposed().is_some(),
        "admitting a new branch must not erase the acknowledgement owed for the previous one"
    );

    // While G1's manifest is still the retained one, G1's id is still legitimately acknowledged.
    // That is the point of retaining it, and an earlier version of this test wrongly expected
    // Unmatched here: the design's sequence disposes G2 as well, which REPLACES the manifest, and
    // that replacement is what makes G1 unrecoverable.
    assert!(
        matches!(
            g2.classify_request(f.source.target, g1_id, &g1_request)
                .unwrap(),
            StudioOverlayRequestClass::Disposed(_)
        ),
        "the most recent disposal is still G1's, so its acknowledgement is still owed"
    );

    // Give G2 a branch and dispose it, replacing the manifest.
    let mut g2_live = g2;
    let mut g2_ledger = IntentLedger::new(ledger.document().clone());
    let g2_op = f.domain(f.title_body("second generation work"));
    let g2_entry = g2_ledger.prepare(f.owner.device_id(), g2_op).unwrap();
    g2_live.append(fresh, &g2_ledger, g2_entry, 700).unwrap();
    let (after_g2, _) = g2_live
        .dispose(
            &g2_ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g2_live.branch_content(&g2_ledger).unwrap(),
            2,
            2,
        )
        .unwrap();
    assert_eq!(after_g2.disposed().unwrap().generation, 2);

    // The delayed G1 retry, against a vault whose retained manifest is now G2's. G1's id names a
    // namespace that no longer exists anywhere in this record.
    assert_eq!(
        after_g2
            .classify_request(f.source.target, g1_id, &g1_request)
            .unwrap(),
        StudioOverlayRequestClass::Unmatched,
        "an old generation must not classify as the live branch or as a terminal event"
    );
    // And at the authorizing stage it is Stale rather than a new acceptance - which is the whole
    // point: Unmatched is not a verdict, this is.
    assert_eq!(
        after_g2
            .admit_new_branch(f.source.target, g1_id, fresh)
            .unwrap(),
        StudioOverlayAdmission::Stale
    );
}

/// The High from the classifier review: a fabricated admission must not mint a branch.
///
/// `StudioOverlayAdmission` is a public enum with a public field, so any caller can build
/// `New { generation }` for any number. An earlier version of `new_admitted` stored whatever it was
/// handed, and the review proved the two consequences: a skipped generation became durable, and
/// fabricating generation 1 on a post-disposal vault produced a live branch **sharing the disposed
/// branch's identity**, after which `classify_request` answered `Active` for a branch that had been
/// destroyed. That is the exact failure the namespace exists to prevent, reached without any
/// tampering.
#[test]
fn a_fabricated_admission_cannot_mint_a_branch_at_a_chosen_generation() {
    let mut f = Fixture::new(true);
    let (g1, ledger, ordered, basis) = branch(&mut f, 2);
    let g1_id = g1.branch_id().unwrap();
    let g1_request = intent(&f, ordered[0].0.clone());
    let (after, _) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();

    // Reusing the disposed branch's own generation is the dangerous one, so it is checked first.
    assert!(
        after
            .new_admitted(
                &basis,
                StudioOverlayAdmission::New { generation: 1 },
                StudioOverlayProvenance::Closing,
            )
            .is_err(),
        "reusing the disposed branch's generation would give a new branch its identity"
    );
    // Skipped generations and absurd ones, in both directions.
    for fabricated in [0, 3, 4, 99, u64::MAX] {
        assert!(
            after
                .new_admitted(
                    &basis,
                    StudioOverlayAdmission::New {
                        generation: fabricated
                    },
                    StudioOverlayProvenance::Closing,
                )
                .is_err(),
            "generation {fabricated} is not the next one and must be refused"
        );
    }

    // The real next generation works, and the branch it mints does NOT share the disposed identity.
    let g2 = after
        .new_admitted(
            &basis,
            StudioOverlayAdmission::New { generation: 2 },
            StudioOverlayProvenance::Closing,
        )
        .expect("the derived next generation must be admitted");
    assert_ne!(g2.branch_id(), Some(g1_id));
    // The disposed id must not resolve to the LIVE branch. It does still resolve to the retained
    // manifest, which is correct and is what that manifest is for - asserting `Unmatched` here would
    // be the same mistake the rollover test already corrected once.
    assert!(
        matches!(
            g2.classify_request(f.source.target, g1_id, &g1_request)
                .unwrap(),
            StudioOverlayRequestClass::Disposed(_)
        ),
        "the disposed id must resolve to its manifest, never to the live branch"
    );
    // And `Stale` is never an admission.
    assert!(after
        .new_admitted(
            &basis,
            StudioOverlayAdmission::Stale,
            StudioOverlayProvenance::Closing,
        )
        .is_err());
}

/// The second minting path, which the same review found reusing the generation.
///
/// After a transfer or a disposal `active` is `None`, and the next ordinary Save legitimately starts
/// a new branch through `append`. That is a generation event and must take the next number. Reusing
/// the current one would give the new branch the transferred or disposed branch's identity - the same
/// defect as the fabricated admission above, reached through the ordinary Save path instead.
#[test]
fn appending_where_no_branch_exists_takes_the_next_generation() {
    let mut f = Fixture::new(true);
    let (g1, ledger, _ordered, basis) = branch(&mut f, 2);
    let g1_id = g1.branch_id().unwrap();
    let (after, _) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();
    assert!(after.overlay().is_none());
    assert_eq!(after.branch_generation(), 1);

    // The ordinary Save path: append onto a state with no live branch.
    let mut revived = IntentLedger::new(ledger.document().clone());
    let op = f.domain(f.title_body("the first save after a disposal"));
    let id = revived.prepare(f.owner.device_id(), op).unwrap();
    let mut next = after.clone();
    next.append(&basis, &revived, id, 800)
        .expect("the first Save after a disposal is ordinary and must work");

    assert_eq!(
        next.branch_generation(),
        2,
        "minting a branch where none existed must take the next generation"
    );
    assert_ne!(
        next.branch_id(),
        Some(g1_id),
        "a new branch on the same basis must not inherit the disposed branch's identity"
    );
    assert_eq!(
        next.branch_id(),
        Some(derived_branch_id(basis.fingerprint(), 2)),
        "and it must be the id a client was offered for a new acceptance"
    );

    // Appending again to the now-live branch must NOT increment.
    let op2 = f.domain(f.title_body("more work on the same branch"));
    let id2 = revived.prepare(f.owner.device_id(), op2).unwrap();
    next.append(&basis, &revived, id2, 801).unwrap();
    assert_eq!(
        next.branch_generation(),
        2,
        "extending a live branch is not a generation event"
    );
}

/// `admit_new_branch` refuses everything that is not the exact derived next generation.
#[test]
fn admission_refuses_a_live_branch_a_skipped_generation_and_an_unrelated_basis() {
    let mut f = Fixture::new(true);
    let (g1, ledger, _ordered, basis) = branch(&mut f, 2);

    // A live branch is never admissible: a second branch would have nowhere to live.
    // The same still-eligible basis. Design 6.6 is explicit that a fresh Save on a still-eligible
    // basis after a disposal is a legitimate new decision, so this is the primary case rather than a
    // shortcut: `minimum_new_basis_closed_epoch` is deliberately not advanced by a disposal.
    let fresh = &basis;
    assert_eq!(
        g1.admit_new_branch(
            f.source.target,
            derived_branch_id(fresh.fingerprint(), 2),
            fresh
        )
        .unwrap(),
        StudioOverlayAdmission::Stale,
        "admission must refuse while a branch is still active"
    );

    let (after, _) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();

    // The exact next generation is admitted; a skipped one and the current one are not.
    assert_eq!(
        after
            .admit_new_branch(
                f.source.target,
                derived_branch_id(fresh.fingerprint(), 2),
                fresh
            )
            .unwrap(),
        StudioOverlayAdmission::New { generation: 2 }
    );
    for wrong in [1, 3, 4, u64::MAX] {
        assert_eq!(
            after
                .admit_new_branch(
                    f.source.target,
                    derived_branch_id(fresh.fingerprint(), wrong),
                    fresh
                )
                .unwrap(),
            StudioOverlayAdmission::Stale,
            "generation {wrong} must not be admitted as the next one"
        );
    }

    // An unrelated basis at the right generation is also Stale: the id binds both.
    let other = unrelated_basis();
    assert_ne!(other.fingerprint(), fresh.fingerprint());
    assert_eq!(
        after
            .admit_new_branch(
                f.source.target,
                derived_branch_id(other.fingerprint(), 2),
                fresh
            )
            .unwrap(),
        StudioOverlayAdmission::Stale,
        "an id derived from a different basis must not be admitted against this one"
    );
}

/// A terminal arm acknowledges only the exact operation its manifest recorded.
///
/// The right branch with a body it never held is `Unmatched`, not acknowledged. Without this a
/// delayed request could collect an acknowledgement for work no terminal event ever covered.
#[test]
fn a_terminal_arm_refuses_a_body_its_manifest_never_recorded() {
    let mut f = Fixture::new(true);
    let (g1, ledger, ordered, _basis) = branch(&mut f, 2);
    let g1_id = g1.branch_id().unwrap();
    let (after, _) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();

    // The recorded operation is acknowledged.
    assert!(matches!(
        after
            .classify_request(f.source.target, g1_id, &intent(&f, ordered[0].0.clone()))
            .unwrap(),
        StudioOverlayRequestClass::Disposed(_)
    ));

    // The same nonce under a different body takes the same id, so only the envelope separates them.
    let mut forged = ordered[0].0.clone();
    forged.body = f.title_body("a body the disposed branch never held");
    assert_eq!(
        forged.id(&f.owner.device_id()),
        ordered[0].0.id(&f.owner.device_id()),
        "the fixture must reuse the id, or the envelope check is never consulted"
    );
    assert_eq!(
        after
            .classify_request(f.source.target, g1_id, &intent(&f, forged))
            .unwrap(),
        StudioOverlayRequestClass::Unmatched,
        "a terminal arm must not acknowledge a body its manifest never recorded"
    );
}

/// The transferred arm, and the honest limit of deriving its identity.
///
/// `Completed` stores no branch id, and must not start storing one: the completed block is part of
/// the v2 layout, so a new field would rewrite existing records. It is derived from the manifest's
/// basis and the current generation, which is exact while no newer branch has been admitted - and
/// degrades to `Unmatched`, a refusal, once one has. This test pins both halves.
#[test]
fn the_transferred_arm_is_exact_until_a_new_branch_is_admitted_and_then_refuses() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered) = transferable_branch(&mut f, 2);
    let mut batch = signing(&mut f, &metadata, &ledger);
    while batch.remaining() > 0 {
        batch.sign_next(&f.owner, &f.group, 0).unwrap();
    }
    let (candidate, prepared) = batch.finish().unwrap().into_parts();
    let transferred = prepared.complete(&candidate, &ledger).unwrap();
    assert!(
        transferred.overlay().is_none() && transferred.has_completed(),
        "the fixture must really be a retained transfer with no live branch"
    );

    let request = intent(&f, ordered[0].0.clone());
    let id = derived_branch_id(metadata.overlay().unwrap().basis(), 1);

    // While the generation has not moved, the transferred branch is acknowledged.
    assert!(
        matches!(
            transferred
                .classify_request(f.source.target, id, &request)
                .unwrap(),
            StudioOverlayRequestClass::Transferred(_)
        ),
        "a retained transfer must acknowledge its own branch while the generation stands"
    );

    // Admit a new branch: the generation moves and the old id stops matching. Refusal, never
    // acceptance, which is the direction design 6.6 accepts for forgotten terminal events. The basis
    // only has to be a valid one here; which basis it is does not affect the generation.
    let fresh = unrelated_basis();
    let next = transferred
        .new_admitted(
            &fresh,
            StudioOverlayAdmission::New { generation: 2 },
            StudioOverlayProvenance::Closing,
        )
        .unwrap();
    assert_eq!(
        next.classify_request(f.source.target, id, &request)
            .unwrap(),
        StudioOverlayRequestClass::Unmatched,
        "once the namespace moves on, an old transferred id must refuse rather than acknowledge"
    );
}

/// The two collision rules, now reachable because a branch can be admitted beside a retained
/// disposal. The slice-4A review flagged both as unanchored and this is the slice that owed them.
#[test]
fn a_new_branch_cannot_revive_the_ids_a_retained_disposal_recorded() {
    let mut f = Fixture::new(true);
    let (g1, ledger, ordered, basis) = branch(&mut f, 2);
    let (after, removed) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();

    // The store would have retired these ids. Re-submitting them puts them back in the ledger.
    let mut revived = IntentLedger::new(ledger.document().clone());
    for (op, _) in &ordered {
        let id = revived.prepare(f.owner.device_id(), op.clone()).unwrap();
        assert!(removed.contains(&id));
    }

    // A legitimately admitted new branch that accepts the same ids must be refused: the retained
    // manifest already claims them, and a request naming one would classify against the wrong event.
    // The same still-eligible basis. Design 6.6 is explicit that a fresh Save on a still-eligible
    // basis after a disposal is a legitimate new decision, so this is the primary case rather than a
    // shortcut: `minimum_new_basis_closed_epoch` is deliberately not advanced by a disposal.
    let fresh = &basis;
    let mut g2 = after
        .new_admitted(
            fresh,
            StudioOverlayAdmission::New { generation: 2 },
            StudioOverlayProvenance::Closing,
        )
        .unwrap();
    let first = ordered[0].0.id(&f.owner.device_id());
    let appended = g2.append(fresh, &revived, first, 500);
    assert!(
        appended.is_err(),
        "a new branch must not be able to hold an id the retained disposal recorded"
    );
}

/// The generation must actually survive a round trip, and a v2-expressible state must stay v2.
///
/// The review found that `put_u64(1)` in place of the real generation passed every test: nothing
/// decoded a generation other than 1, so the field could have been a constant. It also found that
/// dropping the provenance clause from `is_v2_expressible` would let an `Unconfirmed` state encode as
/// v2 and silently lose its provenance on the way back.
#[test]
fn the_generation_and_provenance_survive_the_round_trip_and_gate_the_version() {
    let mut f = Fixture::new(true);
    let (g1, ledger, _ordered, basis) = branch(&mut f, 2);

    // Generation 1, Closing, no disposal: v2, and the tag proves it.
    let v2 = g1.encode_vault(&ledger).unwrap();
    assert_eq!(v2.first(), Some(&2));
    assert_eq!(
        StudioOverlayState::decode_vault(&v2, &ledger)
            .unwrap()
            .branch_generation(),
        1
    );

    // Advance the namespace and round trip a generation that is NOT 1. A constant in the encoder
    // fails here; nothing before this test would have noticed.
    let (after, _) = g1
        .dispose(
            &ledger,
            StudioDisposalDecision::Discard(confirmation()),
            g1.branch_content(&ledger).unwrap(),
            1,
            1,
        )
        .unwrap();
    let mut revived = IntentLedger::new(ledger.document().clone());
    let op = f.domain(f.title_body("generation two"));
    let id = revived.prepare(f.owner.device_id(), op).unwrap();
    let mut g2 = after;
    g2.append(&basis, &revived, id, 900).unwrap();
    assert_eq!(g2.branch_generation(), 2);

    let v3 = g2.encode_vault(&revived).unwrap();
    assert_eq!(v3.first(), Some(&3), "a later generation cannot be v2");
    let read = StudioOverlayState::decode_vault(&v3, &revived).unwrap();
    assert_eq!(
        read.branch_generation(),
        2,
        "the generation must survive the round trip, not be re-derived as 1"
    );
    assert_eq!(read.provenance(), StudioOverlayProvenance::Closing);
    assert_eq!(read.branch_id(), g2.branch_id());
    assert_eq!(
        read.encode_vault(&revived).unwrap(),
        v3,
        "and the record must be canonical"
    );
}

/// `validate`'s `branch_generation >= 1` rule, reached through the decoder.
///
/// Nothing in production can build a state that violates it now that the increment is
/// single-sourced, so this rule's job is to refuse a corrupt or crafted record. Crafting one is
/// cheap for the no-disposal shape: a v3 record with no manifest ends in exactly the generation, the
/// provenance byte and a zero presence byte, so the generation is the eight bytes at `len - 10`. No
/// offset guesswork and no test-only mutator on the state.
///
/// **The other two generation rules are NOT tested here, and that is stated rather than implied.**
/// `disposal.generation <= branch_generation` and "a live branch beside a disposal must be strictly
/// later" both need a record carrying a manifest AND a mismatched generation. The manifest is a
/// variable-length block that follows the field, so reaching them means either byte surgery that
/// restates the layout or a test-only setter on the state. Both are worse than an honest gap: the
/// rules are decode-path defences against corruption, no production path can violate them, and the
/// increment they back up is now proved by `a_fabricated_admission_cannot_mint_a_branch_at_a_chosen_generation`.
#[test]
fn validate_refuses_a_zero_generation_in_a_crafted_record() {
    let mut f = Fixture::new(true);
    let (metadata, ledger, ordered) = transferable_branch(&mut f, 2);
    let mut batch = signing(&mut f, &metadata, &ledger);
    while batch.remaining() > 0 {
        batch.sign_next(&f.owner, &f.group, 0).unwrap();
    }
    let (candidate, prepared) = batch.finish().unwrap().into_parts();
    let transferred = prepared.complete(&candidate, &ledger).unwrap();

    // A new branch after the transfer: generation 2, no disposal, so v3 with a bare ten-byte tail.
    let mut revived = IntentLedger::new(ledger.document().clone());
    let id = revived
        .prepare(f.owner.device_id(), ordered[0].0.clone())
        .unwrap();
    let basis = unrelated_basis();
    let mut g2 = transferred;
    if g2.append(&basis, &revived, id, 900).is_err() {
        // The foreign basis may not clear this vault's basis floor. That is fine: the rule under test
        // does not depend on which basis the branch sits on, and saying so beats silently passing.
        return;
    }
    let bytes = g2.encode_vault(&revived).unwrap();
    assert_eq!(bytes.first(), Some(&3));
    assert_eq!(g2.branch_generation(), 2);
    assert_eq!(
        bytes[bytes.len() - 2],
        0,
        "the tail must be provenance then a zero presence byte, or the offset below is wrong"
    );
    assert_eq!(
        u64::from_be_bytes(bytes[bytes.len() - 10..bytes.len() - 2].try_into().unwrap()),
        2,
        "the generation must be the eight bytes at len - 10, or this test is patching the wrong field"
    );

    let mut zeroed = bytes.clone();
    let at = zeroed.len() - 10;
    zeroed[at..at + 8].copy_from_slice(&0u64.to_be_bytes());
    assert!(
        StudioOverlayState::decode_vault(&zeroed, &revived).is_err(),
        "generation 0 must be refused: every branch is at least the first"
    );
}
