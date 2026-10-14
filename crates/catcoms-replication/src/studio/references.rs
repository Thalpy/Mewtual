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
        DocType::StudioObject => {
            // Vault intent records have logical scope, not current object-family authority.
            // Select the STATIC union by explicit operation/field discriminants, not a parser
            // fallback. Every selected grammar still checks all fields and canonical bytes.
            // This inspection grants no admission or fetch capability; both families' shared
            // title/recipe shapes have no CIDs, while art blob-bearing shapes stay art-only.
            let value = parse(&op.body)?;
            let obj = object(&value)?;
            let name = text(obj, "op")?;
            if matches!(name, "set_cell" | "clear_cell")
                || name == "set_header" && matches!(text(obj, "field")?, "bpm" | "bpb" | "steps")
            {
                ScoreOp::decode(&op.body)?;
                return Ok(None);
            }
            Ok(match FlipnoteOp::decode(&op.body)? {
                FlipnoteOp::InsertFrame { cid, .. }
                | FlipnoteOp::ReplaceFrame { cid, .. }
                | FlipnoteOp::SetExport { cid, .. } => Some(cid),
                _ => None,
            })
        }
        _ => Err(ReplError::EpochScope),
    }
}

impl StudioTarget {
    /// Typed reference inspection. Recipe hashes are not blobs, but an empty result is valid
    /// only after checking the complete score envelope against the independently held target.
    pub fn operation_blob_cid(
        &self,
        document: &LogicalDocument,
        op: &DomainOp,
    ) -> Result<Option<ContentId>, ReplError> {
        if *document != self.document(&document.server_id)? {
            return Err(ReplError::EpochScope);
        }
        match self {
            Self::Score { .. } => {
                ScoreOp::decode_domain(document, op)?;
                Ok(None)
            }
            Self::Index { .. } => {
                scope(document, op, document.doc_type)?;
                IndexOp::decode(&op.body)?;
                Ok(None)
            }
            Self::Flipnote { .. } => {
                // The family-less vault helper admits a static union. A held art target must
                // FIRST enforce its own complete grammar, not accidentally inherit that union.
                FlipnoteOp::decode_domain(document, op)?;
                operation_blob_cid(op)
            }
        }
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
    }
    out
}

impl StudioRecovery {
    /// Every reference needed by this retained/staged version, including superseded operations.
    /// The typed value is decoded/validated before this method; it grants no retirement permit.
    pub fn blob_cids(&self) -> Result<BTreeSet<ContentId>, ReplError> {
        let mut out = projection_cids(self.projection());
        for intent in self.operations().values() {
            // from_snapshot/snapshot already checked every operation against this family.
            if matches!(self.projection(), StudioProjection::Score(_)) {
                ScoreOp::decode_domain(self.projection().document(), &intent.operation)?;
            } else {
                out.extend(operation_blob_cid(&intent.operation)?);
            }
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

    #[test]
    fn typed_reference_inspection_does_not_inherit_the_vault_static_union() {
        let target = StudioTarget::Flipnote {
            channel: [7; 16],
            object: [9; 16],
        };
        let document = target.document(b"reference-union").unwrap();
        for body in [
            ScoreOp::ClearCell { step: 0, row: 0 },
            ScoreOp::SetCell {
                step: 0,
                row: 0,
                value: ScoreCell::Drum { pad: 0 },
            },
            ScoreOp::SetHeader(ScoreHeader::Bpm(120)),
        ] {
            let mut op = DomainOp {
                nonce: [1; 16],
                doc_type: document.doc_type,
                logical_key: document.logical_key.clone(),
                body: body.encode().unwrap(),
            };
            assert_eq!(operation_blob_cid(&op).unwrap(), None);
            assert!(target.operation_blob_cid(&document, &op).is_err());
            assert!(StudioTarget::Score {
                channel: [7; 16],
                object: [9; 16]
            }
            .operation_blob_cid(&document, &op)
            .is_ok());
            op.body.push(b' ');
            assert!(operation_blob_cid(&op).is_err());
        }
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
