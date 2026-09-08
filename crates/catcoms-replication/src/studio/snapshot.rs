//! Private, bounded codec shared by Studio checkpoints and local recovery projections.
//!
//! This is typed content, not a new receipt, transport, journal or persistence protocol. A seed
//! is admitted only by P1's expected-hash verifier; a recovery payload only by the sealed vault.

use std::collections::BTreeMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ScalarValue, ROOT};
use catcoms_crypto::DeviceId;
use catcoms_wire::{Decoder, Encoder};

use super::*;
use crate::checkpoint::am_error;
use crate::epoch::{MAX_EPOCH_OPERATIONS, MAX_RECOVERY_SNAPSHOT_BYTES};
use crate::{CheckpointSeed, MAX_CHECKPOINT_BYTES};

pub(super) const SEED_KEY: &str = "_studio/seed";
pub(super) const MAX_VALUES: usize = 3 * MAX_EPOCH_OPERATIONS + 8192;
pub(super) const CONFLICT_FIELDS: usize = 1024;
pub(super) const CONFLICT_VALUES: usize = 4;

pub(super) fn malformed(_: impl std::fmt::Display) -> ReplError {
    ReplError::Malformed
}

/// Charge before extending the buffer, including length prefixes. Public projection fields can
/// be mutated by callers, so encoding must not first allocate an arbitrarily large whole payload.
pub(super) struct Writer {
    encoder: Option<Encoder>,
    written: usize,
    limit: usize,
}
impl Writer {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            encoder: Some(Encoder::new()),
            written: 0,
            limit,
        }
    }
    /// Run the identical value/count/aggregate checks without allocating an output buffer.
    /// Call this before cloning caller-mutable content for topology checks or compaction.
    pub(super) fn measure(limit: usize) -> Self {
        Self {
            encoder: None,
            written: 0,
            limit,
        }
    }
    fn charge(&mut self, size: usize) -> Result<(), ReplError> {
        if size > self.limit.saturating_sub(self.written) {
            return Err(ReplError::EpochBound);
        }
        self.written += size;
        Ok(())
    }
    pub(super) fn byte(&mut self, value: u8) -> Result<(), ReplError> {
        self.charge(1)?;
        if let Some(e) = &mut self.encoder {
            e.put_u8(value);
        }
        Ok(())
    }
    pub(super) fn integer(&mut self, value: u64) -> Result<(), ReplError> {
        self.charge(8)?;
        if let Some(e) = &mut self.encoder {
            e.put_u64(value);
        }
        Ok(())
    }
    pub(super) fn count(&mut self, value: usize) -> Result<(), ReplError> {
        if value > MAX_VALUES {
            return Err(ReplError::EpochBound);
        }
        self.charge(4)?;
        if let Some(e) = &mut self.encoder {
            e.put_u32(value as u32);
        }
        Ok(())
    }
    pub(super) fn bytes(&mut self, value: &[u8]) -> Result<(), ReplError> {
        self.charge(value.len().checked_add(4).ok_or(ReplError::EpochBound)?)?;
        if let Some(e) = &mut self.encoder {
            e.put_bytes(value).map_err(malformed)?;
        }
        Ok(())
    }
    pub(super) fn optional<const N: usize>(
        &mut self,
        value: Option<[u8; N]>,
    ) -> Result<(), ReplError> {
        self.byte(u8::from(value.is_some()))?;
        if let Some(value) = value {
            self.bytes(&value)?;
        }
        Ok(())
    }
    pub(super) fn finish(self) -> Vec<u8> {
        self.encoder.map_or_else(Vec::new, Encoder::finish)
    }
}

pub(super) fn count(d: &mut Decoder<'_>, max: usize) -> Result<usize, ReplError> {
    let n = d.get_u32().map_err(malformed)? as usize;
    if n > max.min(MAX_VALUES) {
        return Err(ReplError::EpochBound);
    }
    Ok(n)
}
pub(super) fn array<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], ReplError> {
    d.get_bytes()
        .map_err(malformed)?
        .try_into()
        .map_err(|_| ReplError::Malformed)
}
pub(super) fn optional<const N: usize>(d: &mut Decoder<'_>) -> Result<Option<[u8; N]>, ReplError> {
    match d.get_u8().map_err(malformed)? {
        0 => Ok(None),
        1 => Ok(Some(array(d)?)),
        _ => Err(ReplError::Malformed),
    }
}
pub(super) fn derived(document: &LogicalDocument, author: &DeviceId, nonce: [u8; 16]) -> [u8; 32] {
    crate::epoch::hash_parts(
        "catcoms-domain-op:v1",
        &[&document.logical_key, author.as_bytes(), &nonce],
    )
}

/// The seed records values and provenance, not an old operation log. IDs are derived from the
/// supplied logical scope, full identity and nonce; they are never transported as trusted ids.
pub(super) trait SnapshotValue: Sized {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError>;
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError>;
}
impl SnapshotValue for String {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        title_bound(self)?;
        e.bytes(self.as_bytes())
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        let s = d.get_str().map_err(malformed)?;
        title_bound(s)?;
        Ok(s.into())
    }
}
impl SnapshotValue for StudioExpiry {
    fn put(&self, e: &mut Writer) -> Result<(), ReplError> {
        match self {
            Self::Unrecorded => e.byte(0),
            Self::Never => e.byte(1),
            Self::At(n) => {
                integer_bound(*n)?;
                e.byte(2)?;
                e.integer(*n)
            }
        }
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, ReplError> {
        match d.get_u8().map_err(malformed)? {
            0 => Ok(Self::Unrecorded),
            1 => Ok(Self::Never),
            2 => {
                let n = d.get_u64().map_err(malformed)?;
                integer_bound(n)?;
                Ok(Self::At(n))
            }
            _ => Err(ReplError::Malformed),
        }
    }
}

/// Selected values never change just to meet the conflict limit. Extra alternatives and fields
/// remain in the complete recovery projection. A field with alternatives consumes one slot.
pub(super) struct ConflictBudget(pub(super) usize);
impl ConflictBudget {
    pub(super) fn retained(&mut self, alternatives: usize) -> usize {
        if alternatives == 0 || self.0 == 0 {
            return 0;
        }
        self.0 -= 1;
        alternatives.min(CONFLICT_VALUES - 1)
    }
}

pub(super) fn build(
    document: &LogicalDocument,
    next: u64,
    close: [u8; 32],
    mut fields: BTreeMap<String, ScalarValue>,
    payload: Vec<u8>,
) -> Result<CheckpointSeed, ReplError> {
    if payload.len() > MAX_CHECKPOINT_BYTES {
        return Err(ReplError::EpochBound);
    }
    fields.insert(SEED_KEY.into(), ScalarValue::Bytes(payload));
    CheckpointSeed::build(document, next, close, |doc: &mut AutoCommit| {
        for (key, value) in fields {
            doc.put(ROOT, key, value).map_err(am_error)?;
        }
        Ok(())
    })
}

pub(super) fn payload_header(
    e: &mut Writer,
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
) -> Result<(), ReplError> {
    e.byte(1)?;
    e.bytes(&document.server_id)?;
    e.integer(u64::from(document.doc_type.tag()))?;
    e.bytes(&document.logical_key)?;
    e.bytes(&channel)?;
    e.integer(epoch)
}
pub(super) fn check_header(
    d: &mut Decoder<'_>,
    document: &LogicalDocument,
    channel: ElementId,
    epoch: u64,
) -> Result<(), ReplError> {
    if d.get_u8().map_err(malformed)? != 1
        || d.get_bytes().map_err(malformed)? != document.server_id
        || d.get_u64().map_err(malformed)? != u64::from(document.doc_type.tag())
        || d.get_bytes().map_err(malformed)? != document.logical_key
        || array::<16>(d)? != channel
        || d.get_u64().map_err(malformed)? != epoch
    {
        return Err(ReplError::EpochScope);
    }
    Ok(())
}

pub(super) fn payload_bound(bytes: &[u8]) -> Result<(), ReplError> {
    if bytes.len() > MAX_RECOVERY_SNAPSHOT_BYTES {
        return Err(ReplError::EpochBound);
    }
    Ok(())
}
