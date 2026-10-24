//! Vault-local Closing drafts. No signed log, mutable epoch gate or authority capability
//! escapes. Restoring local data cannot create the fresh basis required for an append.
use super::*;
use crate::{IntentLedger, LocalIntent, Receipt, MAX_CHECKPOINT_BYTES};
use automerge::{transaction::Transactable, ActorId, AutoCommit, ReadDoc, ROOT};
use catcoms_wire::{Decoder, Encoder};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_STUDIO_OVERLAY_OPS: usize = 256;
const MAX_METADATA: usize = 64 * 1024;
const MAX_EXTENSION: usize = MAX_CHECKPOINT_BYTES + MAX_METADATA;

pub(in crate::studio) mod archive;
mod handoff;
pub use archive::{StudioDraftArchive, StudioOverlayProvenance, MAX_STUDIO_DRAFT_ARCHIVE_BYTES};
pub use handoff::{
    StudioHandoffAuthority, StudioHandoffCandidate, StudioHandoffEvidence, StudioHandoffOutcome,
    StudioHandoffSigning, StudioOverlaySave, StudioOverlayState,
};

#[derive(Clone)]
struct BasisData {
    target: StudioTarget,
    author: DeviceId,
    source_id: u128,
    source_version: [u8; 32],
    receipt: Receipt,
    seed: Vec<u8>,
}

/// Minted only by a checked Closing source/settlement plan, never by a vault decoder.
pub struct StudioClosingOverlayBasis(BasisData);
impl std::fmt::Debug for StudioClosingOverlayBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioClosingOverlayBasis { .. }")
    }
}
impl StudioClosingOverlayBasis {
    pub(super) fn from_settlement(
        target: StudioTarget,
        author: DeviceId,
        source_id: u128,
        source_version: [u8; 32],
        plan: &StudioSettlementPlan,
    ) -> Self {
        Self(BasisData {
            target,
            author,
            source_id,
            source_version,
            receipt: plan.receipt().clone(),
            seed: plan.checkpoint().bytes().to_vec(),
        })
    }
    /// Local exact-request fence, not receipt or membership authority.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.0.fingerprint()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    id: [u8; 32],
    envelope: [u8; 32],
    sequence: u64,
    ts: u64,
}

/// Local acceptance metadata. Full original envelopes remain in the shared intent ledger.
#[derive(Clone)]
pub struct StudioOverlay {
    base: BasisData,
    entries: Vec<Entry>,
    next_sequence: u64,
}
impl std::fmt::Debug for StudioOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioOverlay")
            .field("accepted", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// Content for LOCAL draft display only, deliberately distinct from an installed epoch.
pub struct StudioLocalDraft {
    projection: StudioProjection,
    basis: [u8; 32],
    accepted: usize,
}
impl std::fmt::Debug for StudioLocalDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioLocalDraft")
            .field("accepted", &self.accepted)
            .finish_non_exhaustive()
    }
}
impl StudioLocalDraft {
    pub fn projection(&self) -> &StudioProjection {
        &self.projection
    }
    pub fn basis(&self) -> [u8; 32] {
        self.basis
    }
    pub fn accepted(&self) -> usize {
        self.accepted
    }
}

fn fixed<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], ReplError> {
    d.get_bytes()
        .map_err(|_| ReplError::Malformed)?
        .try_into()
        .map_err(|_| ReplError::Malformed)
}
fn envelope(intent: &LocalIntent) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-envelope/v1");
    hash.update(intent.author.as_bytes());
    hash.update(&intent.operation.encode()?);
    Ok(*hash.finalize().as_bytes())
}
impl BasisData {
    fn encode(&self, e: &mut Encoder) -> Result<(), ReplError> {
        e.put_u8(match self.target {
            StudioTarget::Index { .. } => 0,
            StudioTarget::Flipnote { .. } => 1,
        });
        e.put_bytes(&self.target.channel())
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(self.author.as_bytes())
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(&self.source_id.to_be_bytes())
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(&self.source_version)
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(&self.receipt.encode())
            .map_err(|_| ReplError::EpochBound)?;
        e.put_bytes(&self.seed).map_err(|_| ReplError::EpochBound)?;
        Ok(())
    }
    fn fingerprint(&self) -> [u8; 32] {
        let mut e = Encoder::new();
        self.encode(&mut e).expect("bounded private basis");
        blake3::derive_key("catcoms/studio-closing-overlay-basis/v1", &e.finish())
    }
    fn graph(&self) -> Result<(AutoCommit, StudioProjection), ReplError> {
        let (mut doc, projection) =
            UnconfirmedStudioSeed::parse(self.target, &self.receipt, &self.seed)?
                .into_local_graph();
        doc.set_actor(ActorId::from(self.author.as_bytes().to_vec()));
        Ok((doc, projection))
    }
}
impl StudioOverlay {
    pub(in crate::studio) fn receipt(&self) -> &Receipt {
        &self.base.receipt
    }
    pub(in crate::studio) fn seed(&self) -> &[u8] {
        &self.base.seed
    }
    pub(in crate::studio) fn base_projection(&self) -> Result<StudioProjection, ReplError> {
        Ok(self.base.graph()?.1)
    }
    pub(in crate::studio) fn ordered<'a>(
        &self,
        ledger: &'a IntentLedger,
    ) -> Result<Vec<(&'a LocalIntent, u64)>, ReplError> {
        Ok(self
            .checked_entries(ledger)?
            .into_iter()
            .map(|(e, i)| (i, e.ts))
            .collect())
    }
    pub fn new(basis: &StudioClosingOverlayBasis) -> Self {
        Self {
            base: basis.0.clone(),
            entries: Vec::new(),
            next_sequence: 1,
        }
    }
    pub fn basis(&self) -> [u8; 32] {
        self.base.fingerprint()
    }
    pub fn target(&self) -> StudioTarget {
        self.base.target
    }
    pub fn author(&self) -> DeviceId {
        self.base.author
    }
    pub fn contains(&self, id: &[u8; 32]) -> bool {
        self.entries.iter().any(|e| &e.id == id)
    }
    /// Exact saved acceptance can be acknowledged after source replacement, without minting
    /// a fresh basis or allowing an append. Full envelope equality is required independently.
    pub fn exact_retry(&self, basis: [u8; 32], intent: &LocalIntent) -> Result<bool, ReplError> {
        let id = intent.operation.id(&intent.author);
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return Ok(false);
        };
        if self.basis() != basis
            || self.base.author != intent.author
            || entry.envelope != envelope(intent)?
        {
            return Err(ReplError::IntentConflict);
        }
        Ok(true)
    }
    /// Caller persists this metadata and the matching ledger together before acknowledging.
    /// Staging is rollback safe; no acceptance is appended when typed reconstruction fails.
    pub fn append(
        &mut self,
        basis: &StudioClosingOverlayBasis,
        ledger: &IntentLedger,
        id: [u8; 32],
        ts: u64,
    ) -> Result<StudioLocalDraft, ReplError> {
        if self.basis() != basis.fingerprint() {
            return Err(ReplError::EpochScope);
        }
        if self.entries.len() >= MAX_STUDIO_OVERLAY_OPS {
            return Err(ReplError::EpochBound);
        }
        if self.contains(&id) {
            return Err(ReplError::IntentConflict);
        }
        integer_bound(ts)?;
        let intent = ledger
            .pending()
            .find(|(key, _)| **key == id)
            .map(|(_, i)| i)
            .ok_or(ReplError::Malformed)?;
        if intent.author != self.base.author {
            return Err(ReplError::EpochAuthority);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(ReplError::EpochBound)?;
        let mut staged = self.clone();
        staged.entries.push(Entry {
            id,
            envelope: envelope(intent)?,
            sequence: self.next_sequence,
            ts,
        });
        staged.next_sequence = next;
        let view = staged.read(ledger)?;
        *self = staged;
        Ok(view)
    }
    fn checked_entries<'a>(
        &self,
        ledger: &'a IntentLedger,
    ) -> Result<Vec<(&Entry, &'a LocalIntent)>, ReplError> {
        if ledger.document() != &self.base.receipt.document
            || self.entries.is_empty()
            || self.entries.len() > MAX_STUDIO_OVERLAY_OPS
            || self.next_sequence != self.entries.len() as u64 + 1
        {
            return Err(ReplError::Malformed);
        }
        let pending: BTreeMap<_, _> = ledger.pending().collect();
        let mut seen = BTreeSet::new();
        let mut out = Vec::with_capacity(self.entries.len());
        for (index, entry) in self.entries.iter().enumerate() {
            let intent = *pending.get(&entry.id).ok_or(ReplError::Malformed)?;
            if !seen.insert(entry.id)
                || entry.sequence != index as u64 + 1
                || intent.author != self.base.author
                || entry.envelope != envelope(intent)?
            {
                return Err(ReplError::IntentConflict);
            }
            integer_bound(entry.ts)?;
            out.push((entry, intent));
        }
        Ok(out)
    }
    pub fn read(&self, ledger: &IntentLedger) -> Result<StudioLocalDraft, ReplError> {
        let entries = self.checked_entries(ledger)?;
        let (mut doc, mut projection) = self.base.graph()?;
        let mut operations = BTreeMap::new();
        for (entry, intent) in entries {
            let domain = &intent.operation;
            self.base
                .target
                .local_policy(&projection, domain, &intent.author)?;
            let prepared = self.base.target.prepare_local_write(
                &projection,
                domain,
                &intent.author,
                entry.ts,
            )?;
            let mut staged = doc.clone();
            let marker = crate::doc::domain_marker_key(&entry.id);
            if staged
                .get(ROOT, &marker)
                .map_err(crate::checkpoint::am_error)?
                .is_some()
            {
                return Err(ReplError::IntentConflict);
            }
            prepared
                .write(&mut staged)
                .map_err(crate::checkpoint::am_error)?;
            staged
                .put(ROOT, marker, 1u64)
                .map_err(crate::checkpoint::am_error)?;
            staged.commit();
            let change = staged.get_last_local_change().ok_or(ReplError::NoChange)?;
            self.base.target.validate(
                ledger.document(),
                projection.epoch(),
                domain,
                &change,
                &doc,
            )?;
            projection = self
                .base
                .target
                .read(ledger.document(), projection.epoch(), &staged)?;
            operations.insert(entry.id, intent.clone());
            recovery::preflight(projection.clone(), &operations)?;
            doc = staged;
        }
        Ok(StudioLocalDraft {
            projection,
            basis: self.basis(),
            accepted: self.entries.len(),
        })
    }
    /// Seed-only references supplement (never replace) all original ledger operation CIDs.
    pub fn base_blob_cids(&self) -> Result<BTreeSet<ContentId>, ReplError> {
        Ok(references::projection_cids(&self.base.graph()?.1))
    }
    pub fn encode_vault(&self, ledger: &IntentLedger) -> Result<Vec<u8>, ReplError> {
        self.checked_entries(ledger)?;
        let mut e = Encoder::new();
        e.put_u8(1);
        self.base.encode(&mut e)?;
        e.put_u64(self.next_sequence);
        e.put_u32(self.entries.len() as u32);
        for entry in &self.entries {
            e.put_bytes(&entry.id).map_err(|_| ReplError::EpochBound)?;
            e.put_bytes(&entry.envelope)
                .map_err(|_| ReplError::EpochBound)?;
            e.put_u64(entry.sequence);
            e.put_u64(entry.ts);
        }
        let bytes = e.finish();
        if self.base.seed.len() > MAX_CHECKPOINT_BYTES
            || bytes.len().saturating_sub(self.base.seed.len()) > MAX_METADATA
        {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }
    /// Authenticated local data only. This cannot return StudioClosingOverlayBasis.
    /// Full validation: structural checks AND complete ordered typed reconstruction.
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError> {
        Self::decode_vault_inner(bytes, ledger, true)
    }

    /// Same bounds, scope, entry, sequence, author, envelope and canonical-encoding checks as
    /// `decode_vault`, without replaying the branch. For metadata readers that need identity and
    /// accounting but no projection; it mints no authority and is never a display result. Full
    /// reconstruction remains mandatory before display, append, handoff preparation or export.
    /// Vault-sealed local bytes only: no network- or renderer-supplied bytes reach this.
    pub fn decode_vault_structural(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError> {
        Self::decode_vault_inner(bytes, ledger, false)
    }

    pub(super) fn decode_vault_inner(
        bytes: &[u8],
        ledger: &IntentLedger,
        replay: bool,
    ) -> Result<Self, ReplError> {
        if bytes.len() > MAX_EXTENSION {
            return Err(ReplError::EpochBound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8().map_err(|_| ReplError::Malformed)? != 1 {
            return Err(ReplError::Malformed);
        }
        let kind = d.get_u8().map_err(|_| ReplError::Malformed)?;
        let channel = fixed(&mut d)?;
        let author = DeviceId::from_bytes(fixed(&mut d)?);
        let source_id = u128::from_be_bytes(fixed(&mut d)?);
        let source_version = fixed(&mut d)?;
        let receipt = Receipt::decode(d.get_bytes().map_err(|_| ReplError::Malformed)?)?;
        let seed = d.get_bytes().map_err(|_| ReplError::Malformed)?;
        if seed.len() > MAX_CHECKPOINT_BYTES || bytes.len() - seed.len() > MAX_METADATA {
            return Err(ReplError::EpochBound);
        }
        let target = match (kind, ledger.document().doc_type) {
            (0, DocType::StudioIndex) => StudioTarget::Index { channel },
            (1, DocType::StudioObject) => StudioTarget::Flipnote {
                channel,
                object: ledger
                    .document()
                    .logical_key
                    .as_slice()
                    .try_into()
                    .map_err(|_| ReplError::EpochScope)?,
            },
            _ => return Err(ReplError::EpochScope),
        };
        if target.document(&receipt.document.server_id)? != receipt.document
            || &receipt.document != ledger.document()
        {
            return Err(ReplError::EpochScope);
        }
        let next_sequence = d.get_u64().map_err(|_| ReplError::Malformed)?;
        let count = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
        if count == 0 || count > MAX_STUDIO_OVERLAY_OPS {
            return Err(ReplError::EpochBound);
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(Entry {
                id: fixed(&mut d)?,
                envelope: fixed(&mut d)?,
                sequence: d.get_u64().map_err(|_| ReplError::Malformed)?,
                ts: d.get_u64().map_err(|_| ReplError::Malformed)?,
            });
        }
        d.finish().map_err(|_| ReplError::Malformed)?;
        let out = Self {
            base: BasisData {
                target,
                author,
                source_id,
                source_version,
                receipt,
                seed: seed.to_vec(),
            },
            entries,
            next_sequence,
        };
        // Redundant with `encode_vault` below, which checks the same predicate. Kept so a
        // structurally inconsistent branch is refused before any reconstruction or re-encoding.
        out.checked_entries(ledger)?;
        if replay {
            out.read(ledger)?;
        }
        if out.encode_vault(ledger)?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(out)
    }
}
