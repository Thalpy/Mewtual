use super::*;
use automerge::transaction::{CommitOptions, Transactable};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

struct Fixture {
    owner: MlsDevice,
    rejoining_owner: Option<MlsDevice>,
    group: ServerGroup,
    source: StudioEpoch,
    rng: ChaCha20Rng,
    nonce: u128,
}
impl Fixture {
    fn new(art: bool) -> Self {
        let owner = MlsDevice::generate().unwrap();
        // Duplicate before creating the group, so re-admission uses the same identity without
        // restoring an already-present MLS group into that device's provider.
        let rejoining_owner = Some(owner.duplicate().unwrap());
        let group = ServerGroup::create(&owner).unwrap();
        let target = if art {
            StudioTarget::Flipnote {
                channel: [7; 16],
                object: [9; 16],
            }
        } else {
            StudioTarget::Index { channel: [7; 16] }
        };
        let source = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        let mut f = Self {
            owner,
            rejoining_owner,
            group,
            source,
            rng: ChaCha20Rng::from_seed([87; 32]),
            nonce: 0,
        };
        let body = if art {
            FlipnoteOp::InsertFrame {
                frame: [1; 16],
                after: None,
                cid: [3; 32],
                bytes: 100,
            }
            .encode()
            .unwrap()
        } else {
            IndexOp::PutObject {
                object: [1; 16],
                kind: StudioKind::Flipnote,
                title: "private title".into(),
                created_by: f.owner.device_id(),
                ts: 100,
                expiry: StudioExpiry::Never,
            }
            .encode()
            .unwrap()
        };
        f.edit(body);
        f
    }
    fn domain(&mut self, body: Vec<u8>) -> DomainOp {
        self.nonce += 1;
        DomainOp {
            nonce: self.nonce.to_be_bytes(),
            doc_type: self.source.logical.doc_type,
            logical_key: self.source.logical.logical_key.clone(),
            body,
        }
    }
    fn title_body(&self, value: &str) -> Vec<u8> {
        match self.source.target {
            StudioTarget::Index { .. } => IndexOp::SetTitle {
                object: [1; 16],
                title: value.into(),
            }
            .encode()
            .unwrap(),
            _ => FlipnoteOp::SetHeader(FlipnoteHeader::Title(value.into()))
                .encode()
                .unwrap(),
        }
    }
    fn edit(&mut self, body: Vec<u8>) -> DomainOp {
        let op = self.domain(body);
        self.source
            .edit_or_reseal(&self.owner, &self.group, &mut self.rng, &op, 100)
            .unwrap();
        op
    }
    fn fill(&mut self) {
        // Real signed envelopes reach P1's existing 2-MiB lower bound. No quota override or
        // forged gate metadata: every message-bearing change passes the real Studio ingest.
        let mut writer = self.source.doc.doc().clone();
        writer.set_actor(ActorId::from(self.owner.device_id().as_bytes().to_vec()));
        for n in 0..10 {
            let op = self.domain(self.title_body(&format!("title {n}")));
            let edit = match self
                .source
                .target
                .read(&self.source.logical, self.source.epoch(), &writer)
                .unwrap()
            {
                StudioProjection::Index(_) => index::prepare(
                    &self.source.logical,
                    self.source.epoch(),
                    &op,
                    &self.owner.device_id(),
                )
                .unwrap(),
                StudioProjection::Flipnote(p) => {
                    frames::prepare(&p, &op, &self.owner.device_id(), 100).unwrap()
                }
            };
            edit.write(&mut writer).unwrap();
            writer
                .put(
                    ROOT,
                    format!(
                        "_p1/op/{}",
                        crate::registry::hex(&op.id(&self.owner.device_id()))
                    ),
                    1u64,
                )
                .unwrap();
            writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
            let signed = SignedOp::sign_domain(
                &self.owner,
                self.source.logical.doc_type,
                self.source.doc_id(),
                writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
                &op,
            )
            .unwrap();
            let sealed = SealedOp::seal(&signed, &self.group, &self.owner, &mut self.rng).unwrap();
            assert_eq!(
                self.source
                    .ingest(&sealed, &self.group, &self.owner)
                    .unwrap(),
                Admission::Accepted
            );
        }
    }
    fn decide(&mut self, previous: Option<&Receipt>) -> StudioOwnerDecision {
        self.source
            .new_owner_decision(&self.group, &self.owner, 0, previous)
            .unwrap()
    }
    fn plan(&mut self, decision: &StudioOwnerDecision) -> StudioSettlementPlan {
        self.source
            .seal(decision.receipt().clone(), &self.group, 0)
            .unwrap();
        self.source
            .prepare_settlement(decision.close(), &self.group, 0)
            .unwrap()
    }
    fn restart(&mut self) {
        self.source = StudioEpoch::restore(
            &self.source.snapshot().unwrap(),
            &self.group,
            self.source.target,
            self.owner.device_id(),
        )
        .unwrap();
    }
}

#[test]
fn studio_owner_settlement_resumes_exact_close_after_later_edit_and_restart() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        assert!(matches!(
            f.source.new_owner_decision(&f.group, &f.owner, 0, None),
            Err(ReplError::EpochBound)
        ));
        f.fill();
        let decision = f.decide(None);
        let late = f.edit(f.title_body("private late edit"));
        f.restart();
        let resumed = f
            .source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
            .unwrap();
        assert_eq!(resumed.close().encode(), decision.close().encode());
        assert_eq!(resumed.receipt().encode(), decision.receipt().encode());
        let plan = f.plan(&decision);
        assert_eq!(plan.included_operations().len(), 11);
        assert_eq!(plan.excluded_operations().len(), 1);
        assert_eq!(
            plan.excluded_operations()[&late.id(&f.owner.device_id())].operation,
            late
        );
        let recovery = StudioRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            [7; 16],
        )
        .unwrap();
        assert_eq!(recovery.projection(), plan.source_projection());
        assert_eq!(recovery.operations(), plan.excluded_operations());
        let before = f.source.snapshot().unwrap();
        let next = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        assert_eq!(next.op_count(), 0);
        assert_eq!(next.epoch(), 1);
        assert_ne!(next.projection().unwrap(), *plan.source_projection());
        assert_eq!(
            f.source.snapshot().unwrap(),
            before,
            "construction is not replacement"
        );
        assert!(!format!("{plan:?} {decision:?}").contains("private"));
        let included = plan.included_operations().values().next().unwrap();
        let mut changed = included.operation.clone();
        changed.body = f.title_body("different body, same nonce");
        assert_eq!(
            changed.id(&included.author),
            included.operation.id(&included.author)
        );
        assert_ne!(
            changed, included.operation,
            "retirement must compare the complete envelope"
        );
    }
}

#[test]
fn studio_owner_settlement_repeated_rotations_preserve_baseline_and_installed_edits() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let mut previous = None;
        for epoch in 0..2 {
            f.fill();
            let decision = f.decide(previous.as_ref());
            assert_eq!(decision.receipt().closed_epoch, epoch);
            assert_eq!(decision.receipt().inherited, InheritedCheckpoint::EpochZero);
            let plan = f.plan(&decision);
            // Index's ordinary selected-only fields compact losslessly. Art's first checkpoint
            // normalizes original gaps, which remain in recovery even with no excluded op.
            if epoch == 0 {
                assert_eq!(plan.recovery_snapshot().is_some(), art);
            }
            assert!(plan.excluded_operations().is_empty());
            f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
            f.edit(f.title_body("after checkpoint"));
            f.restart();
            let before = f.source.snapshot().unwrap();
            f.source
                .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
                .unwrap();
            assert_eq!(
                f.source.snapshot().unwrap(),
                before,
                "retry cannot reseed newer edits"
            );
            assert!(f
                .source
                .new_owner_decision(&f.group, &f.owner, 0, None)
                .is_err());
            previous = Some(decision.receipt().clone());
        }
    }
}

#[test]
fn studio_owner_settlement_rejects_bad_authority_close_seed_fault_and_stale_plan() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.fill();
        let other = MlsDevice::generate().unwrap();
        assert!(f
            .source
            .new_owner_decision(&f.group, &other, 0, None)
            .is_err());
        assert!(f
            .source
            .new_owner_decision(&f.group, &f.owner, 1, None)
            .is_err());
        let decision = f.decide(None);
        let mut bad_close = decision.close().clone();
        bad_close.signature[0] ^= 1;
        assert!(f
            .source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), &bad_close)
            .is_err());
        bad_close = decision.close().clone();
        bad_close.heads = vec![[0; 32]; 65];
        assert!(f
            .source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), &bad_close)
            .is_err());
        let wrong_seed = Receipt::sign(
            f.source.logical.clone(),
            0,
            decision.close().hash(),
            [9; 32],
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        assert!(f
            .source
            .resume_owner_decision(&f.group, &f.owner, 0, &wrong_seed, decision.close())
            .is_err());
        let mut outside = StudioEpoch::restore(
            &f.source.snapshot().unwrap(),
            &f.group,
            f.source.target,
            f.owner.device_id(),
        )
        .unwrap();
        let late = f.domain(f.title_body("unseen before seal"));
        let sealed = outside
            .edit_or_reseal(&f.owner, &f.group, &mut f.rng, &late, 100)
            .unwrap();
        let plan = f.plan(&decision);
        let held_recovery = plan.recovery_snapshot().cloned();
        // A late packet is quarantined, not accepted. It still changes the exact source stamp.
        assert_eq!(
            f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
            Admission::Quarantined
        );
        assert!(!plan.matches_source(&mut f.source).unwrap());
        assert!(f.source.checkpoint_successor(&plan, &f.group, 0).is_err());
        let refreshed = f
            .source
            .prepare_settlement(decision.close(), &f.group, 0)
            .unwrap();
        assert_eq!(
            refreshed.recovery_snapshot().cloned(),
            held_recovery,
            "quarantine cannot reset recovery identity"
        );
        f.source.seal(wrong_seed, &f.group, 0).unwrap();
        assert_eq!(f.source.phase(), EpochPhase::Fault);
        assert!(f
            .source
            .prepare_settlement(decision.close(), &f.group, 0)
            .is_err());
        assert!(f
            .source
            .checkpoint_successor(&refreshed, &f.group, 0)
            .is_err());
        assert!(f
            .source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
            .is_err());
    }
}

#[test]
fn studio_owner_settlement_keeps_fully_included_deletions_and_original_frame_gaps() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.fill();
        let body = if art {
            FlipnoteOp::RemoveFrame { frame: [1; 16] }.encode().unwrap()
        } else {
            IndexOp::TombstoneObject { object: [1; 16] }
                .encode()
                .unwrap()
        };
        f.edit(body);
        let decision = f.decide(None);
        let plan = f.plan(&decision);
        assert!(plan.excluded_operations().is_empty());
        let recovery = StudioRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            [7; 16],
        )
        .unwrap();
        assert_eq!(recovery.projection(), plan.source_projection());
        assert_eq!(recovery.projection(), &f.source.projection().unwrap());
        let next = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        match (recovery.projection(), next.projection().unwrap()) {
            (StudioProjection::Index(p), StudioProjection::Index(n)) => {
                assert_eq!(p.deleted_objects.len(), 1);
                assert!(n.deleted_objects.is_empty() && n.tombstones.is_empty());
            }
            (StudioProjection::Flipnote(p), StudioProjection::Flipnote(n)) => {
                assert_eq!(p.frames.len(), 1);
                assert!(p.tombstones.contains_key(&[1; 16]));
                assert!(n.frames.is_empty() && n.tombstones.is_empty());
                assert!(recovery.blob_cids().unwrap().contains(&[3; 32]));
            }
            _ => panic!("typed projection mismatch"),
        }
    }
}

#[test]
fn studio_owner_settlement_supports_checkpoints_beyond_registry_epoch_limit() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let mut projection = f.source.projection().unwrap();
        match &mut projection {
            StudioProjection::Index(p) => p.epoch = 4095,
            StudioProjection::Flipnote(p) => p.epoch = 4095,
        }
        let seed = projection.checkpoint([4; 32]).unwrap();
        let opening = Receipt::sign(
            f.source.logical.clone(),
            4095,
            [4; 32],
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &f.owner,
        )
        .unwrap();
        f.source = StudioEpoch::from_checkpoint(
            &f.group,
            f.source.target,
            f.owner.device_id(),
            opening.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
        f.fill();
        let decision = f.decide(Some(&opening));
        assert_eq!(decision.receipt().closed_epoch, 4096);
        let plan = f.plan(&decision);
        f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        f.restart();
        assert_eq!(f.source.epoch(), 4097);
        f.source
            .resume_owner_decision(&f.group, &f.owner, 0, decision.receipt(), decision.close())
            .unwrap();
    }
}

#[test]
fn studio_owner_settlement_keeps_fifth_conflicting_value_without_excluded_operations() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        f.fill();
        let mut members = Vec::new();
        for _ in 0..5 {
            let member = MlsDevice::generate().unwrap();
            f.group
                .add_member(&f.owner, member.key_package().unwrap())
                .unwrap();
            members.push(member);
        }
        let base = f.source.doc.doc().clone();
        for (n, member) in members.iter().enumerate() {
            let body = if art {
                FlipnoteOp::ReplaceFrame {
                    frame: [1; 16],
                    cid: [40 + n as u8; 32],
                    bytes: 100,
                }
                .encode()
                .unwrap()
            } else {
                f.title_body(&format!("concurrent {n}"))
            };
            let op = f.domain(body);
            let mut writer = base.clone();
            writer.set_actor(ActorId::from(member.device_id().as_bytes().to_vec()));
            let edit = match f
                .source
                .target
                .read(&f.source.logical, f.source.epoch(), &writer)
                .unwrap()
            {
                StudioProjection::Index(_) => index::prepare(
                    &f.source.logical,
                    f.source.epoch(),
                    &op,
                    &member.device_id(),
                )
                .unwrap(),
                StudioProjection::Flipnote(p) => {
                    frames::prepare(&p, &op, &member.device_id(), 100).unwrap()
                }
            };
            edit.write(&mut writer).unwrap();
            writer
                .put(
                    ROOT,
                    format!(
                        "_p1/op/{}",
                        crate::registry::hex(&op.id(&member.device_id()))
                    ),
                    1u64,
                )
                .unwrap();
            writer.commit();
            let signed = SignedOp::sign_domain(
                member,
                f.source.logical.doc_type,
                f.source.doc_id(),
                writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
                &op,
            )
            .unwrap();
            // A current member may relay another member's signed operation; its inner author is
            // checked independently by the normal typed ingest path.
            let sealed = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
            assert_eq!(
                f.source.ingest(&sealed, &f.group, &f.owner).unwrap(),
                Admission::Accepted
            );
        }
        let decision = f.decide(None);
        let plan = f.plan(&decision);
        assert!(plan.excluded_operations().is_empty());
        let evidence = StudioRecovery::from_snapshot(
            plan.recovery_snapshot().unwrap(),
            &f.source.logical,
            [7; 16],
        )
        .unwrap();
        let next = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
        match (evidence.projection(), next.projection().unwrap()) {
            (StudioProjection::Index(p), StudioProjection::Index(n)) => {
                assert_eq!(p.objects[&[1; 16]].title.conflicts.len(), 4);
                assert_eq!(n.objects[&[1; 16]].title.conflicts.len(), 3);
            }
            (StudioProjection::Flipnote(p), StudioProjection::Flipnote(n)) => {
                assert_eq!(p.frames[&[1; 16]].pixels.conflicts.len(), 4);
                assert_eq!(n.frames[&[1; 16]].pixels.conflicts.len(), 3);
                for n in 40..45 {
                    assert!(evidence.blob_cids().unwrap().contains(&[n; 32]));
                }
            }
            _ => panic!("typed projection mismatch"),
        }
    }
}

#[test]
fn studio_owner_settlement_succession_inherits_installed_seed_not_old_journal() {
    for art in [false, true] {
        let mut f = Fixture::new(art);
        let returning = f.rejoining_owner.take().unwrap();
        f.fill();
        let first = f.decide(None);
        let plan = f.plan(&first);
        f.source = f.source.checkpoint_successor(&plan, &f.group, 0).unwrap();
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
        let tenure = next_group.epoch();
        f.owner = next;
        f.group = next_group;
        f.restart();
        f.fill();
        let second = f
            .source
            .new_owner_decision(&f.group, &f.owner, tenure, None)
            .unwrap();
        assert_eq!(
            second.receipt().inherited,
            InheritedCheckpoint::Checkpoint {
                epoch: 1,
                close_record_hash: first.receipt().close_record_hash,
                seed_change_hash: first.receipt().seed_change_hash,
            }
        );
        f.source
            .seal(second.receipt().clone(), &f.group, tenure)
            .unwrap();
        let plan = f
            .source
            .prepare_settlement(second.close(), &f.group, tenure)
            .unwrap();
        f.source = f
            .source
            .checkpoint_successor(&plan, &f.group, tenure)
            .unwrap();
        let welcome = f
            .group
            .add_member(&f.owner, returning.key_package().unwrap())
            .unwrap()
            .welcome;
        f.group = ServerGroup::join(&returning, &welcome).unwrap();
        f.owner = returning;
        let returned_tenure = f.group.epoch();
        assert!(returned_tenure > tenure);
        assert_eq!(f.group.designated_committer(), Some(f.owner.device_id()));
        f.restart();
        f.fill();
        let third = f
            .source
            .new_owner_decision(&f.group, &f.owner, returned_tenure, Some(first.receipt()))
            .unwrap();
        assert_eq!(
            third.receipt().inherited,
            InheritedCheckpoint::Checkpoint {
                epoch: 2,
                close_record_hash: second.receipt().close_record_hash,
                seed_change_hash: second.receipt().seed_change_hash,
            }
        );
        assert_ne!(third.receipt().tenure_id, first.receipt().tenure_id);
    }
}
