use super::*;
use crate::IntentLedger;

fn branch(
    f: &mut Fixture,
    count: usize,
) -> (StudioOverlayState, IntentLedger, Vec<(DomainOp, u64)>) {
    f.fill();
    let decision = f.decide(None);
    let plan = f.plan(&decision);
    let basis = f
        .source
        .prepare_closing_overlay(decision.close(), &f.group, 0)
        .unwrap();
    let mut ledger = IntentLedger::new(f.source.document().clone());
    let mut metadata = StudioOverlayState::new(&basis);
    let mut ordered = Vec::new();
    for n in 0..count {
        let op = f.domain(f.title_body(&format!("retained title {n}")));
        let ts = 123 + n as u64;
        let id = ledger.prepare(f.owner.device_id(), op.clone()).unwrap();
        metadata.append(&basis, &ledger, id, ts).unwrap();
        ordered.push((op, ts));
    }
    f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
    assert_eq!((f.source.epoch(), f.source.op_count()), (1, 0));
    (metadata, ledger, ordered)
}

fn signing(
    f: &mut Fixture,
    metadata: &StudioOverlayState,
    ledger: &IntentLedger,
) -> StudioHandoffSigning {
    let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
    let source = f.source.copy_handoff_source(&f.group).unwrap();
    metadata
        .clone()
        .prepare_handoff_detached(source, ledger.clone(), authority)
        .unwrap()
}

#[test]
fn studio_handoff_preparation_one_turn_and_full_output_match_ordinary_edits() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, ordered) = branch(&mut f, 4);
        let source_before = f.source.snapshot().unwrap();
        let metadata_before = metadata.encode_vault(&ledger).unwrap();
        let ledger_before = ledger.encode().unwrap();
        let expected = metadata
            .overlay()
            .unwrap()
            .read(&ledger)
            .unwrap()
            .projection()
            .clone();
        // Independent production oracle: ordinary typed edits, with the ORIGINAL acceptance
        // order and timestamp, rather than using either new preparation or the batch adapter.
        let mut ordinary = f.source.copy_handoff_source(&f.group).unwrap();
        for (op, ts) in &ordered {
            ordinary
                .edit_or_reseal(&f.owner, &f.group, &mut f.rng, op, *ts)
                .unwrap();
        }
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert_eq!(batch.remaining(), ordered.len());
        for n in 0..ordered.len() {
            assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
            assert_eq!(
                batch.remaining(),
                ordered.len() - n - 1,
                "signing turn consumed more than one operation"
            );
            assert_eq!(
                f.source.snapshot().unwrap(),
                source_before,
                "private signing changed installed source"
            );
        }
        assert!(!batch.sign_next(&f.owner, &f.group, 0).unwrap());
        let (mut candidate, prepared) = batch.finish().unwrap().into_parts();
        assert_eq!(candidate.op_count(), ordered.len());
        assert_eq!(candidate.projection().unwrap(), expected);
        assert_eq!(
            candidate.doc.signed_log(),
            ordinary.doc.signed_log(),
            "prepared signing changed original full signed envelopes"
        );
        assert_eq!(candidate.snapshot().unwrap(), ordinary.snapshot().unwrap());
        assert!(prepared.matches_source_before(&mut f.source).unwrap());
        assert_eq!(
            prepared.evidence(&candidate, &ledger).unwrap(),
            StudioHandoffEvidence::Complete
        );
        assert!(prepared
            .complete(&candidate, &ledger)
            .unwrap()
            .overlay()
            .is_none());
        assert_eq!(metadata.encode_vault(&ledger).unwrap(), metadata_before);
        assert_eq!(ledger.encode().unwrap(), ledger_before);
        assert_eq!(f.source.snapshot().unwrap(), source_before);
        let restored = StudioEpoch::restore(
            &candidate.snapshot().unwrap(),
            &f.group,
            candidate.target,
            f.owner.device_id(),
        )
        .unwrap();
        assert_eq!(restored.projection().unwrap(), expected);
        assert_eq!(restored.doc.signed_log(), ordinary.doc.signed_log());
    }
}

#[test]
fn studio_handoff_preparation_partial_finish_and_changed_source_refuse() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(batch.remaining(), 1);
        assert!(
            matches!(batch.finish(), Err(ReplError::IntentConflict)),
            "partial signed batch escaped finish"
        );
        assert_eq!(f.source.snapshot().unwrap(), before);
        let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
        f.edit(f.title_body("independent successor progress"));
        let current = f.source.snapshot().unwrap();
        let changed = f.source.copy_handoff_source(&f.group).unwrap();
        assert!(matches!(
            metadata
                .clone()
                .prepare_handoff_detached(changed, ledger.clone(), authority),
            Err(ReplError::EpochClosed)
        ));
        assert_eq!(f.source.snapshot().unwrap(), current);
        assert_eq!(
            metadata
                .overlay()
                .unwrap()
                .read(&ledger)
                .unwrap()
                .accepted(),
            2
        );
    }
}

#[test]
fn studio_handoff_preparation_mls_change_rejects_next_signature_with_same_owner() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let mut batch = signing(&mut f, &metadata, &ledger);
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        let epoch = f.group.epoch();
        let member = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, member.key_package().unwrap())
            .unwrap();
        assert!(f.group.epoch() > epoch);
        // The existing owner remains designated; membership/key and observed tenure remain
        // valid. The actual receipt still verifies, so only the captured MLS epoch is stale.
        assert_eq!(f.group.designated_committer(), Some(f.owner.device_id()));
        assert_eq!(
            f.group.member_signature_key(&f.owner.device_id()),
            Some(f.owner.public_key_bytes())
        );
        metadata
            .overlay()
            .unwrap()
            .receipt()
            .verify_current_owner(&f.group, 0)
            .unwrap();
        assert!(
            matches!(
                batch.sign_next(&f.owner, &f.group, 0),
                Err(ReplError::EpochAuthority)
            ),
            "changed MLS epoch authorized another prepared signature"
        );
        assert_eq!(batch.remaining(), 1);
        assert_eq!(f.source.snapshot().unwrap(), before);
        // A fresh capture can still prepare and sign the same retained work under current context.
        let mut fresh = signing(&mut f, &metadata, &ledger);
        assert!(fresh.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(fresh.remaining(), 1);
    }
}

#[test]
fn studio_handoff_preparation_wrong_signer_and_observed_tenure_preserve_pending() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let mut batch = signing(&mut f, &metadata, &ledger);
        let other = MlsDevice::generate().unwrap();
        for (device, tenure) in [(&other, 0), (&f.owner, 1)] {
            assert!(matches!(
                batch.sign_next(device, &f.group, tenure),
                Err(ReplError::EpochAuthority)
            ));
            assert_eq!(batch.remaining(), 2);
        }
        assert!(batch.sign_next(&f.owner, &f.group, 0).unwrap());
        assert_eq!(batch.remaining(), 1);
    }
}

#[test]
fn studio_handoff_preparation_source_owner_must_match_verified_authority() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let (metadata, ledger, _) = branch(&mut f, 2);
        let before = f.source.snapshot().unwrap();
        let authority = metadata.handoff_authority(&f.owner, &f.group, 0).unwrap();
        let wrong_owner = MlsDevice::generate().unwrap().device_id();
        assert_ne!(wrong_owner, f.owner.device_id());
        let wrong = StudioEpoch::prepare_vault_source(
            &before,
            &f.group.group_id(),
            f.source.target,
            f.owner.device_id(),
            wrong_owner,
        )
        .unwrap();
        assert_eq!(wrong.projection().unwrap(), f.source.projection().unwrap());
        assert_eq!(
            (
                wrong.doc_id(),
                wrong.epoch(),
                wrong.op_count(),
                wrong.phase()
            ),
            (f.source.doc_id(), 1, 0, EpochPhase::Open)
        );
        metadata
            .overlay()
            .unwrap()
            .receipt()
            .verify_current_owner(&f.group, 0)
            .unwrap();
        assert!(
            matches!(
                metadata
                    .clone()
                    .prepare_handoff_detached(wrong, ledger.clone(), authority),
                Err(ReplError::EpochAuthority)
            ),
            "captured source owner bypassed verified handoff authority"
        );
        assert_eq!(f.source.snapshot().unwrap(), before);
        assert_eq!(signing(&mut f, &metadata, &ledger).remaining(), 2);
    }
}
