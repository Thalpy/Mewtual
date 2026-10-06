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
pub(in crate::studio) mod disposal;
mod eligibility;
mod handoff;
pub use archive::{StudioDraftArchive, StudioOverlayProvenance, MAX_STUDIO_DRAFT_ARCHIVE_BYTES};
pub use disposal::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioDisposalMode, StudioOverlayDisposal,
};
pub use eligibility::{StudioOverlayEligibility, StudioOverlayManualReason};
pub use handoff::{
    StudioHandoffAuthority, StudioHandoffCandidate, StudioHandoffEvidence, StudioHandoffOutcome,
    StudioHandoffSigning, StudioOverlayAdmission, StudioOverlayRequestClass, StudioOverlaySave,
    StudioOverlayState,
};

/// Which kind of basis a branch was minted from (design 8.1, review (i) change 2).
///
/// **Not persisted**, and that is the point. The nested basis blob predates provenance and is left
/// byte-for-byte unchanged, so an existing Closing branch keeps its fingerprint. The kind is
/// carried beside the blob instead: a mint sets it from the basis type, and the state decoder sets
/// it from the record's outer provenance once that has been read. `StudioOverlayState::validate`
/// then requires the two to agree. A decoder that forgot to set it would fingerprint a reloaded
/// Unconfirmed branch under the Closing domain, and its `branch_id`, its S3 re-mint and its
/// archive and disposal digests would all stop matching the live branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::studio) enum BasisKind {
    Closing,
    Unconfirmed,
}
impl BasisKind {
    pub(in crate::studio) fn of(provenance: &StudioOverlayProvenance) -> Self {
        match provenance {
            StudioOverlayProvenance::Closing => Self::Closing,
            StudioOverlayProvenance::Unconfirmed { .. } => Self::Unconfirmed,
        }
    }
}

#[derive(Clone)]
struct BasisData {
    target: StudioTarget,
    author: DeviceId,
    source_id: u128,
    source_version: [u8; 32],
    receipt: Receipt,
    seed: Vec<u8>,
    /// Selects the fingerprint's derive-key domain; see [`BasisKind`]. Never encoded.
    kind: BasisKind,
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
            kind: BasisKind::Closing,
        })
    }
    /// Local exact-request fence, not receipt or membership authority.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.0.fingerprint()
    }
}

/// An awaiting-tenure preview's seed, as the base of a local draft (design 8.1).
///
/// Unconfirmed history and never authority: no installed source, no tenure, no signing, no
/// handoff (8.5). `validate` refuses a Prepared state for it, and `prepare_handoff_detached`
/// refuses it by provenance.
///
/// **Identity versus admission facts.** The fingerprint covers what the base *is*: the target,
/// the author, the receipt bytes, the exact seed bytes and the Unconfirmed domain (its source
/// identity is canonically zero). The provider, MLS epoch and wall-clock time say how this base
/// was *observed*. They are carried here only so admission can record them once, in
/// `StudioOverlayProvenance::Unconfirmed`, and are deliberately not fingerprinted. If they were,
/// the S3 re-mint and every later Save to the branch would mint a different fingerprint, and a
/// preview refreshed from another provider would strand the branch. Present-time evidence is
/// the live callback's job, at every mint.
pub struct StudioUnconfirmedOverlayBasis {
    data: BasisData,
    provider: DeviceId,
    observed_mls_epoch: u64,
    observed_at_ms: u64,
}
impl std::fmt::Debug for StudioUnconfirmedOverlayBasis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioUnconfirmedOverlayBasis { .. }")
    }
}
impl StudioUnconfirmedOverlayBasis {
    /// **The one constructor. Call it only from `catcoms-sync`'s live-preview mint**, inside the
    /// scoped hint callback that re-checks every current-scope condition (design 8.1). Hidden
    /// because Rust cannot restrict a public constructor to one downstream crate. Its call sites
    /// are pinned by `clippy.toml`'s `disallowed-methods` and by the source-scan test
    /// `construction_gates_are_called_only_from_their_sanctioned_sites`, together with
    /// `UnconfirmedStudioSeed::parse_live_transfer`, the only maker of the value it takes.
    ///
    /// **Binds, it does not re-parse.** `receipt` must be exactly the receipt the seed's bytes
    /// were proven against when sync parsed them (canonical encoding, receipt binding and
    /// document identity, all at parse time). Equality is the whole check. This runs on the actor
    /// under the sync borrow, at S1b and again at S3, and a full re-parse of up to 2 MiB would
    /// stall sync twice per Save. The same crate keeps the original parse off the actor for that
    /// reason (design review of `47a73463`, M2).
    ///
    /// 8.1 part 3, "do not trust the retention", still holds, just off the actor. Every
    /// `StudioOverlay` reconstruction re-parses its base against its receipt (`BasisData::graph`),
    /// and that includes the typed reconstruction inside every `append`, so no accepted Save and
    /// no reload ever rests on the retained bytes unchecked.
    ///
    /// The base is built with a zero source identity. `observed_at_ms` is wall-clock time
    /// (`Clock::now_ms`), because it is persisted and a monotonic reading means nothing after a
    /// restart.
    #[doc(hidden)]
    pub fn mint_from_live_preview(
        seed: &UnconfirmedStudioSeed,
        receipt: &Receipt,
        author: DeviceId,
        provider: DeviceId,
        observed_mls_epoch: u64,
        observed_at_ms: u64,
    ) -> Result<Self, ReplError> {
        let target = seed.target();
        if seed.proven_receipt() != receipt {
            return Err(ReplError::EpochScope);
        }
        integer_bound(observed_at_ms)?;
        Ok(Self {
            data: BasisData {
                target,
                author,
                source_id: 0,
                source_version: [0; 32],
                receipt: receipt.clone(),
                seed: seed.seed_bytes().to_vec(),
                kind: BasisKind::Unconfirmed,
            },
            provider,
            observed_mls_epoch,
            observed_at_ms,
        })
    }
    /// Local exact-request fence, under the Unconfirmed domain. Never authority.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.data.fingerprint()
    }
    /// The admission facts this mint observed, as admission records them.
    pub fn provenance(&self) -> StudioOverlayProvenance {
        StudioOverlayProvenance::Unconfirmed {
            provider: self.provider,
            observed_mls_epoch: self.observed_mls_epoch,
            observed_at_ms: self.observed_at_ms,
        }
    }
}

/// Either basis, so the branch constructors take one argument whose type *is* its provenance.
///
/// Before this, `new_admitted` took a Closing basis and a provenance as separate arguments, and
/// nothing tied them together (design review (ii) change 4). Every constructor that takes this
/// derives the provenance from the variant. Accepted as `impl Into<StudioOverlayBasis>`, so an
/// existing caller passing `&StudioClosingOverlayBasis` compiles unchanged.
#[derive(Clone, Copy, Debug)]
pub enum StudioOverlayBasis<'a> {
    Closing(&'a StudioClosingOverlayBasis),
    Unconfirmed(&'a StudioUnconfirmedOverlayBasis),
}
impl<'a> From<&'a StudioClosingOverlayBasis> for StudioOverlayBasis<'a> {
    fn from(basis: &'a StudioClosingOverlayBasis) -> Self {
        Self::Closing(basis)
    }
}
impl<'a> From<&'a StudioUnconfirmedOverlayBasis> for StudioOverlayBasis<'a> {
    fn from(basis: &'a StudioUnconfirmedOverlayBasis) -> Self {
        Self::Unconfirmed(basis)
    }
}
impl StudioOverlayBasis<'_> {
    fn data(&self) -> &BasisData {
        match self {
            Self::Closing(basis) => &basis.0,
            Self::Unconfirmed(basis) => &basis.data,
        }
    }
    pub fn fingerprint(&self) -> [u8; 32] {
        self.data().fingerprint()
    }
    /// What a branch opened from this basis records, derived from the variant.
    pub fn provenance(&self) -> StudioOverlayProvenance {
        match self {
            Self::Closing(_) => StudioOverlayProvenance::Closing,
            Self::Unconfirmed(basis) => basis.provenance(),
        }
    }
    /// The exact target this basis was minted for. Public so a Save stage can refuse a basis minted
    /// for another channel BEFORE admission: a Flipnote's logical key omits its channel, so the
    /// identity comparison admission makes cannot see the difference, and a branch opened from the
    /// wrong basis would be recorded under the basis's channel while the request named another.
    pub fn target(&self) -> StudioTarget {
        self.data().target
    }
    pub(in crate::studio) fn closed_epoch(&self) -> u64 {
        self.data().receipt.closed_epoch
    }
    /// The bytes the fingerprint hashes, for tests that recompute it from the documented
    /// derivation. The domain can only be pinned that way: a Closing basis always has a nonzero
    /// source identity and an Unconfirmed one a zero one, so their fingerprints differ even under
    /// a shared domain, and comparing them proves nothing about it.
    #[cfg(test)]
    pub(in crate::studio) fn encoding_for_test(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        self.data().encode(&mut e).unwrap();
        e.finish()
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
        // The domain is the provenance discriminant. Same content, different kind, different
        // fingerprint, so neither basis can stand in for the other. The Closing domain is the one
        // every existing vault was written under and must not change.
        let domain = match self.kind {
            BasisKind::Closing => "catcoms/studio-closing-overlay-basis/v1",
            BasisKind::Unconfirmed => "catcoms/studio-unconfirmed-overlay-basis/v1",
        };
        blake3::derive_key(domain, &e.finish())
    }
    fn graph(&self) -> Result<(AutoCommit, StudioProjection), ReplError> {
        let (mut doc, projection) =
            UnconfirmedStudioSeed::parse_graph(self.target, &self.receipt, &self.seed)?;
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
    pub fn new<'a>(basis: impl Into<StudioOverlayBasis<'a>>) -> Self {
        Self {
            base: basis.into().data().clone(),
            entries: Vec::new(),
            next_sequence: 1,
        }
    }
    pub fn basis(&self) -> [u8; 32] {
        self.base.fingerprint()
    }
    /// Which domain this branch's basis fingerprints under. See [`BasisKind`].
    pub(in crate::studio) fn basis_kind(&self) -> BasisKind {
        self.base.kind
    }
    /// For the state decoder only, once it has read the record's provenance. The nested blob
    /// cannot say, so a standalone decode always starts as Closing.
    pub(in crate::studio) fn set_basis_kind(&mut self, kind: BasisKind) {
        self.base.kind = kind;
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
    /// How many operations this branch has accepted.
    ///
    /// Surfaced because a disposal request carries the count the user was shown, and comparing it is
    /// a distinct check from comparing the content hash: equal hashes with an unequal count would
    /// mean the caller and the vault disagree about size despite agreeing about content, which is a
    /// bug in the caller rather than a race, and it deserves to be reported as one.
    pub fn accepted(&self) -> usize {
        self.entries.len()
    }
    /// Whether this branch's nested basis carries no installed-source identity.
    ///
    /// An `Unconfirmed` branch has no installed source, so these must be canonically zero rather
    /// than merely unread: the nested v1 basis blob is untouched by the outer v3 record, so a
    /// nonzero value there would be a source claim nothing had authorised.
    pub(in crate::studio) fn has_zero_source_identity(&self) -> bool {
        self.base.source_id == 0 && self.base.source_version == [0; 32]
    }
    /// Whether the nested basis names an installed source in BOTH fields: the Closing rule's
    /// converse of the one above. A settlement-minted Closing basis always passes the epoch's
    /// document id and a source-version hash, neither of which is zero, so a Closing label over
    /// a basis missing either one is not a record this build wrote.
    pub(in crate::studio) fn has_complete_source_identity(&self) -> bool {
        self.base.source_id != 0 && self.base.source_version != [0; 32]
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
    pub fn append<'a>(
        &mut self,
        basis: impl Into<StudioOverlayBasis<'a>>,
        ledger: &IntentLedger,
        id: [u8; 32],
        ts: u64,
    ) -> Result<StudioLocalDraft, ReplError> {
        // Fingerprints are domain-separated by kind, so a Closing basis can never extend an
        // Unconfirmed branch, nor the reverse, even over identical receipt and seed bytes.
        if self.basis() != basis.into().fingerprint() {
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
    ///
    /// A standalone decode assumes a **Closing** basis, because the nested blob does not record
    /// its kind. A branch whose record carries Unconfirmed provenance must be decoded through
    /// `StudioOverlayState`, which supplies the kind; decoded here, its `basis()` would come out
    /// under the wrong domain.
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
                // The blob does not carry its kind. A v1 record is Closing by definition; for
                // v2/v3 `StudioOverlayState` overwrites this from the outer provenance before
                // anything fingerprints the branch. A standalone decode stays Closing.
                kind: BasisKind::Closing,
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
