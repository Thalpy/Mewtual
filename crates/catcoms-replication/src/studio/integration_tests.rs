use super::*;
use crate::{
    epoch_zero_id, Admission, CheckpointSeed, EncryptedDoc, EpochGate, InheritedCheckpoint,
    LocalIntent, Receipt, RecoveryReason, SealedOp, SignedOp,
};
use automerge::transaction::Transactable;
use automerge::{ActorId, AutoCommit, Change, ReadDoc, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use std::collections::BTreeMap;

const CHANNEL: ElementId = [7; 16];
const OBJECT: ElementId = [9; 16];
fn id(n: u128) -> ElementId {
    n.to_be_bytes()
}
fn author(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 32])
}
fn target(frame: bool) -> StudioTarget {
    if frame {
        StudioTarget::Flipnote {
            channel: CHANNEL,
            object: OBJECT,
        }
    } else {
        StudioTarget::Index { channel: CHANNEL }
    }
}
fn envelope(doc: &LogicalDocument, body: Vec<u8>, n: u128) -> DomainOp {
    DomainOp {
        nonce: n.to_be_bytes(),
        doc_type: doc.doc_type,
        logical_key: doc.logical_key.clone(),
        body,
    }
}
fn insert(doc: &LogicalDocument, n: u128, after: Option<u128>, who: &DeviceId) -> DomainOp {
    let body = if doc.doc_type == DocType::StudioIndex {
        IndexOp::PutObject {
            object: id(n),
            kind: StudioKind::Flipnote,
            title: format!("object {n}"),
            created_by: *who,
            ts: 1234,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap()
    } else {
        FlipnoteOp::InsertFrame {
            frame: id(n),
            after: after.map(id),
            cid: [n as u8; 32],
            bytes: 100,
        }
        .encode()
        .unwrap()
    };
    envelope(doc, body, n)
}
fn title(doc: &LogicalDocument, object: u128, value: &str, nonce: u128) -> DomainOp {
    let body = if doc.doc_type == DocType::StudioIndex {
        IndexOp::SetTitle {
            object: id(object),
            title: value.into(),
        }
        .encode()
        .unwrap()
    } else {
        FlipnoteOp::SetHeader(FlipnoteHeader::Title(value.into()))
            .encode()
            .unwrap()
    };
    envelope(doc, body, nonce)
}
fn remove(doc: &LogicalDocument, object: u128, nonce: u128) -> DomainOp {
    let body = if doc.doc_type == DocType::StudioIndex {
        IndexOp::TombstoneObject { object: id(object) }
            .encode()
            .unwrap()
    } else {
        FlipnoteOp::RemoveFrame { frame: id(object) }
            .encode()
            .unwrap()
    };
    envelope(doc, body, nonce)
}
fn prepared(
    t: StudioTarget,
    before: &AutoCommit,
    logical: &LogicalDocument,
    epoch: u64,
    domain: &DomainOp,
    who: &DeviceId,
) -> admission::PreparedEdit {
    match t.read(logical, epoch, before).unwrap() {
        StudioProjection::Index(_) => index::prepare(logical, epoch, domain, who).unwrap(),
        StudioProjection::Flipnote(p) => frames::prepare(&p, domain, who, 1234).unwrap(),
    }
}
// Deliberately bypass local policy for well-formed concurrent/malicious ingress fixtures. The
// receiver still runs the actual signature, semantic and exact preflight admission path.
fn raw(
    t: StudioTarget,
    before: &AutoCommit,
    logical: &LogicalDocument,
    epoch: u64,
    domain: &DomainOp,
    who: &DeviceId,
) -> Change {
    let prepared = prepared(t, before, logical, epoch, domain, who);
    let mut next = before
        .clone()
        .with_actor(ActorId::from(who.as_bytes().to_vec()));
    prepared.write(&mut next).unwrap();
    next.put(ROOT, format!("_p1/op/{}", hex(&domain.id(who))), 1u64)
        .unwrap();
    next.commit();
    next.get_last_local_change().unwrap()
}
fn apply(
    t: StudioTarget,
    doc: &mut AutoCommit,
    logical: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
) {
    let change = raw(t, doc, logical, 0, domain, who);
    t.validate(logical, 0, domain, &change, doc).unwrap();
    doc.apply_changes([change]).unwrap();
}
fn sign(
    f: &mut Fixture,
    doc: &EncryptedDoc,
    domain: &DomainOp,
    change: Change,
    author: &MlsDevice,
) -> SealedOp {
    let signed = SignedOp::sign_domain(
        author,
        doc.doc_type(),
        doc.doc_id(),
        change.raw_bytes().to_vec(),
        domain,
    )
    .unwrap();
    SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap()
}
struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        Self {
            owner,
            group,
            rng: ChaCha20Rng::from_seed([17; 32]),
        }
    }
    fn fresh(&self, t: StudioTarget) -> (LogicalDocument, EncryptedDoc, EpochGate) {
        let logical = t.document(&self.group.group_id()).unwrap();
        let physical = epoch_zero_id(logical.doc_type, &logical.logical_key);
        let doc = EncryptedDoc::new(logical.doc_type, physical, &self.owner.device_id());
        let gate = EpochGate::new(logical.clone(), physical, 0, self.owner.device_id());
        (logical, doc, gate)
    }
    fn edit(
        &mut self,
        t: StudioTarget,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        domain: &DomainOp,
    ) -> Result<SealedOp, ReplError> {
        t.edit(
            doc,
            gate,
            &self.owner,
            &self.group,
            &mut self.rng,
            domain,
            1234,
        )
    }
    fn receipt(&self, seed: &CheckpointSeed, close: [u8; 32]) -> Receipt {
        Receipt::sign(
            seed.origin().document().clone(),
            seed.origin().epoch() - 1,
            close,
            seed.change_hash(),
            0,
            InheritedCheckpoint::EpochZero,
            &self.owner,
        )
        .unwrap()
    }
    fn open(
        &self,
        t: StudioTarget,
        seed: &CheckpointSeed,
        close: [u8; 32],
    ) -> (EncryptedDoc, EpochGate) {
        let verified = self
            .receipt(seed, close)
            .verify_current_owner(&self.group, 0)
            .unwrap();
        let seed = match t {
            StudioTarget::Index { .. } => {
                StudioIndexProjection::verify_checkpoint(&verified, seed.bytes())
            }
            StudioTarget::Flipnote { channel, .. } => {
                FlipnoteFrameProjection::verify_checkpoint(&verified, channel, seed.bytes())
            }
        }
        .unwrap();
        let doc = EncryptedDoc::from_checkpoint(&seed, &self.owner.device_id()).unwrap();
        let gate = EpochGate::new(
            seed.origin().document().clone(),
            seed.origin().doc_id(),
            seed.origin().epoch(),
            self.owner.device_id(),
        );
        (doc, gate)
    }
}

#[test]
fn studio_gated_edit_checkpoint_reopen_and_post_seed_concurrency_use_existing_p1() {
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let peer = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, peer.key_package().unwrap())
            .unwrap();
        let (logical, mut doc, gate) = f.fresh(t);
        for n in 1..=3 {
            let op = insert(
                &logical,
                n,
                n.checked_sub(1).filter(|n| *n > 0),
                &f.owner.device_id(),
            );
            f.edit(t, &mut doc, &gate, &op).unwrap();
        }
        f.edit(t, &mut doc, &gate, &remove(&logical, 2, 4)).unwrap();
        let before = t.read(&logical, 0, doc.doc()).unwrap();
        let original = doc.snapshot().unwrap();
        let seed = before.checkpoint([7; 32]).unwrap();
        assert_eq!(original, doc.snapshot().unwrap()); // construction does not retire its source
        let (mut left, lg) = f.open(t, &seed, [7; 32]);
        let (mut right, rg) = f.open(t, &seed, [7; 32]);
        assert!(left.signed_log().is_empty());
        assert!(left
            .doc()
            .keys(ROOT)
            .all(|key| key == snapshot::SEED_KEY || !key.contains('/')));
        let read = t.read(&logical, 1, left.doc()).unwrap();
        match (&before, &read) {
            (StudioProjection::Index(a), StudioProjection::Index(b)) => {
                assert_eq!(a.objects, b.objects);
                assert!(b.tombstones.is_empty());
            }
            (StudioProjection::Flipnote(a), StudioProjection::Flipnote(b)) => {
                assert_eq!(a.timeline, b.timeline);
                assert_eq!(b.timeline, [id(1), id(3)]);
                assert_eq!(a.frames[&id(3)].pixels, b.frames[&id(3)].pixels);
                assert!(b
                    .frames
                    .values()
                    .all(|e| e.insertions.iter().all(|v| v.value.checkpoint)));
                assert!(b.tombstones.is_empty());
            }
            _ => panic!("type changed"),
        }
        let mut signed = Vec::new();
        for (who, value) in [(&f.owner, "owner's value"), (&peer, "peer's value")] {
            let op = title(&logical, 1, value, 5);
            let change = raw(t, left.doc(), &logical, 1, &op, &who.device_id());
            assert_eq!(change.deps(), &[automerge::ChangeHash(seed.change_hash())]);
            let decoded = change.decode();
            // Mutable properties do not consume the packed baseline property's predecessor.
            assert!(decoded.operations.iter().all(|op| op.pred.is_empty()));
            let signed_op = SignedOp::sign_domain(
                who,
                logical.doc_type,
                left.doc_id(),
                change.raw_bytes().to_vec(),
                &op,
            )
            .unwrap();
            signed.push(SealedOp::seal(&signed_op, &f.group, &f.owner, &mut f.rng).unwrap());
        }
        for op in &signed {
            t.ingest(&mut left, &lg, op, &f.group, &f.owner).unwrap();
        }
        for op in signed.iter().rev() {
            t.ingest(&mut right, &rg, op, &f.group, &f.owner).unwrap();
        }
        let p = t.read(&logical, 1, left.doc()).unwrap();
        assert_eq!(p, t.read(&logical, 1, right.doc()).unwrap());
        let saved = left.snapshot().unwrap();
        let mut restored = EncryptedDoc::restore_for_actor(&saved, &f.owner.device_id()).unwrap();
        let restored_gate = EpochGate::decode(&lg.encode().unwrap()).unwrap();
        assert_eq!(p, t.read(&logical, 1, restored.doc()).unwrap());
        assert_eq!(
            t.ingest(
                &mut restored,
                &restored_gate,
                &signed[0],
                &f.group,
                &f.owner
            )
            .unwrap(),
            Admission::Duplicate
        );
        let second = p.checkpoint([8; 32]).unwrap();
        let (mut next, ng) = f.open(t, &second, [8; 32]);
        match (p, t.read(&logical, 2, next.doc()).unwrap()) {
            (StudioProjection::Index(a), StudioProjection::Index(b)) => {
                assert_eq!(a.objects, b.objects)
            }
            (StudioProjection::Flipnote(a), StudioProjection::Flipnote(b)) => {
                assert_eq!(a.title, b.title)
            }
            _ => unreachable!(),
        }
        f.edit(t, &mut next, &ng, &title(&logical, 1, "resolved", 6))
            .unwrap();
    }
}

#[test]
fn studio_recovery_keeps_all_conflicts_births_deletion_authors_and_operation_bodies() {
    for is_frame in [false, true] {
        let t = target(is_frame);
        let logical = t.document(b"recovery-fixture").unwrap();
        let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        let first = insert(&logical, 1, None, &author(1));
        apply(t, &mut doc, &logical, &first, &author(1));
        let base = doc.clone();
        let mut operations = BTreeMap::from([(
            first.id(&author(1)),
            LocalIntent {
                author: author(1),
                operation: first,
            },
        )]);
        for n in 2..=7 {
            let op = title(&logical, 1, &format!("version {n}"), 2);
            let change = raw(t, &base, &logical, 0, &op, &author(n));
            t.validate(&logical, 0, &op, &change, &doc).unwrap();
            doc.apply_changes([change]).unwrap();
            operations.insert(
                op.id(&author(n)),
                LocalIntent {
                    author: author(n),
                    operation: op,
                },
            );
        }
        let projection = t.read(&logical, 0, &doc).unwrap();
        let seed = projection.checkpoint([1; 32]).unwrap();
        let mut seeded = AutoCommit::new();
        seeded
            .apply_changes([Change::from_bytes(seed.bytes().to_vec()).unwrap()])
            .unwrap();
        let compact = t.read(&logical, 1, &seeded).unwrap();
        match (&projection, &compact) {
            (StudioProjection::Index(a), StudioProjection::Index(b)) => {
                assert_eq!(a.objects[&id(1)].title.conflicts.len(), 5);
                assert_eq!(b.objects[&id(1)].title.conflicts.len(), 3);
                assert_eq!(
                    a.objects[&id(1)].title.selected,
                    b.objects[&id(1)].title.selected
                );
            }
            (StudioProjection::Flipnote(a), StudioProjection::Flipnote(b)) => {
                assert_eq!(a.title.as_ref().unwrap().conflicts.len(), 5);
                assert_eq!(b.title.as_ref().unwrap().conflicts.len(), 3);
                assert_eq!(
                    a.title.as_ref().unwrap().selected,
                    b.title.as_ref().unwrap().selected
                );
            }
            _ => unreachable!(),
        }
        // Additional same-id births from the empty causal past must survive together, even
        // when multiple authors later delete that stable id. Neither generic array can hold it.
        for n in 8..=9 {
            let op = insert(&logical, 1, None, &author(n));
            let empty = AutoCommit::new().with_actor(ActorId::from(vec![n; 32]));
            let change = raw(t, &empty, &logical, 0, &op, &author(n));
            doc.apply_changes([change]).unwrap();
            operations.insert(
                op.id(&author(n)),
                LocalIntent {
                    author: author(n),
                    operation: op,
                },
            );
        }
        let base = doc.clone();
        for n in 10..=11 {
            let op = remove(&logical, 1, 1);
            let change = raw(t, &base, &logical, 0, &op, &author(n));
            doc.apply_changes([change]).unwrap();
            operations.insert(
                op.id(&author(n)),
                LocalIntent {
                    author: author(n),
                    operation: op,
                },
            );
        }
        let projection = t.read(&logical, 0, &doc).unwrap();
        let snap = StudioRecovery::snapshot(
            &projection,
            None,
            RecoveryReason::Excluded,
            [8; 32],
            &operations,
        )
        .unwrap();
        let snap = crate::RecoverySnapshot::decode(&snap.encode().unwrap()).unwrap();
        let recovered = StudioRecovery::from_snapshot(&snap, &logical, CHANNEL).unwrap();
        assert_eq!(*recovered.projection(), projection);
        assert_eq!(recovered.operations(), &operations);
        assert!(
            snap.elements.is_empty() && snap.conflicts.is_empty() && snap.tombstones.is_empty()
        );
        assert!(!format!("{recovered:?}").contains("version"));
        let mut wrong = logical.clone();
        wrong.server_id = b"other-server".to_vec();
        assert!(StudioRecovery::from_snapshot(&snap, &wrong, CHANNEL).is_err());
        assert!(StudioRecovery::from_snapshot(&snap, &logical, [8; 16]).is_err());
        let mut wrong = snap.clone();
        wrong.applied_ops.pop();
        assert!(StudioRecovery::from_snapshot(&wrong, &logical, CHANNEL).is_err());
        let mut wrong = snap;
        wrong.projection.push(0);
        assert!(StudioRecovery::from_snapshot(&wrong, &logical, CHANNEL).is_err());
    }
}

#[test]
fn studio_repeated_checkpoints_omit_old_operations_without_growing_the_baseline() {
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let (logical, mut doc, mut gate) = f.fresh(t);
        f.edit(
            t,
            &mut doc,
            &gate,
            &insert(&logical, 1, None, &f.owner.device_id()),
        )
        .unwrap();
        let mut lengths = Vec::new();
        for n in 0..40u128 {
            f.edit(
                t,
                &mut doc,
                &gate,
                &title(&logical, 1, "same-size title", 100 + n),
            )
            .unwrap();
            let added = insert(&logical, 1000 + n, Some(1), &f.owner.device_id());
            f.edit(t, &mut doc, &gate, &added).unwrap();
            f.edit(t, &mut doc, &gate, &remove(&logical, 1000 + n, 2000 + n))
                .unwrap();
            let projection = t.read(&logical, gate.epoch(), doc.doc()).unwrap();
            let seed = projection.checkpoint([7; 32]).unwrap();
            lengths.push(seed.bytes().len());
            (doc, gate) = f.open(t, &seed, [7; 32]);
            assert!(doc.signed_log().is_empty());
            assert_eq!(
                doc.doc()
                    .keys(ROOT)
                    .filter(|k| k.contains('/'))
                    .collect::<Vec<_>>(),
                [snapshot::SEED_KEY]
            );
        }
        assert!(
            lengths.iter().max().unwrap() - lengths.iter().min().unwrap() < 16,
            "{lengths:?}"
        );
    }
}

#[test]
fn studio_exact_checkpoint_preflight_refuses_local_and_signed_remote_growth_atomically() {
    let mut f = Fixture::new();
    let t = target(false);
    let (logical, mut receiver, gate) = f.fresh(t);
    let signer = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.owner, signer.key_package().unwrap())
        .unwrap();
    let long = "x".repeat(60_000);
    let mut rejected = false;
    for n in 1..=40 {
        let body = IndexOp::PutObject {
            object: id(n),
            kind: StudioKind::Flipnote,
            title: long.clone(),
            created_by: f.owner.device_id(),
            ts: 1,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap();
        let op = envelope(&logical, body, n);
        let before = receiver.snapshot().unwrap();
        let before_gate = gate.encode().unwrap();
        match f.edit(t, &mut receiver, &gate, &op) {
            Ok(_) => {}
            Err(ReplError::EpochBound) => {
                assert_eq!(before, receiver.snapshot().unwrap());
                assert_eq!(before_gate, gate.encode().unwrap());
                // An admitted member cannot bypass exact preflight by sending the change.
                let body = IndexOp::PutObject {
                    object: id(n),
                    kind: StudioKind::Flipnote,
                    title: long.clone(),
                    created_by: signer.device_id(),
                    ts: 1,
                    expiry: StudioExpiry::Never,
                }
                .encode()
                .unwrap();
                let op = envelope(&logical, body, n);
                let change = raw(t, receiver.doc(), &logical, 0, &op, &signer.device_id());
                let sealed = sign(&mut f, &receiver, &op, change, &signer);
                assert!(matches!(
                    t.ingest(&mut receiver, &gate, &sealed, &f.group, &f.owner),
                    Err(ReplError::EpochBound)
                ));
                assert_eq!(before, receiver.snapshot().unwrap());
                assert_eq!(before_gate, gate.encode().unwrap());
                rejected = true;
                break;
            }
            Err(e) => panic!("unexpected preflight error: {e:?}"),
        }
    }
    assert!(rejected);
}

#[test]
fn studio_checkpoint_causal_scope_and_baseline_mutation_attacks_leave_state_unchanged() {
    use automerge::legacy::{Key, OpId as AmOpId};
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let attacker = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, attacker.key_package().unwrap())
            .unwrap();
        let (logical, mut source, gate) = f.fresh(t);
        f.edit(
            t,
            &mut source,
            &gate,
            &insert(&logical, 1, None, &f.owner.device_id()),
        )
        .unwrap();
        let seed = t
            .read(&logical, 0, source.doc())
            .unwrap()
            .checkpoint([7; 32])
            .unwrap();
        let (mut receiver, gate) = f.open(t, &seed, [7; 32]);
        let op = title(&logical, 1, "new current value", 2);
        let good = raw(t, receiver.doc(), &logical, 1, &op, &attacker.device_id());
        let (mut same_epoch, same_gate) = f.open(t, &seed, [7; 32]);
        // The incoming change's complete causal view is the seed, not the receiver's later put.
        f.edit(
            t,
            &mut receiver,
            &gate,
            &title(&logical, 1, "receiver only", 3),
        )
        .unwrap();
        for mode in 0..4 {
            let mut decoded = good.decode();
            match mode {
                0 => decoded.deps.clear(),
                1 => decoded.deps.push(automerge::ChangeHash([255; 32])),
                2 => {
                    let automerge::ObjId::Id(counter, actor, _) = receiver
                        .doc()
                        .get(ROOT, snapshot::SEED_KEY)
                        .unwrap()
                        .unwrap()
                        .1
                    else {
                        panic!("seed scalar")
                    };
                    let record = decoded
                        .operations
                        .iter_mut()
                        .find(|op| matches!(&op.key, Key::Map(k) if !k.starts_with("_p1/")))
                        .unwrap();
                    record.pred = vec![AmOpId(counter, actor)].into();
                }
                _ => {
                    let record = decoded
                        .operations
                        .iter_mut()
                        .find(|op| matches!(&op.key, Key::Map(k) if !k.starts_with("_p1/")))
                        .unwrap();
                    record.key = Key::Map(snapshot::SEED_KEY.into());
                }
            }
            let signed = sign(&mut f, &receiver, &op, Change::from(decoded), &attacker);
            let before = receiver.snapshot().unwrap();
            let before_gate = gate.encode().unwrap();
            assert!(
                t.ingest(&mut receiver, &gate, &signed, &f.group, &f.owner)
                    .is_err(),
                "mode {mode}"
            );
            assert_eq!(before, receiver.snapshot().unwrap());
            assert_eq!(before_gate, gate.encode().unwrap());
        }
        let signed = sign(&mut f, &receiver, &op, good, &attacker);
        t.ingest(&mut receiver, &gate, &signed, &f.group, &f.owner)
            .unwrap();
        t.ingest(&mut same_epoch, &same_gate, &signed, &f.group, &f.owner)
            .unwrap();
        // A retained source id is provenance, not a reusable marker or alternate-body retry.
        let before = receiver.snapshot().unwrap();
        assert!(matches!(
            f.edit(
                t,
                &mut receiver,
                &gate,
                &title(&logical, 1, "nonce collision", 1)
            ),
            Err(ReplError::IntentConflict)
        ));
        assert_eq!(before, receiver.snapshot().unwrap());
        let dup = envelope(
            &logical,
            insert(&logical, 1, None, &f.owner.device_id()).body,
            999,
        );
        assert!(f.edit(t, &mut receiver, &gate, &dup).is_err());
        let unknown = title(&logical, 99, "unknown", 4);
        if !is_frame {
            assert!(f.edit(t, &mut receiver, &gate, &unknown).is_err());
        }
    }
}

#[test]
fn studio_typed_seed_verification_rejects_extra_state_even_under_a_matching_owner_receipt() {
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let (logical, mut doc, gate) = f.fresh(t);
        f.edit(
            t,
            &mut doc,
            &gate,
            &insert(&logical, 1, None, &f.owner.device_id()),
        )
        .unwrap();
        let seed = t
            .read(&logical, 0, doc.doc())
            .unwrap()
            .checkpoint([7; 32])
            .unwrap();
        let (base, _) = f.open(t, &seed, [7; 32]);
        for mode in 0..3 {
            let malformed_seed = CheckpointSeed::build(&logical, 1, [7; 32], |next| {
                for key in base.doc().keys(ROOT) {
                    let (value, _) = base.doc().get(ROOT, &key).unwrap().unwrap();
                    let automerge::Value::Scalar(value) = value else {
                        panic!("flat seed")
                    };
                    if mode == 1 && key == snapshot::SEED_KEY {
                        let automerge::ScalarValue::Bytes(mut bytes) = value.into_owned() else {
                            panic!("seed bytes")
                        };
                        bytes.push(0);
                        next.put(ROOT, key, bytes).unwrap();
                    } else {
                        next.put(ROOT, key, value.into_owned()).unwrap();
                    }
                }
                if mode == 0 {
                    next.put(ROOT, "unknown", true).unwrap();
                }
                if mode == 2 {
                    next.put(ROOT, "epoch", 2u64).unwrap();
                }
                Ok(())
            })
            .unwrap();
            let receipt = f
                .receipt(&malformed_seed, [7; 32])
                .verify_current_owner(&f.group, 0)
                .unwrap();
            let checked = if is_frame {
                FlipnoteFrameProjection::verify_checkpoint(
                    &receipt,
                    CHANNEL,
                    malformed_seed.bytes(),
                )
            } else {
                StudioIndexProjection::verify_checkpoint(&receipt, malformed_seed.bytes())
            };
            assert!(checked.is_err(), "mode {mode}");
        }
        let receipt = f
            .receipt(&seed, [7; 32])
            .verify_current_owner(&f.group, 0)
            .unwrap();
        let mut changed = seed.bytes().to_vec();
        *changed.last_mut().unwrap() ^= 1;
        let check = if is_frame {
            FlipnoteFrameProjection::verify_checkpoint(&receipt, CHANNEL, &changed)
        } else {
            StudioIndexProjection::verify_checkpoint(&receipt, &changed)
        };
        assert!(matches!(check, Err(ReplError::BadSignature)));
    }
}

#[test]
fn studio_recovery_rejects_incoherent_source_and_keeps_rewinds_destination_independent() {
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let (logical, mut doc, gate) = f.fresh(t);
        f.edit(
            t,
            &mut doc,
            &gate,
            &insert(&logical, 1, None, &f.owner.device_id()),
        )
        .unwrap();
        let source = t.read(&logical, 0, doc.doc()).unwrap();
        let operations = recovery::current_operations(&doc).unwrap();
        assert!(matches!(
            StudioRecovery::snapshot(
                &source,
                Some([7; 32]),
                RecoveryReason::Excluded,
                [8; 32],
                &operations
            ),
            Err(ReplError::EpochScope)
        ));
        assert!(matches!(
            StudioRecovery::snapshot(&source, None, RecoveryReason::Rewound, [8; 32], &operations),
            Err(ReplError::EpochScope)
        ));
        let snap =
            StudioRecovery::snapshot(&source, None, RecoveryReason::Rewound, [0; 32], &operations)
                .unwrap();
        assert_eq!(
            StudioRecovery::from_snapshot(&snap, &logical, CHANNEL)
                .unwrap()
                .selecting_receipt(),
            [0; 32]
        );
        let mut wrong = snap.clone();
        wrong.base_close_record_hash = Some([7; 32]);
        assert!(matches!(
            StudioRecovery::from_snapshot(&wrong, &logical, CHANNEL),
            Err(ReplError::EpochScope)
        ));
        // Epoch-zero rewind bytes have no destination input. Retargeting the same frozen source
        // therefore preserves its recovery id/warning lifetime instead of consuming another slot.
        assert_eq!(
            StudioRecovery::snapshot(&source, None, RecoveryReason::Rewound, [0; 32], &operations)
                .unwrap(),
            snap
        );
        let seed = source.checkpoint([7; 32]).unwrap();
        let (next, _) = f.open(t, &seed, [7; 32]);
        let source = t.read(&logical, 1, next.doc()).unwrap();
        assert!(matches!(
            StudioRecovery::snapshot(
                &source,
                None,
                RecoveryReason::Excluded,
                [8; 32],
                &BTreeMap::new()
            ),
            Err(ReplError::EpochScope)
        ));
        let snap = StudioRecovery::snapshot(
            &source,
            Some([7; 32]),
            RecoveryReason::Rewound,
            f.receipt(&seed, [7; 32]).hash(),
            &BTreeMap::new(),
        )
        .unwrap();
        StudioRecovery::from_snapshot(&snap, &logical, CHANNEL).unwrap();
        let mut wrong = snap;
        wrong.base_close_record_hash = None;
        assert!(matches!(
            StudioRecovery::from_snapshot(&wrong, &logical, CHANNEL),
            Err(ReplError::EpochScope)
        ));
    }
}

#[test]
fn studio_public_projection_bounds_are_checked_before_compaction_and_recovery_clones() {
    for is_frame in [false, true] {
        let t = target(is_frame);
        let logical = t.document(b"public-bounds").unwrap();
        let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        apply(
            t,
            &mut doc,
            &logical,
            &insert(&logical, 1, None, &author(1)),
            &author(1),
        );
        apply(
            t,
            &mut doc,
            &logical,
            &title(&logical, 1, "small", 2),
            &author(1),
        );
        let original = t.read(&logical, 0, &doc).unwrap();
        for oversized_count in [false, true] {
            let mut p = original.clone();
            match &mut p {
                StudioProjection::Index(p) => {
                    let r = &mut p.objects.get_mut(&id(1)).unwrap().title;
                    if oversized_count {
                        r.conflicts = vec![r.selected.clone(); snapshot::MAX_VALUES + 1];
                    } else {
                        r.selected.value = "x".repeat(MAX_DOMAIN_OP_BYTES + 1);
                    }
                }
                StudioProjection::Flipnote(p) => {
                    let r = p.title.as_mut().unwrap();
                    if oversized_count {
                        r.conflicts = vec![r.selected.clone(); snapshot::MAX_VALUES + 1];
                    } else {
                        r.selected.value = "x".repeat(MAX_DOMAIN_OP_BYTES + 1);
                    }
                }
            }
            assert!(matches!(p.checkpoint([7; 32]), Err(ReplError::EpochBound)));
            assert!(matches!(
                StudioRecovery::snapshot(
                    &p,
                    None,
                    RecoveryReason::Rewound,
                    [0; 32],
                    &BTreeMap::new()
                ),
                Err(ReplError::EpochBound)
            ));
        }
    }
}

#[test]
fn studio_typed_seed_golden_vectors_pin_raw_automerge_and_payload_fields() {
    for is_frame in [false, true] {
        let t = target(is_frame);
        let logical = t.document(b"studio-seed-golden-v1").unwrap();
        let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        apply(
            t,
            &mut doc,
            &logical,
            &insert(&logical, 1, None, &author(1)),
            &author(1),
        );
        apply(
            t,
            &mut doc,
            &logical,
            &title(&logical, 1, "moon cat", 2),
            &author(1),
        );
        let seed = t
            .read(&logical, 0, &doc)
            .unwrap()
            .checkpoint([7; 32])
            .unwrap();
        // Raw change hash covers every Automerge field and every typed payload byte, not just
        // decoded projection equality. Intentional format changes must update these explicitly.
        let (length, hash) = if is_frame {
            (
                622,
                "0b51fdad8ecd06a6db8cfea592219043fd6a1122421e8a6fa12236ae6d10a3eb",
            )
        } else {
            (
                520,
                "bb4ba1ed8ed1af70536d4f8e7624b9b0c854c1d1f1da957079d39ef134d48854",
            )
        };
        assert_eq!(seed.bytes().len(), length);
        assert_eq!(hex(&seed.change_hash()), hash);
    }
}

#[test]
fn studio_recovery_preflight_counts_superseded_operation_bodies_in_outer_budget() {
    let t = target(true);
    let logical = t.document(b"recovery-aggregate").unwrap();
    let mut doc = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
    apply(
        t,
        &mut doc,
        &logical,
        &title(&logical, 1, "visible", 1),
        &author(1),
    );
    let projection = t.read(&logical, 0, &doc).unwrap();
    projection.checkpoint([7; 32]).unwrap();
    let mut operations = BTreeMap::new();
    for nonce in 2..112 {
        let op = title(&logical, 1, &"x".repeat(60_000), nonce);
        operations.insert(
            op.id(&author(1)),
            LocalIntent {
                author: author(1),
                operation: op,
            },
        );
    }
    // Small visible state is not enough: recovery must retain superseded body evidence too.
    assert!(matches!(
        recovery::preflight(projection, &operations),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn studio_signed_concurrency_retains_overflow_while_local_edits_refuse_more_growth() {
    for is_frame in [false, true] {
        let mut f = Fixture::new();
        let t = target(is_frame);
        let peer = MlsDevice::generate().unwrap();
        f.group
            .add_member(&f.owner, peer.key_package().unwrap())
            .unwrap();
        let logical = t.document(&f.group.group_id()).unwrap();
        let empty = AutoCommit::new().with_actor(ActorId::from(vec![1; 32]));
        let mut source = empty.clone();
        let size = if is_frame { 127 } else { 63 };
        let make = |n: u128, who: &DeviceId| {
            if is_frame {
                envelope(
                    &logical,
                    FlipnoteOp::InsertFrame {
                        frame: id(n),
                        after: None,
                        cid: [n as u8; 32],
                        bytes: MAX_FRAME_BYTES,
                    }
                    .encode()
                    .unwrap(),
                    n,
                )
            } else {
                insert(&logical, n, None, who)
            }
        };
        for n in 1..=size {
            let p = prepared(t, &empty, &logical, 0, &make(n, &author(1)), &author(1));
            for (key, value) in p.headers {
                source.put(ROOT, key, value).unwrap();
            }
            source.put(ROOT, p.entry.0, p.entry.1).unwrap();
        }
        source.commit();
        let seed = t
            .read(&logical, 0, &source)
            .unwrap()
            .checkpoint([7; 32])
            .unwrap();
        let (mut doc, gate) = f.open(t, &seed, [7; 32]);
        let peer_op = make(size + 2, &peer.device_id());
        let peer_change = raw(t, doc.doc(), &logical, 1, &peer_op, &peer.device_id());
        let peer_sealed = sign(&mut f, &doc, &peer_op, peer_change, &peer);
        let own = make(size + 1, &f.owner.device_id());
        f.edit(t, &mut doc, &gate, &own).unwrap();
        let refused = make(size + 3, &f.owner.device_id());
        assert!(matches!(
            f.edit(t, &mut doc, &gate, &refused),
            Err(ReplError::EpochBound)
        ));
        t.ingest(&mut doc, &gate, &peer_sealed, &f.group, &f.owner)
            .unwrap();
        let p = t.read(&logical, 1, doc.doc()).unwrap();
        match &p {
            StudioProjection::Index(p) => {
                assert_eq!(p.objects.len(), 64);
                assert_eq!(p.overflow.len(), 1);
            }
            StudioProjection::Flipnote(p) => {
                assert_eq!(p.timeline.len(), 129);
                assert_eq!(p.over_cap.len(), 1);
                assert_eq!(
                    p.declared_frame_bytes,
                    FLIPNOTE_FRAME_BYTES + MAX_FRAME_BYTES
                );
            }
        }
        let snap = StudioRecovery::snapshot(
            &p,
            Some([7; 32]),
            RecoveryReason::Rewound,
            f.receipt(&seed, [7; 32]).hash(),
            &recovery::current_operations(&doc).unwrap(),
        )
        .unwrap();
        assert_eq!(
            *StudioRecovery::from_snapshot(&snap, &logical, CHANNEL)
                .unwrap()
                .projection(),
            p
        );
        // Removing an overflow entry remains possible: the cap is not an unrepairable lock.
        f.edit(t, &mut doc, &gate, &remove(&logical, size + 2, 9999))
            .unwrap();
    }
}
