use super::*;
use automerge::transaction::Transactable;
use automerge::{ActorId, ChangeHash, ObjType};
use catcoms_mls::{MlsDevice, ServerGroup};
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

use crate::{epoch_zero_id, Admission, EncryptedDoc, EpochGate, SealedOp, SignedOp};

fn logical() -> LogicalDocument {
    studio_index_document(b"index-causal-tests", [7; 16]).unwrap()
}
fn author(n: u8) -> DeviceId {
    DeviceId::from_bytes([n; 32])
}
fn empty(who: &DeviceId) -> AutoCommit {
    AutoCommit::new().with_actor(ActorId::from(who.as_bytes().to_vec()))
}
fn put(id: u8, who: &DeviceId) -> IndexOp {
    IndexOp::PutObject {
        object: [id; 16],
        kind: StudioKind::Flipnote,
        title: "Moon cat".into(),
        created_by: *who,
        ts: 1234,
        expiry: StudioExpiry::Unrecorded,
    }
}
fn title(id: u8, value: &str) -> IndexOp {
    IndexOp::SetTitle {
        object: [id; 16],
        title: value.into(),
    }
}
fn domain(document: &LogicalDocument, op: &IndexOp, nonce: u128) -> DomainOp {
    DomainOp {
        nonce: nonce.to_be_bytes(),
        doc_type: DocType::StudioIndex,
        logical_key: document.logical_key.clone(),
        body: op.encode().unwrap(),
    }
}

// Deliberately TEST-ONLY writer. A production adapter must persist an intent, preflight the
// exact checkpoint/recovery encoding, and save before publication; this harness proves none of
// those missing integration steps. Keep all construction here rather than export a partial API.
fn write(
    doc: &mut AutoCommit,
    document: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
) -> Result<(), automerge::AutomergeError> {
    for (key, value) in header(document, 0) {
        if doc.get(ROOT, &key)?.is_none() {
            doc.put(ROOT, key, value)?;
        }
    }
    let op = IndexOp::decode(&domain.body).unwrap();
    let (key, bytes) = record(domain, who, &op).unwrap();
    doc.put(ROOT, key, bytes)?;
    Ok(())
}
fn draft(
    before: &AutoCommit,
    document: &LogicalDocument,
    domain: &DomainOp,
    who: &DeviceId,
) -> (AutoCommit, Change) {
    let mut doc = before
        .clone()
        .with_actor(ActorId::from(who.as_bytes().to_vec()));
    write(&mut doc, document, domain, who).unwrap();
    doc.put(ROOT, format!("_p1/op/{}", hex(&domain.id(who))), 1u64)
        .unwrap();
    doc.commit();
    let change = doc.get_last_local_change().unwrap();
    (doc, change)
}
fn accept(doc: &mut AutoCommit, who: u8, op: &IndexOp, nonce: u128) -> (DomainOp, Change) {
    let domain = domain(&logical(), op, nonce);
    let (next, change) = draft(doc, &logical(), &domain, &author(who));
    validate_index_change(&logical(), 0, &domain, &change, doc).unwrap();
    *doc = next;
    (domain, change)
}
fn reject(before: &AutoCommit, who: u8, op: &IndexOp, nonce: u128) {
    let domain = domain(&logical(), op, nonce);
    let (_, change) = draft(before, &logical(), &domain, &author(who));
    assert!(validate_index_change(&logical(), 0, &domain, &change, before).is_err());
}
fn checked_merge(base: &AutoCommit, operations: &[(DomainOp, Change)]) -> AutoCommit {
    let mut doc = base.clone();
    for (domain, change) in operations {
        validate_index_change(&logical(), 0, domain, change, &doc).unwrap();
        doc.apply_changes([change.clone()]).unwrap();
    }
    doc
}

fn forced_causal_retry(before: &AutoCommit, domain: &DomainOp, who: &DeviceId) -> Change {
    // Re-putting identical bytes is an Automerge no-op: get_last_local_change would return the
    // original accepted change, which exact transport dedup intentionally allows. Build a NEW
    // causal change with explicit writes, then restore their canonical values in the raw delta.
    let op = IndexOp::decode(&domain.body).unwrap();
    let (key, bytes) = record(domain, who, &op).unwrap();
    let marker = format!("_p1/op/{}", hex(&domain.id(who)));
    let mut writer = before
        .clone()
        .with_actor(ActorId::from(who.as_bytes().to_vec()));
    writer.put(ROOT, &key, vec![0u8]).unwrap();
    writer.put(ROOT, &marker, 2u64).unwrap();
    writer.commit();
    let mut change = writer.get_last_local_change().unwrap().decode();
    for op in &mut change.operations {
        if matches!(&op.key, Key::Map(k) if k.as_str() == key) {
            op.action = OpType::Put(ScalarValue::Bytes(bytes.clone()));
        } else if matches!(&op.key, Key::Map(k) if k.as_str() == marker) {
            op.action = OpType::Put(ScalarValue::Uint(1));
        }
    }
    Change::from(change)
}

#[test]
fn index_change_every_operation_and_equal_value_new_intents_are_causally_valid() {
    let mut doc = empty(&author(1));
    let created = accept(&mut doc, 1, &put(9, &author(1)), 1);
    assert_eq!(created.1.len(), 6);
    for (nonce, op) in [
        title(9, "New name"),
        title(9, "New name"), // same text, distinct provenance and marker
        IndexOp::SetExpiry {
            object: [9; 16],
            expiry: StudioExpiry::At(0),
        },
        IndexOp::SetExpiry {
            object: [9; 16],
            expiry: StudioExpiry::Never,
        },
        IndexOp::SetExpiry {
            object: [9; 16],
            expiry: StudioExpiry::Unrecorded,
        },
        IndexOp::TombstoneObject { object: [9; 16] },
    ]
    .into_iter()
    .enumerate()
    {
        let (_, change) = accept(&mut doc, 2, &op, nonce as u128 + 2);
        assert_eq!(change.len(), 2);
    }
    let projection = StudioIndexProjection::read(&logical(), 0, &doc).unwrap();
    assert!(projection.objects.is_empty());
    assert_eq!(
        projection.deleted_objects[&[9; 16]].title.selected.value,
        "New name"
    );
    assert_eq!(
        projection.deleted_objects[&[9; 16]].expiry.selected.value,
        StudioExpiry::Unrecorded
    );
}

#[test]
fn index_change_concurrent_creations_renames_and_deletion_converge_in_both_orders() {
    let base = empty(&author(1));
    let mut a = base.clone();
    let mut b = base.clone();
    let ca = accept(&mut a, 1, &put(9, &author(1)), 1);
    let cb = accept(&mut b, 2, &put(9, &author(2)), 1);
    let mut one = checked_merge(&base, &[ca.clone(), cb.clone()]);
    let two = checked_merge(&base, &[cb, ca]);
    assert_eq!(
        StudioIndexProjection::read(&logical(), 0, &one).unwrap(),
        StudioIndexProjection::read(&logical(), 0, &two).unwrap()
    );
    assert_eq!(one.get_all(ROOT, "v").unwrap().len(), 2);
    // The writer must leave equal concurrent headers alone; re-putting them emits cleanup.
    let rename = accept(&mut one, 1, &title(9, "Named"), 2);
    let mut dead = two.clone();
    let deletion = accept(
        &mut dead,
        2,
        &IndexOp::TombstoneObject { object: [9; 16] },
        2,
    );
    let left = checked_merge(&two, &[rename.clone(), deletion.clone()]);
    let right = checked_merge(&two, &[deletion, rename]);
    let left = StudioIndexProjection::read(&logical(), 0, &left).unwrap();
    assert_eq!(
        left,
        StudioIndexProjection::read(&logical(), 0, &right).unwrap()
    );
    assert_eq!(left.deleted_objects[&[9; 16]].title.selected.value, "Named");
    assert_eq!(left.deleted_objects[&[9; 16]].creations.len(), 2);
}

#[test]
fn index_change_target_checks_include_overflow_and_deleted_ids() {
    let mut doc = empty(&author(1));
    for id in 0..65 {
        accept(&mut doc, 1, &put(id, &author(1)), id as u128);
    }
    assert!(StudioIndexProjection::read(&logical(), 0, &doc)
        .unwrap()
        .overflow
        .contains_key(&[64; 16]));
    accept(&mut doc, 2, &title(64, "Overflow remains editable"), 1);
    accept(
        &mut doc,
        2,
        &IndexOp::SetExpiry {
            object: [64; 16],
            expiry: StudioExpiry::Never,
        },
        2,
    );
    reject(&doc, 2, &put(64, &author(2)), 3);
    accept(
        &mut doc,
        2,
        &IndexOp::TombstoneObject { object: [64; 16] },
        3,
    );
    for op in [
        put(64, &author(2)),
        title(64, "No resurrection"),
        IndexOp::SetExpiry {
            object: [64; 16],
            expiry: StudioExpiry::Never,
        },
        IndexOp::TombstoneObject { object: [64; 16] },
        title(99, "Missing"),
        IndexOp::SetExpiry {
            object: [99; 16],
            expiry: StudioExpiry::Never,
        },
        IndexOp::TombstoneObject { object: [99; 16] },
    ] {
        reject(&doc, 2, &op, 4);
    }
}

#[test]
fn index_change_mutable_predecessors_require_all_and_only_the_causal_property_values() {
    let mut base = empty(&author(1));
    accept(&mut base, 1, &put(9, &author(1)), 1);
    let a = accept(&mut base.clone(), 1, &title(9, "A"), 2);
    let b = accept(&mut base.clone(), 2, &title(9, "B"), 1);
    let merged = checked_merge(&base, &[a, b]);
    let domain = domain(&logical(), &title(9, "Resolve"), 3);
    let (_, good) = draft(&merged, &logical(), &domain, &author(1));
    validate_index_change(&logical(), 0, &domain, &good, &merged).unwrap();
    let victim = merged.get(ROOT, "v").unwrap().unwrap().1;
    let ObjId::Id(counter, actor, _) = victim else {
        panic!("header id")
    };
    for mode in 0..4 {
        let mut bad = good.decode();
        let op = bad
            .operations
            .iter_mut()
            .find(|op| matches!(&op.key, Key::Map(k) if k.starts_with("t/")))
            .unwrap();
        assert_eq!(op.pred.len(), 2);
        let first = op.pred.iter().next().unwrap().clone();
        let second = op.pred.iter().nth(1).unwrap().clone();
        op.pred = match mode {
            0 => vec![],
            1 => vec![first.clone()],
            2 => vec![first.clone(), first, second],
            _ => vec![OpId(counter, actor.clone())],
        }
        .into();
        let bad = Change::from(bad);
        if mode == 2 {
            // The whole required set is present; ONLY its duplicate makes this invalid. Pin
            // that raw change encoding preserves the duplicate, not an accidentally partial set.
            let decoded = bad.decode();
            let entry = decoded
                .operations
                .iter()
                .find(|op| matches!(&op.key, Key::Map(k) if k.starts_with("t/")))
                .unwrap();
            assert_eq!(entry.pred.len(), 3);
            assert_eq!(entry.pred.iter().collect::<BTreeSet<_>>().len(), 2);
        }
        assert!(validate_index_change(&logical(), 0, &domain, &bad, &merged).is_err());
    }
}

#[test]
fn index_change_receiver_future_creation_and_predecessor_cannot_be_borrowed() {
    let empty = empty(&author(1));
    let mut base = empty.clone();
    accept(&mut base, 1, &put(9, &author(1)), 1);
    // The receiver has the object; the change's author causally knew only an empty graph.
    let missing = domain(&logical(), &title(9, "Borrowed target"), 1);
    let (_, change) = draft(&empty, &logical(), &missing, &author(2));
    assert!(validate_index_change(&logical(), 0, &missing, &change, &base).is_err());
    accept(&mut base, 1, &title(9, "Old"), 2);
    let old = base.clone();
    let old_domain = domain(&logical(), &title(9, "Own old view"), 3);
    let (_, good) = draft(&old, &logical(), &old_domain, &author(2));
    accept(&mut base, 1, &title(9, "Future"), 3);
    validate_index_change(&logical(), 0, &old_domain, &good, &base).unwrap();
    let ObjId::Id(counter, actor, _) = base
        .get(ROOT, format!("t/{}", hex(&[9; 16])))
        .unwrap()
        .unwrap()
        .1
    else {
        panic!("future id")
    };
    let mut forged = good.decode();
    for op in &mut forged.operations {
        if matches!(&op.key, Key::Map(k) if k.starts_with("t/")) {
            op.pred = vec![OpId(counter, actor.clone())].into();
        }
    }
    assert!(
        validate_index_change(&logical(), 0, &old_domain, &Change::from(forged), &base).is_err()
    );
    let mut unknown = good.decode();
    unknown.deps = vec![ChangeHash([255; 32])];
    assert!(
        validate_index_change(&logical(), 0, &old_domain, &Change::from(unknown), &base).is_err()
    );
}

#[test]
fn index_change_rejects_wrong_records_missing_headers_hidden_actions_and_marker_reuse() {
    let base = empty(&author(1));
    let domain = domain(&logical(), &put(9, &author(1)), 1);
    let (created, good) = draft(&base, &logical(), &domain, &author(1));
    for mode in 0..9 {
        let mut bad = good.decode();
        match mode {
            0 => {
                bad.operations
                    .retain(|op| !matches!(&op.key, Key::Map(k) if k == "kind"));
            }
            1 => {
                bad.operations
                    .retain(|op| !matches!(&op.key, Key::Map(k) if k.starts_with("_p1/")));
            }
            2 => {
                bad.operations
                    .retain(|op| !matches!(&op.key, Key::Map(k) if k.starts_with("i/")));
            }
            _ => {
                let op = bad
                    .operations
                    .iter_mut()
                    .find(|op| matches!(&op.key, Key::Map(k) if k.starts_with("i/")))
                    .unwrap();
                match mode {
                    3 => {
                        op.key = Key::Map("arbitrary".into());
                    }
                    4 => {
                        op.action = OpType::Delete;
                    }
                    5 => {
                        op.action = OpType::Make(ObjType::Map);
                    }
                    6 => {
                        op.insert = true;
                    }
                    7 => {
                        let OpType::Put(ScalarValue::Bytes(bytes)) = &mut op.action else {
                            panic!("record")
                        };
                        bytes[1] ^= 1; // full recorded author differs from change actor
                    }
                    _ => {
                        op.obj = ObjectId::Id(OpId(1, good.actor_id().clone()));
                    }
                }
            }
        }
        assert!(
            validate_index_change(&logical(), 0, &domain, &Change::from(bad), &base).is_err(),
            "mode {mode}"
        );
    }
    // A causal repetition of the same nonce cannot replace an immutable insertion/marker.
    let again = forced_causal_retry(&created, &domain, &author(1));
    assert_ne!(again.hash(), good.hash());
    assert_eq!(again.deps(), &[good.hash()]);
    assert!(validate_index_change(&logical(), 0, &domain, &again, &created).is_err());
    let renamed = self::domain(&logical(), &title(9, "One"), 2);
    let (after, _) = draft(&created, &logical(), &renamed, &author(1));
    let again = forced_causal_retry(&after, &renamed, &author(1));
    assert!(matches!(
        validate_index_change(&logical(), 0, &renamed, &again, &after),
        Err(ReplError::IntentConflict)
    ));
}

#[test]
fn index_change_refuses_bad_scope_seedless_checkpoint_epochs_and_immutable_header_rewrites() {
    let base = empty(&author(1));
    let domain = domain(&logical(), &put(9, &author(1)), 1);
    let (created, good) = draft(&base, &logical(), &domain, &author(1));
    for epoch in [1, 4000, u64::MAX] {
        assert!(matches!(
            validate_index_change(&logical(), epoch, &domain, &good, &base),
            Err(ReplError::EpochScope)
        ));
    }
    for len in [0, 15, 17] {
        let mut wrong = logical();
        wrong.logical_key = vec![1; len];
        assert!(validate_index_change(&wrong, 0, &domain, &good, &base).is_err());
    }
    let mut wrong = logical();
    wrong.doc_type = DocType::StudioObject;
    assert!(validate_index_change(&wrong, 0, &domain, &good, &base).is_err());
    let mut wrong = domain.clone();
    wrong.logical_key = vec![3; 16];
    assert!(validate_index_change(&logical(), 0, &wrong, &good, &base).is_err());
    let mut wrong_actor = good.decode();
    wrong_actor.actor_id = ActorId::from(vec![1; 4]);
    assert!(matches!(
        validate_index_change(&logical(), 0, &domain, &Change::from(wrong_actor), &base),
        Err(ReplError::EpochAuthority)
    ));
    let mut writer = created.clone();
    let new = self::domain(&logical(), &title(9, "New"), 2);
    write(&mut writer, &logical(), &new, &author(1)).unwrap();
    writer.put(ROOT, "kind", "wrong temporarily").unwrap();
    writer
        .put(ROOT, format!("_p1/op/{}", hex(&new.id(&author(1)))), 1u64)
        .unwrap();
    writer.commit();
    let mut bad = writer.get_last_local_change().unwrap().decode();
    for op in &mut bad.operations {
        if matches!(&op.key, Key::Map(k) if k == "kind") {
            op.action = OpType::Put(ScalarValue::Str("index".into()));
        }
    }
    assert!(validate_index_change(&logical(), 0, &new, &Change::from(bad), &created).is_err());
}

// The following fixtures deliberately substitute only the existing reader for checkpoint
// preflight. They isolate the signature/causal/atomic gate boundary, NOT production admission.
// A dedicated refusal case proves that a future exact preflight failure rolls back both paths.
struct Fixture {
    owner: MlsDevice,
    group: ServerGroup,
    logical: LogicalDocument,
    rng: ChaCha20Rng,
}
impl Fixture {
    fn new() -> Self {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let logical = studio_index_document(&group.group_id(), [7; 16]).unwrap();
        Self {
            owner,
            group,
            logical,
            rng: ChaCha20Rng::from_seed([27; 32]),
        }
    }
    fn target(&self) -> (EncryptedDoc, EpochGate) {
        let id = epoch_zero_id(self.logical.doc_type, &self.logical.logical_key);
        (
            EncryptedDoc::new(self.logical.doc_type, id, &self.owner.device_id()),
            EpochGate::new(self.logical.clone(), id, 0, self.owner.device_id()),
        )
    }
    fn edit(
        &mut self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        domain: &DomainOp,
        refuse: bool,
    ) -> Result<SealedOp, ReplError> {
        let before = doc.doc().clone();
        doc.edit_domain_preflight_gated(
            &self.logical,
            gate,
            &self.owner,
            &self.group,
            &mut self.rng,
            domain,
            |next| write(next, &self.logical, domain, &self.owner.device_id()),
            |domain, change| validate_index_change(&self.logical, 0, domain, change, &before),
            |next| {
                if refuse {
                    Err(ReplError::EpochBound)
                } else {
                    StudioIndexProjection::read(&self.logical, 0, next).map(|_| ())
                }
            },
        )
        .map(|(sealed, _)| sealed)
    }
    fn ingest(
        &self,
        doc: &mut EncryptedDoc,
        gate: &EpochGate,
        sealed: &SealedOp,
        refuse: bool,
    ) -> Result<Admission, ReplError> {
        let before = doc.doc().clone();
        doc.ingest_domain_preflight_gated(
            &self.logical,
            gate,
            sealed,
            &self.group,
            &self.owner,
            |domain, change| validate_index_change(&self.logical, 0, domain, change, &before),
            |next| {
                if refuse {
                    Err(ReplError::EpochBound)
                } else {
                    StudioIndexProjection::read(&self.logical, 0, next).map(|_| ())
                }
            },
        )
    }
}

#[test]
fn index_change_signed_gate_restart_exact_retries_and_preflight_rejection_are_atomic() {
    let mut f = Fixture::new();
    let (mut sender, gate) = f.target();
    let (mut receiver, incoming) = f.target();
    let domain = domain(&f.logical, &put(9, &f.owner.device_id()), 1);
    let unchanged = sender.snapshot().unwrap();
    let gate_unchanged = gate.encode().unwrap();
    assert!(matches!(
        f.edit(&mut sender, &gate, &domain, true),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(sender.snapshot().unwrap(), unchanged);
    assert_eq!(gate.encode().unwrap(), gate_unchanged);
    let sealed = f.edit(&mut sender, &gate, &domain, false).unwrap();
    assert!(matches!(
        f.edit(&mut sender, &gate, &domain, false),
        Err(ReplError::NoChange)
    ));
    assert!(matches!(
        f.ingest(&mut receiver, &incoming, &sealed, true),
        Err(ReplError::EpochBound)
    ));
    assert_eq!(receiver.snapshot().unwrap(), unchanged);
    assert_eq!(incoming.encode().unwrap(), gate_unchanged);
    assert_eq!(
        f.ingest(&mut receiver, &incoming, &sealed, false).unwrap(),
        Admission::Accepted
    );
    assert_eq!(
        f.ingest(&mut receiver, &incoming, &sealed, false).unwrap(),
        Admission::Duplicate
    );
    let saved = receiver.snapshot().unwrap();
    receiver = EncryptedDoc::restore_for_actor(&saved, &f.owner.device_id()).unwrap();
    let incoming = EpochGate::decode(&incoming.encode().unwrap()).unwrap();
    assert_eq!(
        f.ingest(&mut receiver, &incoming, &sealed, false).unwrap(),
        Admission::Duplicate
    );
    // New typed edits remain valid on the restored accepted DAG, not only a fresh fixture.
    let rename = self::domain(&f.logical, &title(9, "After restart"), 2);
    f.edit(&mut receiver, &incoming, &rename, false).unwrap();
    assert_eq!(
        StudioIndexProjection::read(&f.logical, 0, receiver.doc())
            .unwrap()
            .objects[&[9; 16]]
            .title
            .selected
            .value,
        "After restart"
    );
}

#[test]
fn index_change_signed_gate_rejects_forged_author_scope_and_mutations_without_state_changes() {
    let mut f = Fixture::new();
    let (mut receiver, gate) = f.target();
    let snapshot = receiver.snapshot().unwrap();
    let gate_snapshot = gate.encode().unwrap();
    let outsider = MlsDevice::generate().unwrap();
    for mode in 0..5 {
        let who = if mode == 0 {
            outsider.device_id()
        } else {
            f.owner.device_id()
        };
        let logical = if mode == 2 {
            studio_index_document(&f.group.group_id(), [8; 16]).unwrap()
        } else {
            f.logical.clone()
        };
        let domain = domain(&logical, &put(9, &who), 1);
        let (_, good) = draft(receiver.doc(), &logical, &domain, &who);
        let mut decoded = good.decode();
        if mode == 1 {
            decoded.actor_id = ActorId::from(outsider.device_id().as_bytes().to_vec());
        }
        if mode == 3 {
            for op in &mut decoded.operations {
                if matches!(&op.key, Key::Map(k) if k.starts_with("i/")) {
                    op.action = OpType::Delete;
                }
            }
        }
        if mode == 4 {
            for op in &mut decoded.operations {
                if let OpType::Put(ScalarValue::Bytes(bytes)) = &mut op.action {
                    bytes[1] ^= 1;
                }
            }
        }
        let signed = SignedOp::sign_domain(
            if mode == 0 { &outsider } else { &f.owner },
            DocType::StudioIndex,
            receiver.doc_id(),
            Change::from(decoded).raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
        assert!(
            f.ingest(&mut receiver, &gate, &sealed, false).is_err(),
            "mode {mode}"
        );
        assert_eq!(receiver.snapshot().unwrap(), snapshot);
        assert_eq!(gate.encode().unwrap(), gate_snapshot);
    }
    // A valid inner peer signature resealed under another server does not authorize that target.
    let second = Fixture::new();
    let domain = domain(&f.logical, &put(9, &f.owner.device_id()), 1);
    let (_, good) = draft(receiver.doc(), &f.logical, &domain, &f.owner.device_id());
    let signed = SignedOp::sign_domain(
        &f.owner,
        DocType::StudioIndex,
        receiver.doc_id(),
        good.raw_bytes().to_vec(),
        &domain,
    )
    .unwrap();
    let foreign = SealedOp::seal(&signed, &second.group, &second.owner, &mut f.rng).unwrap();
    assert!(f.ingest(&mut receiver, &gate, &foreign, false).is_err());
    assert_eq!(receiver.snapshot().unwrap(), snapshot);
    assert_eq!(gate.encode().unwrap(), gate_snapshot);
}

#[test]
fn index_change_signed_members_converge_and_predecessor_attack_preserves_gate_and_log() {
    let mut f = Fixture::new();
    let peer = MlsDevice::generate().unwrap();
    f.group
        .add_member(&f.owner, peer.key_package().unwrap())
        .unwrap();
    let (mut left, lg) = f.target();
    let (mut right, rg) = f.target();
    let mut creations = Vec::new();
    for signer in [&f.owner, &peer] {
        let domain = domain(&f.logical, &put(9, &signer.device_id()), 1);
        let (_, change) = draft(left.doc(), &f.logical, &domain, &signer.device_id());
        let signed = SignedOp::sign_domain(
            signer,
            DocType::StudioIndex,
            left.doc_id(),
            change.raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        creations.push(SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap());
    }
    for sealed in &creations {
        assert_eq!(
            f.ingest(&mut left, &lg, sealed, false).unwrap(),
            Admission::Accepted
        );
    }
    for sealed in creations.iter().rev() {
        assert_eq!(
            f.ingest(&mut right, &rg, sealed, false).unwrap(),
            Admission::Accepted
        );
    }
    assert_eq!(
        StudioIndexProjection::read(&f.logical, 0, left.doc()).unwrap(),
        StudioIndexProjection::read(&f.logical, 0, right.doc()).unwrap()
    );

    let mut updates = Vec::new();
    for (signer, operation) in [(&f.owner, title(9, "A")), (&peer, title(9, "B"))] {
        let domain = domain(&f.logical, &operation, 2);
        let (_, change) = draft(left.doc(), &f.logical, &domain, &signer.device_id());
        let signed = SignedOp::sign_domain(
            signer,
            DocType::StudioIndex,
            left.doc_id(),
            change.raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        updates.push(SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap());
    }
    for sealed in &updates {
        f.ingest(&mut left, &lg, sealed, false).unwrap();
    }
    for sealed in updates.iter().rev() {
        f.ingest(&mut right, &rg, sealed, false).unwrap();
    }
    assert_eq!(
        StudioIndexProjection::read(&f.logical, 0, left.doc()).unwrap(),
        StudioIndexProjection::read(&f.logical, 0, right.doc()).unwrap()
    );

    let resolving = domain(&f.logical, &title(9, "Resolved"), 3);
    let (_, good) = draft(left.doc(), &f.logical, &resolving, &f.owner.device_id());
    let original = left.snapshot().unwrap();
    let original_gate = lg.encode().unwrap();
    let ObjId::Id(counter, actor, _) = left.doc().get(ROOT, "kind").unwrap().unwrap().1 else {
        panic!("header id")
    };
    for mode in 0..3 {
        let mut changed = good.decode();
        let entry = changed
            .operations
            .iter_mut()
            .find(|op| matches!(&op.key, Key::Map(k) if k.starts_with("t/")))
            .unwrap();
        assert_eq!(entry.pred.len(), 2);
        entry.pred = match mode {
            0 => vec![],
            1 => vec![entry.pred.iter().next().unwrap().clone()],
            _ => vec![OpId(counter, actor.clone())],
        }
        .into();
        let signed = SignedOp::sign_domain(
            &f.owner,
            DocType::StudioIndex,
            left.doc_id(),
            Change::from(changed).raw_bytes().to_vec(),
            &resolving,
        )
        .unwrap();
        let sealed = SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap();
        assert!(f.ingest(&mut left, &lg, &sealed, false).is_err());
        assert_eq!(left.snapshot().unwrap(), original);
        assert_eq!(lg.encode().unwrap(), original_gate);
    }
    // Causal validity, not receiver delivery order, permits a rename concurrent with deletion.
    let deletion = domain(&f.logical, &IndexOp::TombstoneObject { object: [9; 16] }, 3);
    let (_, dead) = draft(right.doc(), &f.logical, &deletion, &peer.device_id());
    let mut last = Vec::new();
    for (signer, domain, change) in [(&f.owner, &resolving, good), (&peer, &deletion, dead)] {
        let signed = SignedOp::sign_domain(
            signer,
            DocType::StudioIndex,
            left.doc_id(),
            change.raw_bytes().to_vec(),
            domain,
        )
        .unwrap();
        last.push(SealedOp::seal(&signed, &f.group, &f.owner, &mut f.rng).unwrap());
    }
    for sealed in &last {
        f.ingest(&mut left, &lg, sealed, false).unwrap();
    }
    for sealed in last.iter().rev() {
        f.ingest(&mut right, &rg, sealed, false).unwrap();
    }
    let projection = StudioIndexProjection::read(&f.logical, 0, left.doc()).unwrap();
    assert_eq!(
        projection,
        StudioIndexProjection::read(&f.logical, 0, right.doc()).unwrap()
    );
    assert_eq!(
        projection.deleted_objects[&[9; 16]].title.selected.value,
        "Resolved"
    );
}
