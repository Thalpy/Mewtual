//! Checkpoints exercise real Automerge bytes, receipt verification, encrypted edits and vault
//! restore. Schema tests below the public boundary separately pin registry capacity/reclamation.

use std::cell::Cell;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, Change, ReadDoc, ROOT};
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::checkpoint::seed_actor;
use catcoms_replication::registry::{
    edit_registry, ingest_registry, registry_document, PointerKey, RegistryOp, RegistryProjection,
};
use catcoms_replication::{
    epoch_zero_id, Admission, CheckpointSeed, CloseRecord, EncryptedDoc, EpochGate,
    InheritedCheckpoint, LogicalDocument, Receipt, ReplError, SealedOp, SignedOp,
    MAX_CHECKPOINT_BYTES,
};
use catcoms_wire::DocType;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn receipt(owner: &MlsDevice, _group: &ServerGroup, seed: &CheckpointSeed) -> Receipt {
    Receipt::sign(
        seed.origin().document().clone(),
        seed.origin().epoch() - 1,
        [7; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        owner,
    )
    .unwrap()
}

fn small_seed(logical: &LogicalDocument) -> CheckpointSeed {
    CheckpointSeed::build(logical, 1, [7; 32], |doc| {
        doc.put(ROOT, "epoch", 1u64).unwrap();
        doc.put(ROOT, "kind", "test").unwrap();
        doc.put(ROOT, "title", "same pixels").unwrap();
        Ok(())
    })
    .unwrap()
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn whole_raw_seed_and_actor_have_fixed_vectors() {
    let logical = LogicalDocument::new(
        b"golden-server".to_vec(),
        DocType::StudioObject,
        b"golden-object".to_vec(),
    )
    .unwrap();
    let one = small_seed(&logical);
    let two = small_seed(&logical);
    assert_eq!(one.bytes(), two.bytes());
    // Whole Automerge 0.10 raw change, including checksum, actor and operation columns. Any
    // dependency upgrade that moves these bytes needs an explicit checkpoint-format decision.
    assert_eq!(bytes_hex(one.bytes()), "856f4a83a9e36e210160002060918912f1f3ec77b2f0f6efcd2d28dff92f0ed775bc7f39f0f003b51ce8a0fb0101000000061512340142025605571070027d0565706f6368046b696e64057469746c650303017d1346b601017465737473616d6520706978656c730300");
    assert_eq!(
        bytes_hex(&seed_actor(one.origin().doc_id())),
        "60918912f1f3ec77b2f0f6efcd2d28dff92f0ed775bc7f39f0f003b51ce8a0fb"
    );
    let parsed = Change::from_bytes(one.bytes().to_vec()).unwrap();
    assert_eq!(parsed.seq(), 1);
    assert_eq!(parsed.start_op().get(), 1);
    assert_eq!(parsed.timestamp(), 0);
    assert!(parsed.deps().is_empty());
}

#[test]
fn expected_seed_hash_rejects_before_schema_or_automerge_application() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let logical =
        LogicalDocument::new(group.group_id(), DocType::StudioObject, b"x".to_vec()).unwrap();
    let seed = small_seed(&logical);
    let receipt = receipt(&owner, &group, &seed)
        .verify_current_owner(&group, 0)
        .unwrap();
    let called = Cell::new(false);
    let mut corrupt = seed.bytes().to_vec();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        CheckpointSeed::verify(&receipt, &corrupt, |_, _, _| {
            called.set(true);
            Ok(())
        }),
        Err(ReplError::BadSignature)
    ));
    assert!(!called.get());
    assert!(matches!(
        CheckpointSeed::verify(
            &receipt,
            &vec![0; MAX_CHECKPOINT_BYTES + 1],
            |_, _, _| panic!("oversize reached schema")
        ),
        Err(ReplError::EpochBound)
    ));
    let mut compressed = Change::from_bytes(seed.bytes().to_vec()).unwrap();
    let compressed = compressed.bytes().into_owned();
    if compressed[8] == 2 {
        assert!(
            CheckpointSeed::verify(&receipt, &compressed, |_, _, _| panic!(
                "compressed reached schema"
            ))
            .is_err()
        );
    }
    assert!(
        CheckpointSeed::verify(&receipt, seed.bytes(), |_, _, _| Err(ReplError::Malformed))
            .is_err()
    );
}

#[test]
fn receipt_for_other_close_cannot_open_the_same_raw_seed() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let logical =
        LogicalDocument::new(group.group_id(), DocType::StudioObject, b"x".to_vec()).unwrap();
    let seed = small_seed(&logical);
    let wrong = Receipt::sign(
        logical,
        0,
        [8; 32],
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap()
    .verify_current_owner(&group, 0)
    .unwrap();
    assert!(matches!(
        CheckpointSeed::verify(&wrong, seed.bytes(), |_, _, _| Ok(())),
        Err(ReplError::Malformed)
    ));
}

#[test]
fn writer_cannot_copy_markers_or_an_old_dag_into_a_seed() {
    let logical =
        LogicalDocument::new(b"server".to_vec(), DocType::StudioObject, b"x".to_vec()).unwrap();
    assert!(CheckpointSeed::build(&logical, 1, [7; 32], |doc| {
        doc.put(ROOT, "_p1/op/forged", 1u64).unwrap();
        Ok(())
    })
    .is_err());
    assert!(CheckpointSeed::build(&logical, 1, [7; 32], |doc| {
        doc.put(ROOT, "_p1/op/hidden", 1u64).unwrap();
        doc.delete(ROOT, "_p1/op/hidden").unwrap();
        doc.put(ROOT, "v", 1u64).unwrap();
        Ok(())
    })
    .is_err());
    assert!(CheckpointSeed::build(&logical, 1, [7; 32], |doc| {
        doc.put(ROOT, "one", 1u64).unwrap();
        doc.commit();
        doc.put(ROOT, "two", 2u64).unwrap();
        Ok(())
    })
    .is_err());
    assert!(matches!(
        CheckpointSeed::build(&logical, 1, [7; 32], |doc| {
            doc.put(ROOT, "large", "a".repeat(MAX_CHECKPOINT_BYTES))
                .unwrap();
            Ok(())
        }),
        Err(ReplError::EpochBound)
    ));
}

#[test]
fn installed_registry_restarts_and_accepts_only_its_own_successor_history() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"flipnote-a".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(DocType::DocRegistry, &logical.logical_key);
    let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let mut original = EncryptedDoc::new(DocType::DocRegistry, id, &owner.device_id());
    let mut rng = ChaCha20Rng::from_seed([5; 32]);
    let first = RegistryOp::Put {
        key: key.clone(),
        epoch: 0,
    }
    .domain_op(&group.group_id(), [1; 16])
    .unwrap();
    edit_registry(
        &mut original,
        &gate,
        bucket,
        &owner,
        &group,
        &mut rng,
        &first,
    )
    .unwrap();
    let seed = RegistryProjection::read(&logical, bucket, 0, original.doc())
        .unwrap()
        .checkpoint([7; 32])
        .unwrap();
    let selected = receipt(&owner, &group, &seed)
        .verify_current_owner(&group, 0)
        .unwrap();
    let checkpoint =
        RegistryProjection::verify_checkpoint(&selected, bucket, seed.bytes()).unwrap();
    let mut successor = EncryptedDoc::from_checkpoint(&checkpoint, &owner.device_id()).unwrap();
    let mut peer = EncryptedDoc::from_checkpoint(&checkpoint, &owner.device_id()).unwrap();
    assert_eq!(successor.op_count(), 0);
    assert!(!successor
        .has_domain_marker(&first.id(&owner.device_id()))
        .unwrap());
    let next_gate = EpochGate::new(
        logical.clone(),
        checkpoint.origin().doc_id(),
        1,
        owner.device_id(),
    );
    let peer_gate = EpochGate::new(
        logical.clone(),
        checkpoint.origin().doc_id(),
        1,
        owner.device_id(),
    );
    let second = RegistryOp::Put {
        key: key.clone(),
        epoch: 1,
    }
    .domain_op(&group.group_id(), [2; 16])
    .unwrap();
    let sealed = edit_registry(
        &mut successor,
        &next_gate,
        bucket,
        &owner,
        &group,
        &mut rng,
        &second,
    )
    .unwrap();
    let op = sealed
        .open(
            &group
                .channel_secret(&owner, DocType::DocRegistry, successor.doc_id())
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        Change::from_bytes(op.delta).unwrap().deps(),
        &[automerge::ChangeHash(seed.change_hash())]
    );
    assert_eq!(
        ingest_registry(&mut peer, &peer_gate, bucket, &sealed, &group, &owner).unwrap(),
        Admission::Accepted
    );
    assert_eq!(
        RegistryProjection::read(&logical, bucket, 1, peer.doc())
            .unwrap()
            .pointers[&key],
        1
    );
    let snapshot = successor.snapshot().unwrap();
    let mut restored = EncryptedDoc::restore_for_actor(&snapshot, &owner.device_id()).unwrap();
    assert_eq!(restored.checkpoint_origin(), Some(checkpoint.origin()));
    assert_eq!(restored.checkpoint_bytes().unwrap().unwrap(), seed.bytes());
    let restored_gate = EpochGate::decode(&next_gate.encode().unwrap()).unwrap();
    let third = RegistryOp::Put { key, epoch: 2 }
        .domain_op(&group.group_id(), [3; 16])
        .unwrap();
    let sealed = edit_registry(
        &mut restored,
        &restored_gate,
        bucket,
        &owner,
        &group,
        &mut rng,
        &third,
    )
    .unwrap();
    ingest_registry(&mut peer, &peer_gate, bucket, &sealed, &group, &owner).unwrap();
    assert_eq!(restored.heads(), peer.heads());
    let mut seedless = EncryptedDoc::new(
        DocType::DocRegistry,
        checkpoint.origin().doc_id(),
        &owner.device_id(),
    );
    assert!(matches!(
        edit_registry(
            &mut seedless,
            &next_gate,
            bucket,
            &owner,
            &group,
            &mut rng,
            &third
        ),
        Err(ReplError::EpochScope)
    ));
}

#[test]
fn exact_preflight_rejection_leaves_the_document_and_gate_unchanged() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let logical =
        LogicalDocument::new(group.group_id(), DocType::StudioObject, b"x".to_vec()).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let mut doc = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
    let before = doc.snapshot().unwrap();
    let domain = catcoms_replication::DomainOp {
        nonce: [1; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        body: b"{}".to_vec(),
    };
    let result = doc.edit_domain_preflight_gated(
        &logical,
        &gate,
        &owner,
        &group,
        &mut ChaCha20Rng::from_seed([0; 32]),
        &domain,
        |doc| {
            doc.put(ROOT, "title", "valid but full")?;
            Ok(())
        },
        |_, _| Ok(()),
        |_| Err(ReplError::EpochBound),
    );
    assert!(matches!(result, Err(ReplError::EpochBound)));
    assert_eq!(doc.snapshot().unwrap(), before);
    assert!(gate.accepted_hashes().is_empty());
    assert!(doc.doc().get(ROOT, "title").unwrap().is_none());
}

#[test]
fn seed_preflight_length_is_independent_of_the_not_yet_known_close_hash() {
    let logical = registry_document(b"server", 0).unwrap();
    let projection = RegistryProjection::read(&logical, 0, 0, &AutoCommit::new()).unwrap();
    let a = projection.checkpoint([0; 32]).unwrap();
    let b = projection.checkpoint([255; 32]).unwrap();
    assert_eq!(a.bytes().len(), b.bytes().len());
    assert_ne!(a.change_hash(), b.change_hash());
}

#[test]
fn registry_delta_cannot_smuggle_slots_or_confirm_an_unapplied_operation() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"frame".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let domain = RegistryOp::Put { key, epoch: 0 }
        .domain_op(&group.group_id(), [4; 16])
        .unwrap();
    for extra in [None, Some("s/0010/6672616d65"), Some("unrelated")] {
        let mut sender = AutoCommit::new().with_actor(automerge::ActorId::from(
            owner.device_id().as_bytes().to_vec(),
        ));
        if let Some(key) = extra {
            sender.put(ROOT, key, true).unwrap();
        }
        sender
            .put(
                ROOT,
                format!("_p1/op/{}", bytes_hex(&domain.id(&owner.device_id()))),
                1u64,
            )
            .unwrap();
        sender.commit();
        let op = SignedOp::sign_domain(
            &owner,
            logical.doc_type,
            id,
            sender.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed =
            SealedOp::seal(&op, &group, &owner, &mut ChaCha20Rng::from_seed([5; 32])).unwrap();
        let mut target = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
        let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
        assert!(ingest_registry(&mut target, &gate, bucket, &sealed, &group, &owner).is_err());
        assert!(target.heads().is_empty());
        assert!(gate.accepted_hashes().is_empty());
    }
}

#[test]
fn checkpoint_rejects_a_validly_signed_independent_root() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let logical =
        LogicalDocument::new(group.group_id(), DocType::StudioObject, b"x".to_vec()).unwrap();
    let seed = small_seed(&logical);
    let receipt = receipt(&owner, &group, &seed)
        .verify_current_owner(&group, 0)
        .unwrap();
    let checkpoint = CheckpointSeed::verify(&receipt, seed.bytes(), |_, _, _| Ok(())).unwrap();
    let mut target = EncryptedDoc::from_checkpoint(&checkpoint, &owner.device_id()).unwrap();
    let gate = EpochGate::new(logical.clone(), target.doc_id(), 1, owner.device_id());
    let domain = catcoms_replication::DomainOp {
        nonce: [3; 16],
        doc_type: logical.doc_type,
        logical_key: logical.logical_key.clone(),
        body: b"{}".to_vec(),
    };
    let mut foreign = AutoCommit::new().with_actor(automerge::ActorId::from(
        owner.device_id().as_bytes().to_vec(),
    ));
    foreign.put(ROOT, "title", "foreign root").unwrap();
    foreign
        .put(
            ROOT,
            format!("_p1/op/{}", bytes_hex(&domain.id(&owner.device_id()))),
            1u64,
        )
        .unwrap();
    foreign.commit();
    let op = SignedOp::sign_domain(
        &owner,
        logical.doc_type,
        target.doc_id(),
        foreign
            .get_last_local_change()
            .unwrap()
            .raw_bytes()
            .to_vec(),
        &domain,
    )
    .unwrap();
    let sealed = SealedOp::seal(&op, &group, &owner, &mut ChaCha20Rng::from_seed([4; 32])).unwrap();
    let before = target.snapshot().unwrap();
    assert!(matches!(
        target.ingest_domain_gated(&logical, &gate, &sealed, &group, &owner, |_, _| Ok(())),
        Err(ReplError::EpochScope)
    ));
    assert_eq!(target.snapshot().unwrap(), before);
    assert!(gate.accepted_hashes().is_empty());
}

#[test]
fn close_accounting_excludes_only_the_installed_seed_and_selects_exact_heads() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let logical =
        LogicalDocument::new(group.group_id(), DocType::StudioObject, b"x".to_vec()).unwrap();
    let seed = small_seed(&logical);
    let verified = receipt(&owner, &group, &seed)
        .verify_current_owner(&group, 0)
        .unwrap();
    let checkpoint = CheckpointSeed::verify(&verified, seed.bytes(), |_, _, _| Ok(())).unwrap();
    let mut doc = EncryptedDoc::from_checkpoint(&checkpoint, &owner.device_id()).unwrap();
    let gate = EpochGate::new(logical.clone(), doc.doc_id(), 1, owner.device_id());
    let mut rng = ChaCha20Rng::from_seed([1; 32]);
    for i in 0..10u8 {
        let domain = catcoms_replication::DomainOp {
            nonce: [i; 16],
            doc_type: logical.doc_type,
            logical_key: logical.logical_key.clone(),
            body: b"{}".to_vec(),
        };
        doc.edit_domain_gated(
            &logical,
            &gate,
            &owner,
            &group,
            &mut rng,
            &domain,
            |doc| {
                doc.put(
                    ROOT,
                    "large",
                    char::from(b'a' + i).to_string().repeat(230_000),
                )?;
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap();
    }
    let close = CloseRecord::sign(&logical, doc.doc_id(), 1, doc.heads(), &owner).unwrap();
    let stats = close
        .verify_and_validate(
            &logical,
            doc.doc_id(),
            &group,
            None,
            &mut doc,
            Some(seed.change_hash()),
        )
        .unwrap();
    assert_eq!(stats.operation_count, 10);
    assert!(stats.encoded_bytes >= 2 * 1024 * 1024);
    assert!(matches!(
        close.verify_and_validate(&logical, doc.doc_id(), &group, None, &mut doc, None),
        Err(ReplError::EpochScope)
    ));
    let wrong_epoch = CloseRecord::sign(&logical, doc.doc_id(), 0, doc.heads(), &owner).unwrap();
    assert!(matches!(
        wrong_epoch.verify_and_validate(
            &logical,
            doc.doc_id(),
            &group,
            None,
            &mut doc,
            Some(seed.change_hash())
        ),
        Err(ReplError::EpochScope)
    ));
}

#[test]
fn registry_seed_comes_from_receipted_heads_and_does_not_erase_later_work() {
    use automerge::transaction::CommitOptions;
    use catcoms_replication::registry::checkpoint_registry_close;
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"flipnote".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut target = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
    let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let mut writer = AutoCommit::new().with_actor(automerge::ActorId::from(
        owner.device_id().as_bytes().to_vec(),
    ));
    let mut rng = ChaCha20Rng::from_seed([21; 32]);
    let mut close = None;
    for i in 0..11u8 {
        let domain = RegistryOp::Put {
            key: key.clone(),
            epoch: u64::from(i),
        }
        .domain_op(&group.group_id(), [i; 16])
        .unwrap();
        writer.put(ROOT, "bucket", u64::from(bucket)).unwrap();
        writer.put(ROOT, "epoch", 0u64).unwrap();
        writer
            .put(ROOT, "key", bytes_hex(&logical.logical_key))
            .unwrap();
        writer.put(ROOT, "kind", "registry").unwrap();
        writer.put(ROOT, "v", 1u64).unwrap();
        writer
            .put(
                ROOT,
                format!("p/0010/{}", bytes_hex(key.logical_key())),
                u64::from(i),
            )
            .unwrap();
        writer
            .put(
                ROOT,
                format!("_p1/op/{}", bytes_hex(&domain.id(&owner.device_id()))),
                1u64,
            )
            .unwrap();
        // The whole signed envelope is charged. A large advisory commit message reaches the
        // real byte lower bound with ten operations, without a slow 10,000-operation fixture.
        writer.commit_with(CommitOptions::default().with_message("x".repeat(220_000)));
        let op = SignedOp::sign_domain(
            &owner,
            logical.doc_type,
            id,
            writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
            &domain,
        )
        .unwrap();
        let sealed = SealedOp::seal(&op, &group, &owner, &mut rng).unwrap();
        ingest_registry(&mut target, &gate, bucket, &sealed, &group, &owner).unwrap();
        if i == 9 {
            close = Some(CloseRecord::sign(&logical, id, 0, target.heads(), &owner).unwrap());
        }
    }
    let before = target.heads();
    let close = close.unwrap();
    let (seed, closure) =
        checkpoint_registry_close(&mut target, &gate, bucket, &close, &group, None).unwrap();
    assert_eq!(closure.operation_count, 10);
    let receipt = Receipt::sign(
        logical.clone(),
        0,
        close.hash(),
        seed.change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        &owner,
    )
    .unwrap()
    .verify_current_owner(&group, 0)
    .unwrap();
    let verified = RegistryProjection::verify_checkpoint(&receipt, bucket, seed.bytes()).unwrap();
    let opened = EncryptedDoc::from_checkpoint(&verified, &owner.device_id()).unwrap();
    assert_eq!(
        RegistryProjection::read(&logical, bucket, 1, opened.doc())
            .unwrap()
            .pointers[&key],
        9
    );
    assert_eq!(
        RegistryProjection::read(&logical, bucket, 0, target.doc())
            .unwrap()
            .pointers[&key],
        10
    );
    assert_eq!(target.heads(), before);
    assert_eq!(target.op_count(), 11);
}

#[test]
fn admitted_relayer_cannot_allocate_shares_for_unadmitted_signing_keys() {
    let owner = MlsDevice::generate().unwrap();
    let outsider = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"entry".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let domain = RegistryOp::Put { key, epoch: 0 }
        .domain_op(&group.group_id(), [1; 16])
        .unwrap();
    let mut writer = AutoCommit::new().with_actor(automerge::ActorId::from(
        outsider.device_id().as_bytes().to_vec(),
    ));
    // Keep the entire registry schema valid so this fixture would have been accepted before
    // the inner-author roster check; no unrelated malformed-header rejection can mask it.
    writer.put(ROOT, "bucket", u64::from(bucket)).unwrap();
    writer.put(ROOT, "epoch", 0u64).unwrap();
    writer
        .put(ROOT, "key", bytes_hex(&logical.logical_key))
        .unwrap();
    writer.put(ROOT, "kind", "registry").unwrap();
    writer.put(ROOT, "v", 1u64).unwrap();
    writer.put(ROOT, "p/0010/656e747279", 0u64).unwrap();
    writer
        .put(
            ROOT,
            format!("_p1/op/{}", bytes_hex(&domain.id(&outsider.device_id()))),
            1u64,
        )
        .unwrap();
    writer.commit();
    let op = SignedOp::sign_domain(
        &outsider,
        logical.doc_type,
        id,
        writer.get_last_local_change().unwrap().raw_bytes().to_vec(),
        &domain,
    )
    .unwrap();
    let sealed = SealedOp::seal(&op, &group, &owner, &mut ChaCha20Rng::from_seed([1; 32])).unwrap();
    let mut target = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
    let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    assert!(matches!(
        ingest_registry(&mut target, &gate, bucket, &sealed, &group, &owner),
        Err(ReplError::EpochAuthority)
    ));
    assert!(gate.accepted_hashes().is_empty());
    assert!(target.heads().is_empty());
}

#[test]
fn concurrent_bucket_creation_remains_editable_and_repeated_intents_get_markers() {
    let owner = MlsDevice::generate().unwrap();
    let peer = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    let added = group
        .add_member(&owner, peer.key_package().unwrap())
        .unwrap();
    let peer_group = ServerGroup::join(&peer, &added.welcome).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"same-pointer".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut a = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
    let mut b = EncryptedDoc::new(logical.doc_type, id, &peer.device_id());
    let ga = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let gb = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let mut rng = ChaCha20Rng::from_seed([22; 32]);
    let op = RegistryOp::Put {
        key: key.clone(),
        epoch: 0,
    };
    let da = op.domain_op(&group.group_id(), [1; 16]).unwrap();
    let db = op.domain_op(&group.group_id(), [2; 16]).unwrap();
    let sa = edit_registry(&mut a, &ga, bucket, &owner, &group, &mut rng, &da).unwrap();
    let sb = edit_registry(&mut b, &gb, bucket, &peer, &peer_group, &mut rng, &db).unwrap();
    ingest_registry(&mut a, &ga, bucket, &sb, &group, &owner).unwrap();
    ingest_registry(&mut b, &gb, bucket, &sa, &peer_group, &peer).unwrap();
    assert_eq!(a.doc().get_all(ROOT, "v").unwrap().len(), 2);
    for (i, op) in [
        op,
        RegistryOp::Put {
            key: key.clone(),
            epoch: 1,
        },
        RegistryOp::Tombstone { key: key.clone() },
        RegistryOp::Tombstone { key: key.clone() },
    ]
    .into_iter()
    .enumerate()
    {
        let domain = op.domain_op(&group.group_id(), [10 + i as u8; 16]).unwrap();
        let sealed = edit_registry(&mut a, &ga, bucket, &owner, &group, &mut rng, &domain).unwrap();
        ingest_registry(&mut b, &gb, bucket, &sealed, &peer_group, &peer).unwrap();
        assert!(a.has_domain_marker(&domain.id(&owner.device_id())).unwrap());
        assert!(b.has_domain_marker(&domain.id(&owner.device_id())).unwrap());
    }
    assert_eq!(a.heads(), b.heads());
    assert!(!RegistryProjection::read(&logical, bucket, 0, a.doc())
        .unwrap()
        .pointers
        .contains_key(&key));
}

#[test]
fn allowed_header_put_cannot_remove_a_seed_slot_through_a_cross_key_predecessor() {
    use automerge::legacy::{Key, OpId, OpType};
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let key = PointerKey::new(DocType::StudioObject, b"entry".to_vec()).unwrap();
    let bucket = key.bucket();
    let logical = registry_document(&group.group_id(), bucket).unwrap();
    let id = epoch_zero_id(logical.doc_type, &logical.logical_key);
    let mut source = EncryptedDoc::new(logical.doc_type, id, &owner.device_id());
    let gate = EpochGate::new(logical.clone(), id, 0, owner.device_id());
    let mut rng = ChaCha20Rng::from_seed([25; 32]);
    let domain = RegistryOp::Put {
        key: key.clone(),
        epoch: 0,
    }
    .domain_op(&group.group_id(), [1; 16])
    .unwrap();
    edit_registry(
        &mut source,
        &gate,
        bucket,
        &owner,
        &group,
        &mut rng,
        &domain,
    )
    .unwrap();
    let seed = RegistryProjection::read(&logical, bucket, 0, source.doc())
        .unwrap()
        .checkpoint([7; 32])
        .unwrap();
    let selected = receipt(&owner, &group, &seed)
        .verify_current_owner(&group, 0)
        .unwrap();
    let checkpoint =
        RegistryProjection::verify_checkpoint(&selected, bucket, seed.bytes()).unwrap();
    let mut target = EncryptedDoc::from_checkpoint(&checkpoint, &owner.device_id()).unwrap();
    let next_gate = EpochGate::new(logical.clone(), target.doc_id(), 1, owner.device_id());
    let domain = RegistryOp::Put { key, epoch: 0 }
        .domain_op(&group.group_id(), [2; 16])
        .unwrap();
    let mut malicious = target.doc().clone();
    let slot_id = malicious.get(ROOT, "s/0010/656e747279").unwrap().unwrap().1;
    let automerge::ObjId::Id(counter, actor, _) = slot_id else {
        panic!("seed slot id");
    };
    malicious.put(ROOT, "v", 2u64).unwrap();
    malicious
        .put(
            ROOT,
            format!("_p1/op/{}", bytes_hex(&domain.id(&owner.device_id()))),
            1u64,
        )
        .unwrap();
    malicious.commit();
    let mut expanded = malicious.get_last_local_change().unwrap().decode();
    for op in &mut expanded.operations {
        if matches!(&op.key, Key::Map(key) if key == "v") {
            op.action = OpType::Put(automerge::ScalarValue::Uint(1));
            op.pred = vec![OpId(counter, actor.clone())].into();
        }
    }
    let forged = Change::from(expanded);
    let op = SignedOp::sign_domain(
        &owner,
        logical.doc_type,
        target.doc_id(),
        forged.raw_bytes().to_vec(),
        &domain,
    )
    .unwrap();
    let sealed = SealedOp::seal(&op, &group, &owner, &mut rng).unwrap();
    let before = target.snapshot().unwrap();
    assert!(matches!(
        ingest_registry(&mut target, &next_gate, bucket, &sealed, &group, &owner),
        Err(ReplError::Malformed)
    ));
    assert_eq!(target.snapshot().unwrap(), before);
    assert!(next_gate.accepted_hashes().is_empty());
}
