//! Studio's specialization of the EXISTING P1 recovery envelope. All alternatives, deletion
//! authors, hidden positions and excluded operation bodies live in the typed payload: the
//! generic bounded summary arrays cannot express a fifth conflict or two births of one id.

use catcoms_wire::Decoder;
use std::collections::BTreeMap;

use super::snapshot::*;
use super::*;
use crate::epoch::{MAX_EPOCH_OPERATIONS, MAX_RECOVERY_SNAPSHOT_BYTES};
use crate::{LocalIntent, RecoveryReason, RecoverySnapshot};

/// Detached typed Studio content. This is neither current-owner authority nor a durable save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioProjection {
    Index(StudioIndexProjection),
    Flipnote(Box<FlipnoteFrameProjection>),
}
impl StudioProjection {
    pub fn document(&self) -> &LogicalDocument {
        match self {
            Self::Index(p) => p.document(),
            Self::Flipnote(p) => p.document(),
        }
    }
    pub fn epoch(&self) -> u64 {
        match self {
            Self::Index(p) => p.epoch,
            Self::Flipnote(p) => p.epoch,
        }
    }
    pub fn channel(&self) -> ElementId {
        match self {
            Self::Index(p) => p
                .document()
                .logical_key
                .as_slice()
                .try_into()
                .expect("validated index channel"),
            Self::Flipnote(p) => p.channel,
        }
    }
    pub fn checkpoint(&self, close: [u8; 32]) -> Result<crate::CheckpointSeed, ReplError> {
        match self {
            Self::Index(p) => p.checkpoint(close),
            Self::Flipnote(p) => p.checkpoint(close),
        }
    }
    fn encode(&self, limit: usize) -> Result<Vec<u8>, ReplError> {
        match self {
            Self::Index(p) => p.encode_snapshot(limit),
            Self::Flipnote(p) => p.encode_snapshot(limit),
        }
    }
    fn decode(
        document: &LogicalDocument,
        channel: ElementId,
        epoch: u64,
        bytes: &[u8],
    ) -> Result<Self, ReplError> {
        match document.doc_type {
            DocType::StudioIndex if document.logical_key == channel => Ok(Self::Index(
                StudioIndexProjection::decode_snapshot(document, epoch, bytes)?,
            )),
            DocType::StudioObject => Ok(Self::Flipnote(Box::new(
                FlipnoteFrameProjection::decode_snapshot(document, channel, epoch, bytes)?,
            ))),
            _ => Err(ReplError::EpochScope),
        }
    }
}

/// Complete local historical evidence. Operation bodies are preserved even when their field was
/// subsequently overwritten. Only the original author may replay an intent; Restore authors NEW
/// operations as the restoring member. No recovery payload proves a receipt is current.
pub struct StudioRecovery {
    projection: StudioProjection,
    selecting_receipt: [u8; 32],
    operations: BTreeMap<[u8; 32], LocalIntent>,
}
impl std::fmt::Debug for StudioRecovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioRecovery")
            .field("epoch", &self.projection.epoch())
            .field("operations", &self.operations.len())
            .finish_non_exhaustive()
    }
}
impl StudioRecovery {
    pub fn projection(&self) -> &StudioProjection {
        &self.projection
    }
    pub fn operations(&self) -> &BTreeMap<[u8; 32], LocalIntent> {
        &self.operations
    }
    pub fn selecting_receipt(&self) -> [u8; 32] {
        self.selecting_receipt
    }

    /// Build bounded bytes only. Inputs must come from a checked frozen source/settlement; this
    /// constructor does not authenticate arbitrary provenance or retire an intent. Current-open
    /// preflight conservatively supplies ALL its operations, enough for a future full rewind.
    /// `base_close` identifies the SOURCE opening and is absent exactly in epoch zero. For
    /// `Rewound`, `selecting_receipt` is also SOURCE provenance: the source opening receipt hash
    /// (zero in epoch zero), never the destination receipt. This keeps identical frozen sources
    /// byte-identical across retargeting so recovery slots and eviction warnings are not reset.
    /// For `Excluded`, it identifies the receipt selecting the closure from this source.
    pub fn snapshot(
        projection: &StudioProjection,
        base_close: Option<[u8; 32]>,
        reason: RecoveryReason,
        selecting_receipt: [u8; 32],
        operations: &BTreeMap<[u8; 32], LocalIntent>,
    ) -> Result<RecoverySnapshot, ReplError> {
        if (projection.epoch() == 0) != base_close.is_none()
            || (reason == RecoveryReason::Rewound
                && projection.epoch() == 0
                && selecting_receipt != [0; 32])
        {
            return Err(ReplError::EpochScope);
        }
        if operations.len() > MAX_EPOCH_OPERATIONS {
            return Err(ReplError::EpochBound);
        }
        let mut out = RecoverySnapshot {
            doc_type: projection.document().doc_type,
            logical_key: projection.document().logical_key.clone(),
            epoch: projection.epoch(),
            base_close_record_hash: base_close,
            reason,
            projection: Vec::new(),
            tombstones: Vec::new(),
            elements: Vec::new(),
            conflicts: Vec::new(),
            applied_ops: operations.keys().copied().collect(),
        };
        let remaining = MAX_RECOVERY_SNAPSHOT_BYTES
            .checked_sub(out.encode()?.len())
            .ok_or(ReplError::EpochBound)?;
        let mut e = Writer::new(remaining);
        e.byte(1)?;
        e.bytes(&selecting_receipt)?;
        // Encode operations before projection, so the latter receives the exact remaining budget.
        e.count(operations.len())?;
        for (id, intent) in operations {
            validate_operation(projection.document(), id, intent)?;
            e.bytes(intent.author.as_bytes())?;
            e.bytes(&intent.operation.encode()?)?;
        }
        let prefix = e.finish();
        let payload_limit = remaining
            .checked_sub(prefix.len() + 4)
            .ok_or(ReplError::EpochBound)?;
        let payload = projection.encode(payload_limit)?;
        // Prefix and payload have already been bounded separately; no unbounded temporary copy.
        let mut bytes = prefix;
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&payload);
        out.projection = bytes;
        out.encode()?;
        Ok(out)
    }

    /// Validate the exact local envelope before exposing any Restore/Export content. Expected
    /// server/type/key/channel come from the authenticated vault record, never this payload.
    pub fn from_snapshot(
        snapshot: &RecoverySnapshot,
        document: &LogicalDocument,
        channel: ElementId,
    ) -> Result<Self, ReplError> {
        snapshot.encode()?;
        if snapshot.doc_type != document.doc_type
            || snapshot.logical_key != document.logical_key
            || (snapshot.epoch == 0) != snapshot.base_close_record_hash.is_none()
            || !snapshot.tombstones.is_empty()
            || !snapshot.elements.is_empty()
            || !snapshot.conflicts.is_empty()
        {
            return Err(ReplError::EpochScope);
        }
        let mut d = Decoder::new(&snapshot.projection);
        if d.get_u8().map_err(malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let selecting_receipt = array(&mut d)?;
        let n = count(&mut d, MAX_EPOCH_OPERATIONS)?;
        let mut operations = BTreeMap::new();
        for _ in 0..n {
            let author = DeviceId::from_bytes(array(&mut d)?);
            let bytes = d.get_bytes().map_err(malformed)?;
            if bytes.len() > MAX_DOMAIN_OP_BYTES {
                return Err(ReplError::EpochBound);
            }
            let operation = DomainOp::decode(bytes)?;
            let id = operation.id(&author);
            let intent = LocalIntent { author, operation };
            validate_operation(document, &id, &intent)?;
            if operations.last_key_value().is_some_and(|(p, _)| p >= &id) {
                return Err(ReplError::Malformed);
            }
            operations.insert(id, intent);
        }
        let projection = StudioProjection::decode(
            document,
            channel,
            snapshot.epoch,
            d.get_bytes().map_err(malformed)?,
        )?;
        d.finish().map_err(malformed)?;
        if operations.keys().copied().collect::<Vec<_>>() != snapshot.applied_ops {
            return Err(ReplError::Malformed);
        }
        // Full canonical reconstruction also checks every source/value/ordering invariant. Raw
        // field order and all outer metadata must agree, not only the selected visible pixels.
        if Self::snapshot(
            &projection,
            snapshot.base_close_record_hash,
            snapshot.reason,
            selecting_receipt,
            &operations,
        )? != *snapshot
        {
            return Err(ReplError::Malformed);
        }
        Ok(Self {
            projection,
            selecting_receipt,
            operations,
        })
    }
}

fn validate_operation(
    document: &LogicalDocument,
    id: &[u8; 32],
    intent: &LocalIntent,
) -> Result<(), ReplError> {
    if *id != intent.operation.id(&intent.author) {
        return Err(ReplError::Malformed);
    }
    match document.doc_type {
        DocType::StudioIndex => {
            IndexOp::decode_domain(document, &intent.operation, &intent.author)?;
        }
        DocType::StudioObject => match FlipnoteOp::decode_domain(document, &intent.operation)? {
            FlipnoteOp::InsertFrame { .. }
            | FlipnoteOp::RemoveFrame { .. }
            | FlipnoteOp::ReplaceFrame { .. }
            | FlipnoteOp::SetHeader(FlipnoteHeader::Title(_) | FlipnoteHeader::Fps(_)) => {}
            _ => return Err(ReplError::Malformed),
        },
        _ => return Err(ReplError::EpochScope),
    }
    Ok(())
}

pub(super) fn current_operations(
    doc: &crate::EncryptedDoc,
) -> Result<BTreeMap<[u8; 32], LocalIntent>, ReplError> {
    if doc.signed_log().len() > MAX_EPOCH_OPERATIONS {
        return Err(ReplError::EpochBound);
    }
    doc.signed_log()
        .iter()
        .map(|op| {
            let operation = op.parsed_domain_op()?.ok_or(ReplError::Malformed)?;
            Ok((
                operation.id(&op.author_device),
                LocalIntent {
                    author: op.author_device,
                    operation,
                },
            ))
        })
        .collect()
}
pub(super) fn preflight(
    projection: StudioProjection,
    operations: &BTreeMap<[u8; 32], LocalIntent>,
) -> Result<(), ReplError> {
    // The placeholder changes only fixed-width hashes. Encode actual Automerge seed framing,
    // then the complete recovery envelope with every current operation, not a JSON size estimate.
    projection.checkpoint([0; 32])?;
    let base = (projection.epoch() > 0).then_some([0; 32]);
    StudioRecovery::snapshot(
        &projection,
        base,
        RecoveryReason::Rewound,
        [0; 32],
        operations,
    )?;
    Ok(())
}
