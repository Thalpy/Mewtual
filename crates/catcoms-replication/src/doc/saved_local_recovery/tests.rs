//! Actual signed graph inputs. Work counters measure the real vendor reconstruction/value
//! paths, not an injected membership boolean or wall-clock threshold.
use super::*;
use rand_core::SeedableRng;

fn budget(progress: &SavedLocalChatRecovery) {
    assert!(progress.last_work.records <= MAX_LOCAL_CHAT_RECOVERY_SCANNED_OPS);
    assert!(progress.last_work.bytes <= MAX_LOCAL_CHAT_RECOVERY_SCANNED_BYTES);
    assert!(progress.last_work.signatures <= HISTORY_SIGNATURES_PER_TURN);
}

#[test]
fn many_overlapping_origins_and_successors_never_reconstruct_or_commit_pending_edits() {
    let local = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&local).unwrap();
    let mut rng = rand_chacha::ChaCha20Rng::from_seed([0x95; 32]);
    let mut base =
        AutoCommit::new().with_actor(ActorId::from(local.device_id().as_bytes().to_vec()));
    base.put(ROOT, "shared", "base").unwrap();
    base.commit();
    let base_op = SignedOp::sign(
        &local,
        DocType::Channel,
        5,
        base.get_last_local_change().unwrap().raw_bytes().to_vec(),
    )
    .unwrap();
    let base_bytes = base.save();
    let mut doc = EncryptedDoc::new(DocType::Channel, 5, &local.device_id());
    doc.apply_signed_tracked_observed(base_op, |_, _| {})
        .unwrap();
    let (_, target) = doc
        .edit_tracked(&local, &group, &mut rng, |graph| {
            graph.put(ROOT, "shared", "tiny target")
        })
        .unwrap();
    for actor in 0..50 {
        let foreign = MlsDevice::generate().unwrap();
        let mut branch = AutoCommit::load(&base_bytes)
            .unwrap()
            .with_actor(ActorId::from(foreign.device_id().as_bytes().to_vec()));
        for round in 0..3 {
            branch
                .put(
                    ROOT,
                    "shared",
                    format!("{actor}/{round}/{}", "v".repeat(64 * 1024)),
                )
                .unwrap();
            branch.commit();
            let op = SignedOp::sign(
                &foreign,
                DocType::Channel,
                5,
                branch.get_last_local_change().unwrap().raw_bytes().to_vec(),
            )
            .unwrap();
            doc.apply_signed_tracked_observed(op, |_, _| {}).unwrap();
        }
    }
    // Positive control: the old observer really traverses reconstruction/value paths on
    // this exact graph, which has fifty origins sharing counter ranges and successors.
    let before_probe = automerge::history_recovery_work_on_this_thread();
    assert!(doc.doc.get_change_by_hash(&target).is_some());
    let measured = automerge::history_recovery_work_on_this_thread();
    assert!(measured.0 > before_probe.0 && measured.1 > before_probe.1);
    let heads = doc.doc.get_heads();
    let hashes = doc.doc.applied_change_hashes().to_vec();
    doc.doc
        .put(ROOT, "pending", "must stay uncommitted")
        .unwrap();
    let mut cursor = doc.start_saved_local_chat_recovery(&local, target);
    let mut turns = 0;
    loop {
        let result = doc
            .recover_saved_local_chat_op(&local, &mut cursor)
            .unwrap();
        budget(&cursor);
        turns += 1;
        assert_eq!(doc.doc.pending_ops(), 1);
        assert_eq!(doc.doc.applied_change_hashes(), hashes);
        assert_eq!(automerge::history_recovery_work_on_this_thread(), measured);
        if result.is_some() {
            break;
        }
        assert!(turns < 8);
    }
    assert!(turns > 1);
    let mut missing = doc.start_saved_local_chat_recovery(&local, ChangeHash([0; 32]));
    assert!(doc
        .recover_saved_local_chat_op(&local, &mut missing)
        .is_err());
    assert_eq!(doc.doc.pending_ops(), 1);
    assert_eq!(doc.doc.rollback(), 1);
    assert_eq!(doc.doc.get_heads(), heads);
}

#[test]
fn signed_orphan_is_not_applied_graph_provenance_until_its_real_parent_arrives() {
    let device = MlsDevice::generate().unwrap();
    let mut graph =
        AutoCommit::new().with_actor(ActorId::from(device.device_id().as_bytes().to_vec()));
    graph.put(ROOT, "first", 1u64).unwrap();
    graph.commit();
    let parent = SignedOp::sign(
        &device,
        DocType::Channel,
        5,
        graph.get_last_local_change().unwrap().raw_bytes().to_vec(),
    )
    .unwrap();
    graph.put(ROOT, "second", 2u64).unwrap();
    graph.commit();
    let child = graph.get_last_local_change().unwrap();
    let target = child.hash();
    let child = SignedOp::sign(&device, DocType::Channel, 5, child.raw_bytes().to_vec()).unwrap();
    let mut doc = EncryptedDoc::new(DocType::Channel, 5, &device.device_id());
    doc.apply_signed_tracked_observed(child.clone(), |_, _| {})
        .unwrap();
    assert!(!doc.doc.has_applied_change(&target));
    let mut progress = doc.start_saved_local_chat_recovery(&device, target);
    assert!(doc
        .recover_saved_local_chat_op(&device, &mut progress)
        .is_err());
    assert_eq!(progress.position, 0);
    doc.apply_signed_tracked_observed(parent, |_, _| {})
        .unwrap();
    assert!(doc.doc.has_applied_change(&target));
    let found = doc
        .recover_saved_local_chat_op(&device, &mut progress)
        .unwrap()
        .unwrap();
    assert_eq!(found.0, child);
}

#[test]
fn near_limit_original_yields_between_paid_phases_without_rehashing_or_reverification() {
    let device = MlsDevice::generate().unwrap();
    let build = |size: usize| {
        let mut graph =
            AutoCommit::new().with_actor(ActorId::from(device.device_id().as_bytes().to_vec()));
        graph.put(ROOT, "large", "x".repeat(size)).unwrap();
        graph.commit();
        let change = graph.get_last_local_change().unwrap();
        let hash = change.hash();
        let op = SignedOp::sign(&device, DocType::Channel, 5, change.raw_bytes().to_vec()).unwrap();
        let size = SignedOpFrame::new(&op).unwrap().encoded_len();
        let mut doc = EncryptedDoc::new(DocType::Channel, 5, &device.device_id());
        doc.apply_signed_tracked_observed(op, |_, _| {}).unwrap();
        (doc, hash, size)
    };
    let initial = MAX_SAVED_LOCAL_CHAT_BYTES - 1024;
    let (_, _, size) = build(initial);
    let (doc, target, exact) = build(initial + MAX_SAVED_LOCAL_CHAT_BYTES - size);
    assert_eq!(
        exact, MAX_SAVED_LOCAL_CHAT_BYTES,
        "the largest admitted frame must still make progress"
    );
    let mut cursor = doc.start_saved_local_chat_recovery(&device, target);
    assert!(doc
        .recover_saved_local_chat_op(&device, &mut cursor)
        .unwrap()
        .is_none());
    budget(&cursor);
    assert!(cursor.last_work.hash_bytes > 240_000);
    assert_eq!(cursor.last_work.signatures, 0);
    assert!(doc
        .recover_saved_local_chat_op(&device, &mut cursor)
        .unwrap()
        .is_none());
    budget(&cursor);
    assert_eq!(cursor.last_work.hash_bytes, 0);
    assert_eq!(cursor.last_work.signatures, 1);
    assert!(doc
        .recover_saved_local_chat_op(&device, &mut cursor)
        .unwrap()
        .is_some());
    budget(&cursor);
    assert_eq!(cursor.last_work.hash_bytes, 0);
    assert_eq!(cursor.last_work.signatures, 0);
}

#[test]
fn raw_identity_matches_real_change_hash_and_rejects_noncanonical_or_changed_framing() {
    let device = MlsDevice::generate().unwrap();
    let mut graph =
        AutoCommit::new().with_actor(ActorId::from(device.device_id().as_bytes().to_vec()));
    graph.put(ROOT, "v", "retained").unwrap();
    graph.commit();
    let change = graph.get_last_local_change().unwrap();
    let original = change.raw_bytes().to_vec();
    let actor = raw_identity(&original, change.hash()).unwrap().unwrap();
    assert_eq!(actor, *device.device_id().as_bytes());
    let mut bad = original.clone();
    bad[4] ^= 1;
    assert!(raw_identity(&bad, change.hash()).is_err());
    let mut bad = original.clone();
    bad.push(0);
    assert!(raw_identity(&bad, change.hash()).unwrap().is_none());
    // This small payload has a one-byte length. An overlong but equal LEB value must refuse.
    assert!(original[9] < 128);
    let mut bad = original.clone();
    bad[9] |= 128;
    bad.insert(10, 0);
    assert!(raw_identity(&bad, change.hash()).unwrap().is_none());
    let mut unsupported = original;
    unsupported[8] = 2;
    assert!(raw_identity(&unsupported, change.hash()).unwrap().is_none());
}

#[test]
fn same_key_legacy_actor_and_oversized_non_targets_do_not_block_a_small_original() {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let mut rng = rand_chacha::ChaCha20Rng::from_seed([0x98; 32]);
    let mut legacy = AutoCommit::new().with_actor(ActorId::from(vec![7; 16]));
    let mut doc = EncryptedDoc::new(DocType::Channel, 5, &device.device_id());
    let append_legacy = |graph: &mut AutoCommit, doc: &mut EncryptedDoc, value: u64| {
        graph.put(ROOT, "legacy", value).unwrap();
        graph.commit();
        let op = SignedOp::sign(
            &device,
            DocType::Channel,
            5,
            graph.get_last_local_change().unwrap().raw_bytes().to_vec(),
        )
        .unwrap();
        doc.apply_signed_tracked_observed(op, |_, _| {}).unwrap();
    };
    append_legacy(&mut legacy, &mut doc, 1);
    doc.edit_tracked(&device, &group, &mut rng, |graph| {
        graph.put(ROOT, "large", "x".repeat(300_000))
    })
    .unwrap();
    let (_, target) = doc
        .edit_tracked(&device, &group, &mut rng, |graph| {
            graph.put(ROOT, "target", "small")
        })
        .unwrap();
    append_legacy(&mut legacy, &mut doc, 2);
    let before = automerge::history_recovery_work_on_this_thread();
    let mut progress = doc.start_saved_local_chat_recovery(&device, target);
    let found = doc
        .recover_saved_local_chat_op(&device, &mut progress)
        .unwrap()
        .unwrap();
    budget(&progress);
    assert!(found.0.delta.len() < 1024);
    assert_eq!(automerge::history_recovery_work_on_this_thread(), before);
}

#[test]
fn a_later_conflicting_signature_is_not_hidden_by_a_previously_completed_cursor() {
    let device = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&device).unwrap();
    let mut doc = EncryptedDoc::new(DocType::Channel, 5, &device.device_id());
    let mut rng = rand_chacha::ChaCha20Rng::from_seed([0x97; 32]);
    let (_, target) = doc
        .edit_tracked(&device, &group, &mut rng, |graph| {
            graph.put(ROOT, "v", "original")
        })
        .unwrap();
    let mut cursor = doc.start_saved_local_chat_recovery(&device, target);
    assert!(doc
        .recover_saved_local_chat_op(&device, &mut cursor)
        .unwrap()
        .is_some());
    let mut invalid = doc.log[0].clone();
    invalid.signature[0] ^= 1;
    doc.log.push(invalid); // Negative retained-envelope fixture, not an authority issuer.
    assert!(matches!(
        doc.recover_saved_local_chat_op(&device, &mut cursor),
        Err(ReplError::BadSignature)
    ));
    budget(&cursor);
    assert_eq!(cursor.position, 1);
}
