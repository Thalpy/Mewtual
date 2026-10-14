//! Conservative byte-liveness, not circulation expiry. Retained history and replay instructions
//! can still need an overwritten/deleted frame. Never substitute the visible timeline here.

use super::*;
use std::collections::BTreeSet;

/// Extract a real blob reference from a bounded Studio envelope. Patch hashes and score ids are
/// not blob CIDs. This validates grammar/scope shape only, never authorizes an edit or fetch.
pub fn operation_blob_cid(op: &DomainOp) -> Result<Option<ContentId>, ReplError> {
    op.encode()?;
    if op.logical_key.len() != 16 {
        return Err(ReplError::EpochScope);
    }
    match op.doc_type {
        DocType::StudioIndex => {
            IndexOp::decode(&op.body)?;
            Ok(None)
        }
        DocType::StudioObject => Ok(match FlipnoteOp::decode(&op.body)? {
            FlipnoteOp::InsertFrame { cid, .. }
            | FlipnoteOp::ReplaceFrame { cid, .. }
            | FlipnoteOp::SetExport { cid, .. } => Some(cid),
            _ => None,
        }),
        _ => Err(ReplError::EpochScope),
    }
}

pub(super) fn projection_cids(projection: &StudioProjection) -> BTreeSet<ContentId> {
    let mut out = BTreeSet::new();
    if let StudioProjection::Flipnote(p) = projection {
        for entry in p.frames.values() {
            out.insert(entry.pixels.selected.value.cid);
            out.extend(entry.pixels.conflicts.iter().map(|v| v.value.cid));
            out.extend(entry.insertions.iter().map(|v| v.value.blob.cid));
        }
        for r in p.sound.records.values() {
            for v in std::iter::once(&r.selected).chain(&r.conflicts) {
                if let FlipnoteOp::SetExport { cid, .. } = v.value {
                    out.insert(cid);
                }
            }
        }
    }
    out
}

impl StudioRecovery {
    /// Every reference needed by this retained/staged version, including superseded operations.
    /// The typed value is decoded/validated before this method; it grants no retirement permit.
    pub fn blob_cids(&self) -> Result<BTreeSet<ContentId>, ReplError> {
        let mut out = projection_cids(self.projection());
        for intent in self.operations().values() {
            out.extend(operation_blob_cid(&intent.operation)?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalIntent, RecoveryReason};
    use catcoms_mls::{MlsDevice, ServerGroup};
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;
    use std::collections::BTreeMap;

    #[test]
    fn frame_sound_signed_epoch_restart_recovery_and_deleted_export_holds_use_existing_p1() {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let target = StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        };
        let mut unit = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        let descriptor = StudioPatch::new(&serde_json::json!({"v":1,"o":[{"w":0,"t":0,"c":0,"l":100}],"e":{"a":0,"d":0,"s":100,"r":500},"f":{"m":0,"c":18000,"q":0,"e":0},"l":{"r":1,"d":0,"t":0},"x":{"c":0,"d":0,"r":0}})).unwrap();
        let patch = descriptor.id();
        let mut rng = ChaCha20Rng::seed_from_u64(164);
        let mut operations = BTreeMap::new();
        for (n, op) in [
            FlipnoteOp::InsertFrame {
                frame: [1; 16],
                after: None,
                cid: [1; 32],
                bytes: 100,
            },
            FlipnoteOp::SetPatch { patch, descriptor },
            FlipnoteOp::SetSfx {
                sfx: [2; 16],
                frame: [1; 16],
                patch,
                note: 84,
                duration_ms: 600_000,
            },
            FlipnoteOp::SetHeader(FlipnoteHeader::Score(Some([4; 16]))),
            FlipnoteOp::SetExport {
                export: [5; 16],
                cid: [5; 32],
                bytes: 1000,
                expiry: StudioExpiry::At(0),
            },
            FlipnoteOp::RemoveExport { export: [5; 16] },
        ]
        .into_iter()
        .enumerate()
        {
            let operation = DomainOp {
                nonce: [n as u8; 16],
                doc_type: catcoms_wire::DocType::StudioObject,
                logical_key: vec![9; 16],
                body: op.encode().unwrap(),
            };
            unit.edit_or_reseal(&owner, &group, &mut rng, &operation, n as u64)
                .unwrap();
            operations.insert(
                operation.id(&owner.device_id()),
                LocalIntent {
                    author: owner.device_id(),
                    operation,
                },
            );
        }
        let expected = unit.projection().unwrap();
        let raw = unit.snapshot().unwrap();
        let restored = StudioEpoch::restore(&raw, &group, target, owner.device_id()).unwrap();
        assert_eq!(restored.projection().unwrap(), expected);
        assert_eq!(
            restored.blob_cids().unwrap(),
            BTreeSet::from([[1; 32], [5; 32]])
        );
        let snapshot = StudioRecovery::snapshot(
            &expected,
            None,
            RecoveryReason::Rewound,
            [0; 32],
            &operations,
        )
        .unwrap();
        assert_eq!(
            StudioRecovery::from_snapshot(&snapshot, expected.document(), [7; 16])
                .unwrap()
                .projection(),
            &expected
        );
        assert_eq!(
            StudioRecovery::inspect_vault_references(&snapshot, expected.document()).unwrap(),
            BTreeSet::from([[1; 32], [5; 32]])
        );
        assert!(
            !restored.blob_cids().unwrap().contains(&patch),
            "recipe identity is not a content reference"
        );
        assert!(StudioEpoch::restore(
            &raw,
            &group,
            StudioTarget::Flipnote {
                channel: [8; 16],
                object: [9; 16]
            },
            owner.device_id()
        )
        .is_err());
    }

    #[test]
    fn retained_sources_and_recovery_keep_sequentially_superseded_and_deleted_pixels() {
        let owner = MlsDevice::generate().unwrap();
        let group = ServerGroup::create(&owner).unwrap();
        let target = StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        };
        let logical = target.document(&group.group_id()).unwrap();
        let mut unit = StudioEpoch::new(&group, target, owner.device_id()).unwrap();
        let mut operations = std::collections::BTreeMap::new();
        let mut rng = ChaCha20Rng::seed_from_u64(313);
        for (n, body) in [
            FlipnoteOp::InsertFrame {
                frame: [1; 16],
                after: None,
                cid: [1; 32],
                bytes: 10,
            },
            FlipnoteOp::ReplaceFrame {
                frame: [1; 16],
                cid: [2; 32],
                bytes: 10,
            },
            FlipnoteOp::ReplaceFrame {
                frame: [1; 16],
                cid: [3; 32],
                bytes: 10,
            },
            FlipnoteOp::RemoveFrame { frame: [1; 16] },
        ]
        .into_iter()
        .enumerate()
        {
            let operation = DomainOp {
                nonce: [n as u8; 16],
                doc_type: logical.doc_type,
                logical_key: logical.logical_key.clone(),
                body: body.encode().unwrap(),
            };
            unit.edit_or_reseal(&owner, &group, &mut rng, &operation, n as u64)
                .unwrap();
            operations.insert(
                operation.id(&owner.device_id()),
                LocalIntent {
                    author: owner.device_id(),
                    operation,
                },
            );
        }
        let all = BTreeSet::from([[1; 32], [2; 32], [3; 32]]);
        assert!(
            !projection_cids(&unit.projection().unwrap()).contains(&[2; 32]),
            "sequential replacement is absent even from live alternatives"
        );
        assert_eq!(unit.blob_cids().unwrap(), all);
        let raw = unit.snapshot().unwrap();
        assert_eq!(
            StudioEpoch::inspect_vault_references(&raw, &group.group_id(), target)
                .unwrap()
                .1,
            all
        );
        let snapshot = StudioRecovery::snapshot(
            &unit.projection().unwrap(),
            None,
            RecoveryReason::Rewound,
            [0; 32],
            &operations,
        )
        .unwrap();
        assert_eq!(
            StudioRecovery::inspect_vault_references(&snapshot, &logical).unwrap(),
            all
        );
        let mut wrong = logical.clone();
        wrong.server_id.push(1);
        assert!(StudioRecovery::inspect_vault_references(&snapshot, &wrong).is_err());
        assert!(StudioEpoch::inspect_vault_references(
            &raw[..raw.len() - 1],
            &group.group_id(),
            target
        )
        .is_err());
    }

    #[test]
    fn reference_codec_distinguishes_blob_cids_from_patch_hashes_and_expiry() {
        let mut op = DomainOp {
            nonce: [1; 16],
            doc_type: DocType::StudioObject,
            logical_key: vec![9; 16],
            body: Vec::new(),
        };
        for expiry in [
            StudioExpiry::Unrecorded,
            StudioExpiry::Never,
            StudioExpiry::At(0),
        ] {
            op.body = FlipnoteOp::SetExport {
                export: [1; 16],
                cid: [8; 32],
                bytes: 100,
                expiry,
            }
            .encode()
            .unwrap();
            assert_eq!(operation_blob_cid(&op).unwrap(), Some([8; 32]));
        }
        op.body = FlipnoteOp::SetSfx {
            sfx: [1; 16],
            frame: [2; 16],
            patch: [8; 32],
            note: 60,
            duration_ms: 500,
        }
        .encode()
        .unwrap();
        assert_eq!(operation_blob_cid(&op).unwrap(), None);
        op.body = b"{}".to_vec();
        assert!(operation_blob_cid(&op).is_err());
        op.body = vec![0; 65 * 1024];
        assert!(operation_blob_cid(&op).is_err());
    }
}
