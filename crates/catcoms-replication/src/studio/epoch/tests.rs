use super::*;
use crate::InheritedCheckpoint;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;

fn rng() -> ChaCha20Rng {
    ChaCha20Rng::from_seed([31; 32])
}
fn target(frame: bool) -> StudioTarget {
    if frame {
        StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        }
    } else {
        StudioTarget::Index { channel: [7; 16] }
    }
}
fn domain(unit: &StudioEpoch, body: Vec<u8>, nonce: u8) -> DomainOp {
    DomainOp {
        nonce: [nonce; 16],
        doc_type: unit.logical.doc_type,
        logical_key: unit.logical.logical_key.clone(),
        body,
    }
}
fn insert(unit: &StudioEpoch, author: DeviceId) -> DomainOp {
    let body = match unit.target {
        StudioTarget::Index { .. } => IndexOp::PutObject {
            object: [1; 16],
            kind: StudioKind::Flipnote,
            title: "private moon".into(),
            created_by: author,
            ts: 100,
            expiry: StudioExpiry::Never,
        }
        .encode()
        .unwrap(),
        _ => FlipnoteOp::InsertFrame {
            frame: [1; 16],
            after: None,
            cid: [3; 32],
            bytes: 10,
        }
        .encode()
        .unwrap(),
    };
    domain(unit, body, 1)
}
fn title(unit: &StudioEpoch, nonce: u8) -> DomainOp {
    let body = match unit.target {
        StudioTarget::Index { .. } => IndexOp::SetTitle {
            object: [1; 16],
            title: "renamed".into(),
        }
        .encode()
        .unwrap(),
        _ => FlipnoteOp::SetHeader(FlipnoteHeader::Title("renamed".into()))
            .encode()
            .unwrap(),
    };
    domain(unit, body, nonce)
}
fn receipt(unit: &StudioEpoch, owner: &MlsDevice, close: u8) -> Receipt {
    Receipt::sign(
        unit.logical.clone(),
        unit.epoch(),
        [close; 32],
        unit.projection()
            .unwrap()
            .checkpoint([close; 32])
            .unwrap()
            .change_hash(),
        0,
        InheritedCheckpoint::EpochZero,
        owner,
    )
    .unwrap()
}

#[test]
fn studio_epoch_restarts_and_reseals_original_change_after_later_deletion() {
    for frame in [false, true] {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let mut unit = StudioEpoch::new(&group, target(frame), owner.device_id()).unwrap();
        let op = insert(&unit, owner.device_id());
        let first = unit
            .edit_or_reseal(&owner, &group, &mut rng(), &op, 100)
            .unwrap();
        let body = if frame {
            FlipnoteOp::RemoveFrame { frame: [1; 16] }.encode().unwrap()
        } else {
            IndexOp::TombstoneObject { object: [1; 16] }
                .encode()
                .unwrap()
        };
        let delete = domain(&unit, body, 2);
        unit.edit_or_reseal(&owner, &group, &mut rng(), &delete, 101)
            .unwrap();
        let bytes = unit.snapshot().unwrap();
        let projection = unit.projection().unwrap();
        let mut restored =
            StudioEpoch::restore(&bytes, &group, target(frame), owner.device_id()).unwrap();
        assert_eq!(restored.projection().unwrap(), projection);
        assert!(restored
            .contains_exact_operation(owner.device_id(), &op)
            .unwrap());
        assert!(!restored
            .contains_exact_operation(DeviceId::from_bytes([0; 32]), &op)
            .unwrap());
        let again = restored
            .edit_or_reseal(&owner, &group, &mut rng(), &op, 999)
            .unwrap();
        let key = group
            .channel_secret(&owner, first.doc_type, first.doc_id)
            .unwrap();
        assert_eq!(
            first.open(&key).unwrap().encode(),
            again.open(&key).unwrap().encode()
        );
        assert_eq!(restored.snapshot().unwrap(), bytes);
        assert!(!format!("{restored:?}").contains("private"));
        let mut conflict = op;
        conflict.body = title(&unit, 1).body;
        assert!(matches!(
            restored.validate_local_edit(&owner, &group, &conflict, 0),
            Err(ReplError::IntentConflict)
        ));
        let mut wrong = target(frame);
        if let StudioTarget::Flipnote {
            ref mut channel, ..
        } = wrong
        {
            *channel = [8; 16];
        } else {
            wrong = StudioTarget::Index { channel: [8; 16] };
        }
        assert!(StudioEpoch::restore(&bytes, &group, wrong, owner.device_id()).is_err());
        for malformed in [
            bytes[..bytes.len() - 1].to_vec(),
            [bytes.clone(), vec![0]].concat(),
            vec![0; MAX_STUDIO_EPOCH_SNAPSHOT_BYTES + 1],
        ] {
            assert!(
                StudioEpoch::restore(&malformed, &group, target(frame), owner.device_id()).is_err()
            );
        }
    }
}

#[test]
fn studio_epoch_seed_log_gate_and_receipts_restore_together_and_delayed_fault_survives() {
    for frame in [false, true] {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let mut source = StudioEpoch::new(&group, target(frame), owner.device_id()).unwrap();
        let op = insert(&source, owner.device_id());
        source
            .edit_or_reseal(&owner, &group, &mut rng(), &op, 100)
            .unwrap();
        let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
        let opening = receipt(&source, &owner, 7);
        let mut unit = StudioEpoch::from_checkpoint(
            &group,
            target(frame),
            owner.device_id(),
            opening.clone(),
            0,
            seed.bytes(),
        )
        .unwrap();
        let op = title(&unit, 2);
        unit.edit_or_reseal(&owner, &group, &mut rng(), &op, 101)
            .unwrap();
        let bytes = unit.snapshot().unwrap();
        let protocol = unit.storage_protocol_bytes().unwrap();
        assert_eq!(
            StudioEpoch::validate_vault_snapshot(&bytes, &group.group_id(), target(frame)).unwrap(),
            protocol
        );
        let mut unit =
            StudioEpoch::restore(&bytes, &group, target(frame), owner.device_id()).unwrap();
        assert_eq!(unit.epoch(), 1);
        assert_eq!(unit.op_count(), 1);
        let next = receipt(&unit, &owner, 8);
        unit.seal(next, &group, 0).unwrap();
        assert_eq!(unit.phase(), EpochPhase::Closing);
        let closed = unit.snapshot().unwrap();
        let mut unit =
            StudioEpoch::restore(&closed, &group, target(frame), owner.device_id()).unwrap();
        let conflicting = receipt(&source, &owner, 9);
        unit.seal(conflicting, &group, 0).unwrap();
        assert_eq!(unit.phase(), EpochPhase::Fault);
        let mut restored = StudioEpoch::restore(
            &unit.snapshot().unwrap(),
            &group,
            target(frame),
            owner.device_id(),
        )
        .unwrap();
        assert_eq!(restored.phase(), EpochPhase::Fault);
        assert_eq!(restored.op_count(), 1);
        assert!(restored
            .edit_or_reseal(&owner, &group, &mut rng(), &op, 100)
            .is_err());
        // Public projection equality cannot replace exact signed-log/gate or receipt evidence.
        let mut valid =
            StudioEpoch::restore(&bytes, &group, target(frame), owner.device_id()).unwrap();
        valid.receipts = ReceiptBook::default();
        assert!(StudioEpoch::restore(
            &valid.snapshot().unwrap(),
            &group,
            target(frame),
            owner.device_id()
        )
        .is_err());
    }
}

#[test]
fn studio_epoch_removed_author_history_remains_readable_but_cannot_reseal() {
    let owner = MlsDevice::generate().unwrap();
    let peer = MlsDevice::generate().unwrap();
    let mut group = ServerGroup::create(&owner).unwrap();
    group
        .add_member(&owner, peer.key_package().unwrap())
        .unwrap();
    let mut unit = StudioEpoch::new(&group, target(true), peer.device_id()).unwrap();
    let op = insert(&unit, peer.device_id());
    unit.edit_or_reseal(&peer, &group, &mut rng(), &op, 100)
        .unwrap();
    let bytes = unit.snapshot().unwrap();
    group.remove_member(&owner, &peer.device_id()).unwrap();
    let mut restored =
        StudioEpoch::restore(&bytes, &group, target(true), peer.device_id()).unwrap();
    assert_eq!(restored.op_count(), 1);
    assert!(matches!(
        restored.edit_or_reseal(&peer, &group, &mut rng(), &op, 100),
        Err(ReplError::EpochAuthority)
    ));
    assert_eq!(restored.projection().unwrap(), unit.projection().unwrap());
    let inventory =
        StudioEpoch::validate_vault_snapshot(&bytes, &group.group_id(), target(true)).unwrap();
    assert_eq!(inventory, 0);
}

#[test]
fn studio_references_keep_seed_replacements_hidden_by_successor_edits() {
    let owner = MlsDevice::generate().unwrap();
    let group = ServerGroup::create(&owner).unwrap();
    let mut source = StudioEpoch::new(&group, target(true), owner.device_id()).unwrap();
    let a = insert(&source, owner.device_id());
    source
        .edit_or_reseal(&owner, &group, &mut rng(), &a, 100)
        .unwrap(); // birth A = 3
    let b = domain(
        &source,
        FlipnoteOp::ReplaceFrame {
            frame: [1; 16],
            cid: [4; 32],
            bytes: 10,
        }
        .encode()
        .unwrap(),
        2,
    );
    source
        .edit_or_reseal(&owner, &group, &mut rng(), &b, 101)
        .unwrap();
    let seed = source.projection().unwrap().checkpoint([7; 32]).unwrap();
    let opening = receipt(&source, &owner, 7);
    let mut next = StudioEpoch::from_checkpoint(
        &group,
        target(true),
        owner.device_id(),
        opening,
        0,
        seed.bytes(),
    )
    .unwrap();
    let c = domain(
        &next,
        FlipnoteOp::ReplaceFrame {
            frame: [1; 16],
            cid: [5; 32],
            bytes: 10,
        }
        .encode()
        .unwrap(),
        3,
    );
    next.edit_or_reseal(&owner, &group, &mut rng(), &c, 102)
        .unwrap();
    assert!(
        !super::super::references::projection_cids(&next.projection().unwrap()).contains(&[4; 32])
    );
    let all = std::collections::BTreeSet::from([[3; 32], [4; 32], [5; 32]]);
    assert_eq!(next.blob_cids().unwrap(), all);
    let bytes = next.snapshot().unwrap();
    assert_eq!(
        StudioEpoch::inspect_vault_references(&bytes, &group.group_id(), target(true))
            .unwrap()
            .1,
        all
    );
}

// Re-encode a vault-authenticated container with damaged internals. Vault authentication is not
// a substitute for checking signed-change provenance, dependency closure and the restart gate.
fn rewrite_snapshot(
    bytes: &[u8],
    edit: impl FnOnce(&mut [Vec<u8>], &mut Vec<SignedOp>),
) -> Vec<u8> {
    let mut d = Decoder::new(bytes);
    assert_eq!(d.get_u8().unwrap(), 1);
    let mut fields: Vec<Vec<u8>> = (0..5).map(|_| d.get_bytes().unwrap().to_vec()).collect();
    let count = d.get_u32().unwrap();
    let mut ops = (0..count)
        .map(|_| SignedOp::decode(d.get_bytes().unwrap()).unwrap())
        .collect();
    d.finish().unwrap();
    edit(&mut fields, &mut ops);
    let mut e = Encoder::new();
    e.put_u8(1);
    for field in fields {
        e.put_bytes(&field).unwrap();
    }
    e.put_u32(ops.len() as u32);
    for op in ops {
        e.put_bytes(&op.encode()).unwrap();
    }
    e.finish()
}

#[test]
fn studio_epoch_rejects_tampered_log_gate_seed_and_allocation_counts() {
    for frame in [false, true] {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let mut unit = StudioEpoch::new(&group, target(frame), owner.device_id()).unwrap();
        let empty = unit.snapshot().unwrap();
        let op = insert(&unit, owner.device_id());
        unit.edit_or_reseal(&owner, &group, &mut rng(), &op, 100)
            .unwrap();
        let op = title(&unit, 2);
        unit.edit_or_reseal(&owner, &group, &mut rng(), &op, 101)
            .unwrap();
        let bytes = unit.snapshot().unwrap();
        let mut empty_decoder = Decoder::new(&empty);
        empty_decoder.get_u8().unwrap();
        for _ in 0..4 {
            empty_decoder.get_bytes().unwrap();
        }
        let empty_gate = empty_decoder.get_bytes().unwrap();
        let malformed = [
            rewrite_snapshot(&bytes, |_, ops| {
                ops.remove(0);
            }), // missing causal predecessor
            rewrite_snapshot(&bytes, |_, ops| ops.push(ops[0].clone())),
            rewrite_snapshot(&bytes, |_, ops| ops[0].signature[0] ^= 1),
            rewrite_snapshot(&bytes, |_, ops| ops[0].domain_op = None),
            rewrite_snapshot(&bytes, |fields, _| fields[4] = empty_gate.to_vec()),
            rewrite_snapshot(&bytes, |fields, _| fields[2] = vec![1]), // seed without receipt
            {
                let mut value = empty.clone();
                let end = value.len();
                value[end - 4..].copy_from_slice(&u32::MAX.to_be_bytes());
                value
            },
        ];
        for (case, malformed) in malformed.iter().enumerate() {
            assert!(
                StudioEpoch::restore(malformed, &group, target(frame), owner.device_id()).is_err(),
                "tamper case {case}, frame={frame}"
            );
        }
        assert_eq!(unit.snapshot().unwrap(), bytes);
    }
}
