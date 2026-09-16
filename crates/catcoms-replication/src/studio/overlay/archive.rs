//! Lossless local evidence for a disposed draft. This is a backup format, never authority:
//! there is no path from these bytes to a basis, an overlay, a verified receipt or a source,
//! and nothing in this module can construct one. Building an archive deliberately needs no
//! typed reconstruction, so a branch that survives structural validation but cannot be replayed
//! still has a complete lossless export and a preserving disposal available to it.
use super::*;
use crate::epoch::{MAX_INTENT_BYTES_PER_DOCUMENT, MAX_RECEIPT_BYTES};
use crate::LocalIntent;

/// Fixed per-entry cost excluding the operation body: id, envelope, sequence, timestamp, author
/// and the body's own length prefix, with this encoder's framing on each byte string.
/// `draft_archive_constants_cover_the_real_encoder` measures the true value and fails if it ever
/// exceeds this; the margin is headroom for a future field, not a guess about the current one.
pub(in crate::studio) const ENTRY_OVERHEAD_BYTES: usize = 160;
/// Version, provenance and replayable bytes, the document at its largest admissible shape, the
/// target, four 32-byte digests, the generation, the optional unconfirmed triple, and framing.
/// Measured against the real encoder by the same test.
pub(in crate::studio) const HEADER_BYTES: usize = 2048;

/// Bound on the canonical payload, from the field maxima rather than from the intent record's
/// cap, which the archive is not obliged to obey: it is its own record kind. The seed and
/// operation terms cannot both be saturated by a branch that fit a live record, so the reachable
/// maximum is lower, but the bound must not depend on that coincidence.
///
/// The two overhead terms above are padding constants, not schema-derived expressions. What
/// makes them trustworthy is not the arithmetic here but the measurement in
/// `draft_archive_constants_cover_the_real_encoder`, which encodes real archives, recovers the
/// actual fixed costs, extrapolates each variable field to its documented maximum and asserts
/// the result still fits. A future field or framing change fails that test rather than silently
/// narrowing this bound.
pub const MAX_STUDIO_DRAFT_ARCHIVE_BYTES: usize = HEADER_BYTES
    + MAX_RECEIPT_BYTES
    + MAX_CHECKPOINT_BYTES
    + MAX_INTENT_BYTES_PER_DOCUMENT
    + MAX_STUDIO_OVERLAY_OPS * ENTRY_OVERHEAD_BYTES;

/// Where an archived branch's base came from. Carried in the archive because it changes what
/// the base means, never because it grants anything: neither variant is authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayProvenance {
    /// An installed Closing source, its matching saved signed close and observed tenure.
    Closing,
    /// An authenticated member's unconfirmed preview checkpoint. No installed source and no
    /// verified owner authority.
    Unconfirmed {
        provider: DeviceId,
        observed_mls_epoch: u64,
        observed_at_ms: u64,
    },
}
impl StudioOverlayProvenance {
    fn tag(&self) -> u8 {
        match self {
            Self::Closing => 0,
            Self::Unconfirmed { .. } => 1,
        }
    }
}

/// One archived operation, complete: the stable id, the accepted envelope, its saved position
/// and timestamp, and the original author and body.
///
/// The envelope is carried deliberately, and the earlier reasoning for omitting it was wrong.
/// `DomainOp::id` hashes the logical key, author and nonce and **not the body**, so an id alone
/// binds identity, not content: a different body under the same nonce keeps the same id. The
/// live branch does not rely on the id either, it compares `envelope` in `checked_entries`. An
/// archive that dropped it could decode an entry whose body was not the accepted one, which is
/// unacceptable in the artefact whose entire job is to be the surviving evidence of that body.
#[derive(Clone, PartialEq, Eq)]
struct ArchiveEntry {
    id: [u8; 32],
    envelope: [u8; 32],
    sequence: u64,
    ts: u64,
    author: DeviceId,
    operation: DomainOp,
}

/// A complete local draft, detached from the vault. Deliberately not `Clone`: an archive is
/// evidence of one disposal and copies of it invite treating it as a live object.
pub struct StudioDraftArchive {
    provenance: StudioOverlayProvenance,
    replayable: bool,
    document: LogicalDocument,
    target: StudioTarget,
    author: DeviceId,
    basis: [u8; 32],
    branch: [u8; 32],
    content: [u8; 32],
    generation: u64,
    receipt: Receipt,
    seed: Vec<u8>,
    entries: Vec<ArchiveEntry>,
}

impl std::fmt::Debug for StudioDraftArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Archived operation bodies and the seed are private vault content.
        f.debug_struct("StudioDraftArchive")
            .field("accepted", &self.entries.len())
            .field("replayable", &self.replayable)
            .finish_non_exhaustive()
    }
}

impl StudioDraftArchive {
    /// Build from a structurally decoded branch and its ledger. No typed reconstruction runs
    /// here: entries come from the branch's own saved order and envelopes from the ledger, so
    /// a non-replayable branch archives exactly as well as a replayable one. `replayable`
    /// records what a separate reconstruction attempt found; it is a label, not a gate.
    #[allow(clippy::too_many_arguments)]
    pub fn from_branch(
        overlay: &StudioOverlay,
        ledger: &IntentLedger,
        provenance: StudioOverlayProvenance,
        replayable: bool,
        branch: [u8; 32],
        content: [u8; 32],
        generation: u64,
    ) -> Result<Self, ReplError> {
        let entries = overlay
            .checked_entries(ledger)?
            .into_iter()
            .map(|(entry, intent)| ArchiveEntry {
                id: entry.id,
                // The exact value `checked_entries` has just verified against this intent.
                envelope: entry.envelope,
                sequence: entry.sequence,
                ts: entry.ts,
                author: intent.author,
                operation: intent.operation.clone(),
            })
            .collect();
        let out = Self {
            provenance,
            replayable,
            document: ledger.document().clone(),
            target: overlay.target(),
            author: overlay.author(),
            basis: overlay.basis(),
            branch,
            content,
            generation,
            receipt: overlay.receipt().clone(),
            seed: overlay.seed().to_vec(),
            entries,
        };
        // Refuse at construction rather than at the writer, so an over-size branch never
        // reaches a durable transaction that would have to unwind.
        out.encode()?;
        Ok(out)
    }

    pub fn provenance(&self) -> StudioOverlayProvenance {
        self.provenance
    }
    pub fn replayable(&self) -> bool {
        self.replayable
    }
    pub fn document(&self) -> &LogicalDocument {
        &self.document
    }
    pub fn target(&self) -> StudioTarget {
        self.target
    }
    pub fn author(&self) -> DeviceId {
        self.author
    }
    pub fn basis(&self) -> [u8; 32] {
        self.basis
    }
    pub fn branch(&self) -> [u8; 32] {
        self.branch
    }
    pub fn content(&self) -> [u8; 32] {
        self.content
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn accepted(&self) -> usize {
        self.entries.len()
    }

    /// The conservative reference set this archive keeps alive: the seed-derived base CIDs plus
    /// every archived operation's CID. This is the same pair the live-branch inventory arm
    /// collects, which is what makes a preserving disposal leave nothing reclaimable that the
    /// branch was protecting.
    pub fn blob_cids(&self) -> Result<BTreeSet<ContentId>, ReplError> {
        let seed = UnconfirmedStudioSeed::parse(self.target, &self.receipt, &self.seed)?;
        let mut cids = crate::studio::references::projection_cids(seed.projection());
        for entry in &self.entries {
            if let Some(cid) = references::operation_blob_cid(&entry.operation)? {
                cids.insert(cid);
            }
        }
        Ok(cids)
    }

    /// Canonical bytes. The caller seals these; this format carries no signature and no key
    /// material of its own.
    pub fn encode(&self) -> Result<Vec<u8>, ReplError> {
        self.validate()?;
        let mut e = Encoder::new();
        e.put_u8(1);
        e.put_u8(self.provenance.tag());
        e.put_u8(u8::from(self.replayable));
        put(&mut e, &self.document.server_id)?;
        e.put_u16(self.document.doc_type.tag());
        put(&mut e, &self.document.logical_key)?;
        put_target(&mut e, self.target)?;
        put(&mut e, self.author.as_bytes())?;
        for digest in [&self.basis, &self.branch, &self.content] {
            put(&mut e, digest)?;
        }
        e.put_u64(self.generation);
        put(&mut e, &self.receipt.encode())?;
        put(&mut e, &self.seed)?;
        if let StudioOverlayProvenance::Unconfirmed {
            provider,
            observed_mls_epoch,
            observed_at_ms,
        } = self.provenance
        {
            put(&mut e, provider.as_bytes())?;
            e.put_u64(observed_mls_epoch);
            e.put_u64(observed_at_ms);
        }
        e.put_u32(self.entries.len() as u32);
        for entry in &self.entries {
            put(&mut e, &entry.id)?;
            put(&mut e, &entry.envelope)?;
            e.put_u64(entry.sequence);
            e.put_u64(entry.ts);
            put(&mut e, entry.author.as_bytes())?;
            put(&mut e, &entry.operation.encode()?)?;
        }
        let bytes = e.finish();
        if bytes.len() > MAX_STUDIO_DRAFT_ARCHIVE_BYTES {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }

    /// Authenticated local bytes only. Unknown versions, trailing data, duplicate ids,
    /// noncanonical order and any re-encoding difference all reject, exactly as the vault
    /// extension decoder does. This cannot return a basis, an overlay or a verified receipt.
    pub fn decode(bytes: &[u8]) -> Result<Self, ReplError> {
        if bytes.len() > MAX_STUDIO_DRAFT_ARCHIVE_BYTES {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if byte(&mut d)? != 1 {
            return Err(ReplError::Malformed);
        }
        let provenance_tag = byte(&mut d)?;
        let replayable = match byte(&mut d)? {
            0 => false,
            1 => true,
            _ => return Err(ReplError::Malformed),
        };
        let server_id = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let doc_type = DocType::from_tag(d.get_u16().map_err(|_| ReplError::Malformed)?)
            .ok_or(ReplError::Malformed)?;
        let logical_key = d.get_bytes().map_err(|_| ReplError::Malformed)?.to_vec();
        let document = LogicalDocument::new(server_id, doc_type, logical_key)?;
        let target = get_target(&mut d)?;
        let author = DeviceId::from_bytes(fixed(&mut d)?);
        let basis = fixed(&mut d)?;
        let branch = fixed(&mut d)?;
        let content = fixed(&mut d)?;
        let generation = number(&mut d)?;
        let receipt = Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?;
        let seed = d.get_bytes().map_err(|_| ReplError::Malformed)?;
        if seed.len() > MAX_CHECKPOINT_BYTES {
            return Err(ReplError::EpochBound);
        }
        let seed = seed.to_vec();
        let provenance = match provenance_tag {
            0 => StudioOverlayProvenance::Closing,
            1 => StudioOverlayProvenance::Unconfirmed {
                provider: DeviceId::from_bytes(fixed(&mut d)?),
                observed_mls_epoch: number(&mut d)?,
                observed_at_ms: number(&mut d)?,
            },
            _ => return Err(ReplError::Malformed),
        };
        let count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if count == 0 || count > MAX_STUDIO_OVERLAY_OPS {
            return Err(ReplError::EpochBound);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(ArchiveEntry {
                id: fixed(&mut d)?,
                envelope: fixed(&mut d)?,
                sequence: number(&mut d)?,
                ts: number(&mut d)?,
                author: DeviceId::from_bytes(fixed(&mut d)?),
                operation: DomainOp::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?,
            });
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        let out = Self {
            provenance,
            replayable,
            document,
            target,
            author,
            basis,
            branch,
            content,
            generation,
            receipt,
            seed,
            entries,
        };
        if out.encode()?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(out)
    }

    /// Shared by the encoder and the decoder, so neither can accept a shape the other rejects.
    fn validate(&self) -> Result<(), ReplError> {
        if self.target.document(&self.document.server_id)? != self.document
            || self.receipt.document != self.document
        {
            return Err(ReplError::EpochScope);
        }
        if self.seed.len() > MAX_CHECKPOINT_BYTES {
            return Err(ReplError::EpochBound);
        }
        if self.entries.is_empty() || self.entries.len() > MAX_STUDIO_OVERLAY_OPS {
            return Err(ReplError::EpochBound);
        }
        let mut seen = BTreeSet::new();
        let mut operation_bytes = 0usize;
        for (index, entry) in self.entries.iter().enumerate() {
            // Saved order is the authored order. Hash order is not, and a renumbered or
            // duplicated manifest must not survive a round trip.
            if entry.sequence != index as u64 + 1 || !seen.insert(entry.id) {
                return Err(ReplError::IntentConflict);
            }
            // Two separate bindings, because neither alone is enough. The id covers the
            // logical key, author and nonce, so it fixes WHICH accepted operation this is;
            // the envelope covers the author and the complete encoded body, so it fixes WHAT
            // that operation says. `DomainOp::id` does not hash the body, so without the
            // envelope a different body under the same nonce would decode unchallenged.
            if entry.operation.id(&entry.author) != entry.id
                || super::envelope(&LocalIntent {
                    author: entry.author,
                    operation: entry.operation.clone(),
                })? != entry.envelope
            {
                return Err(ReplError::IntentConflict);
            }
            // Invariants `from_branch` already gets from `checked_entries` and the ledger.
            // Restated here so a decoded archive stands on its own rather than on the
            // provenance of the path that happened to build it.
            if entry.author != self.author
                || entry.operation.doc_type != self.document.doc_type
                || entry.operation.logical_key != self.document.logical_key
            {
                return Err(ReplError::EpochScope);
            }
            integer_bound(entry.ts)?;
            operation_bytes = operation_bytes
                .checked_add(entry.operation.encode()?.len())
                .ok_or(ReplError::EpochBound)?;
        }
        if operation_bytes > MAX_INTENT_BYTES_PER_DOCUMENT {
            return Err(ReplError::EpochBound);
        }
        Ok(())
    }
}

// The vault extension's encoder helpers are private to `handoff`, and this is a different
// format that must not silently inherit changes to that one. They are duplicated here rather
// than promoted to the shared parent so that no edit to the extension codec can alter the
// archive's bytes by accident. Collapsing them would couple two formats that have no reason
// to agree; if they are ever promoted, the target encodings must stay independently tested.
fn put(e: &mut Encoder, bytes: &[u8]) -> Result<(), ReplError> {
    e.put_bytes(bytes)
        .map(|_| ())
        .map_err(|_| ReplError::EpochBound)
}
fn byte(d: &mut Decoder<'_>) -> Result<u8, ReplError> {
    d.get_u8().map_err(|_| ReplError::Malformed)
}
fn number(d: &mut Decoder<'_>) -> Result<u64, ReplError> {
    d.get_u64().map_err(|_| ReplError::Malformed)
}
fn put_target(e: &mut Encoder, target: StudioTarget) -> Result<(), ReplError> {
    e.put_u8(match target {
        StudioTarget::Index { .. } => 0,
        StudioTarget::Flipnote { .. } => 1,
    });
    put(e, &target.channel())?;
    if let StudioTarget::Flipnote { object, .. } = target {
        put(e, &object)?;
    }
    Ok(())
}
fn get_target(d: &mut Decoder<'_>) -> Result<StudioTarget, ReplError> {
    let tag = byte(d)?;
    let channel = fixed(d)?;
    match tag {
        0 => Ok(StudioTarget::Index { channel }),
        1 => Ok(StudioTarget::Flipnote {
            channel,
            object: fixed(d)?,
        }),
        _ => Err(ReplError::Malformed),
    }
}
