//! Local transfer metadata. Decoding this record confers neither authority nor durability.
use super::*;
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_rt::CryptoRngCore;

#[derive(Debug)]
pub enum StudioOverlaySave {
    Local(StudioLocalDraft),
    HandedOff(StudioHandoffOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudioHandoffOutcome {
    pub basis: [u8; 32],
    pub epoch: u64,
    pub doc_id: u128,
    pub accepted: usize,
}

#[derive(Clone)]
struct Completed {
    target: StudioTarget,
    author: DeviceId,
    outcome: StudioHandoffOutcome,
    entries: Vec<Entry>,
}

#[derive(Clone)]
struct Prepared {
    epoch: u64,
    doc_id: u128,
    receipt: [u8; 32],
    before: [u8; 32],
    branch: [u8; 32],
    signed: Vec<[u8; 32]>,
}

/// Read-only local provenance. The mandatory target remains when the seed and ledger disappear.
#[derive(Clone)]
pub struct StudioOverlayState {
    target: StudioTarget,
    active: Option<StudioOverlay>,
    prepared: Option<Prepared>,
    completed: Option<Completed>,
    minimum_new_basis_closed_epoch: u64,
    legacy: bool,
}

impl std::fmt::Debug for StudioOverlayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StudioOverlayState")
            .field("prepared", &self.prepared.is_some())
            .field("completed", &self.completed.is_some())
            .finish_non_exhaustive()
    }
}

/// A separate signed source plus its exact local manifest, never produced by a decoder.
#[derive(Debug)]
pub struct StudioHandoffCandidate {
    source: StudioEpoch,
    metadata: StudioOverlayState,
}
impl StudioHandoffCandidate {
    pub fn into_parts(self) -> (StudioEpoch, StudioOverlayState) {
        (self.source, self.metadata)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioHandoffEvidence {
    /// No branch id exists in the same physical destination. Full local work must remain.
    Absent,
    /// Every full signed change, not merely its domain envelope, is retained.
    Complete,
    /// Partial, conflicting or different physical source. No completion or prefix replay.
    Hold,
}

impl StudioOverlayState {
    pub fn new(basis: &StudioClosingOverlayBasis) -> Self {
        Self {
            target: basis.0.target,
            active: Some(StudioOverlay::new(basis)),
            prepared: None,
            completed: None,
            minimum_new_basis_closed_epoch: 0,
            legacy: false,
        }
    }
    pub fn target(&self) -> StudioTarget {
        self.target
    }
    pub fn overlay(&self) -> Option<&StudioOverlay> {
        self.active.as_ref()
    }
    pub fn is_prepared(&self) -> bool {
        self.prepared.is_some()
    }
    pub fn has_completed(&self) -> bool {
        self.completed.is_some()
    }
    pub fn minimum_new_basis_closed_epoch(&self) -> u64 {
        self.minimum_new_basis_closed_epoch
    }
    pub fn check_target(&self, target: StudioTarget) -> Result<(), ReplError> {
        if target != self.target {
            return Err(ReplError::EpochScope);
        }
        Ok(())
    }
    /// This equality must precede any acknowledgement, source lookup or sync reservation.
    pub fn completed_retry(
        &self,
        target: StudioTarget,
        basis: [u8; 32],
        intent: &LocalIntent,
    ) -> Result<Option<StudioHandoffOutcome>, ReplError> {
        self.check_target(target)?;
        let Some(done) = &self.completed else {
            return Ok(None);
        };
        let id = intent.operation.id(&intent.author);
        let Some(entry) = done.entries.iter().find(|e| e.id == id) else {
            return Ok(None);
        };
        if done.author != intent.author
            || done.outcome.basis != basis
            || entry.envelope != envelope(intent)?
        {
            return Err(ReplError::IntentConflict);
        }
        Ok(Some(done.outcome.clone()))
    }
    pub fn completed_branch(
        &self,
        target: StudioTarget,
        author: DeviceId,
        basis: [u8; 32],
    ) -> Result<Option<StudioHandoffOutcome>, ReplError> {
        self.check_target(target)?;
        Ok(self
            .completed
            .as_ref()
            .filter(|c| c.author == author && c.outcome.basis == basis)
            .map(|c| c.outcome.clone()))
    }
    pub fn append(
        &mut self,
        basis: &StudioClosingOverlayBasis,
        ledger: &IntentLedger,
        id: [u8; 32],
        ts: u64,
    ) -> Result<StudioLocalDraft, ReplError> {
        self.check_target(basis.0.target)?;
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        check_basis_floor(
            basis.0.receipt.closed_epoch,
            self.minimum_new_basis_closed_epoch,
        )?;
        let mut active = self
            .active
            .clone()
            .unwrap_or_else(|| StudioOverlay::new(basis));
        let view = active.append(basis, ledger, id, ts)?;
        let mut next = self.clone();
        next.active = Some(active);
        next.legacy = false;
        next.encode_vault(ledger)?;
        *self = next;
        Ok(view)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_handoff(
        &self,
        source: &mut StudioEpoch,
        ledger: &IntentLedger,
        device: &MlsDevice,
        group: &ServerGroup,
        tenure: u64,
        rng: &mut impl CryptoRngCore,
    ) -> Result<StudioHandoffCandidate, ReplError> {
        self.validate(ledger)?;
        if self.prepared.is_some() {
            return Err(ReplError::EpochClosed);
        }
        let overlay = self.active.as_ref().ok_or(ReplError::EpochScope)?;
        let before = source_hash(source)?;
        let candidate = source.overlay_candidate(overlay, ledger, device, group, tenure, rng)?;
        let signed = overlay
            .checked_entries(ledger)?
            .into_iter()
            .map(|(_, i)| {
                candidate
                    .overlay_signed_hash(i)?
                    .ok_or(ReplError::Malformed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut metadata = self.clone();
        metadata.legacy = false;
        metadata.prepared = Some(Prepared {
            epoch: candidate.epoch(),
            doc_id: candidate.doc_id(),
            receipt: overlay.receipt().hash(),
            before,
            branch: branch_hash(overlay, ledger)?,
            signed,
        });
        metadata.encode_vault(ledger)?;
        Ok(StudioHandoffCandidate {
            source: candidate,
            metadata,
        })
    }
    /// Read-only evidence classification; never clears the local hold or implies a flush.
    pub fn evidence(
        &self,
        source: &StudioEpoch,
        ledger: &IntentLedger,
    ) -> Result<StudioHandoffEvidence, ReplError> {
        self.validate(ledger)?;
        let p = self.prepared.as_ref().ok_or(ReplError::EpochScope)?;
        if source.target() != self.target
            || source.doc_id() != p.doc_id
            || source.epoch() != p.epoch
        {
            return Ok(StudioHandoffEvidence::Hold);
        }
        let overlay = self.active.as_ref().ok_or(ReplError::Malformed)?;
        let mut count = 0;
        for ((_, intent), expected) in overlay.checked_entries(ledger)?.into_iter().zip(&p.signed) {
            match source.overlay_signed_hash(intent) {
                Ok(Some(actual)) if &actual == expected => count += 1,
                Ok(None) => {}
                _ => return Ok(StudioHandoffEvidence::Hold),
            }
        }
        Ok(if count == p.signed.len() {
            StudioHandoffEvidence::Complete
        } else if count == 0 {
            StudioHandoffEvidence::Absent
        } else {
            StudioHandoffEvidence::Hold
        })
    }
    pub fn matches_source_before(&self, source: &mut StudioEpoch) -> Result<bool, ReplError> {
        let hash = source_hash(source)?;
        Ok(self.prepared.as_ref().is_some_and(|p| p.before == hash))
    }
    /// Caller must persist this through its accounted writer. The full branch remains intact.
    pub fn return_to_active(
        &self,
        source: &StudioEpoch,
        ledger: &IntentLedger,
    ) -> Result<Self, ReplError> {
        if self.evidence(source, ledger)? != StudioHandoffEvidence::Absent {
            return Err(ReplError::IntentConflict);
        }
        let mut next = self.clone();
        next.prepared = None;
        Ok(next)
    }
    /// Only a checked complete signed log can release the base. Store must flush it FIRST.
    pub fn complete(&self, source: &StudioEpoch, ledger: &IntentLedger) -> Result<Self, ReplError> {
        if self.evidence(source, ledger)? != StudioHandoffEvidence::Complete {
            return Err(ReplError::IntentConflict);
        }
        let p = self.prepared.as_ref().ok_or(ReplError::Malformed)?;
        let active = self.active.as_ref().ok_or(ReplError::Malformed)?;
        let mut next = self.clone();
        next.completed = Some(Completed {
            target: self.target,
            author: active.author(),
            outcome: StudioHandoffOutcome {
                basis: active.basis(),
                epoch: p.epoch,
                doc_id: p.doc_id,
                accepted: active.entries.len(),
            },
            entries: active.entries.clone(),
        });
        next.minimum_new_basis_closed_epoch = next.minimum_new_basis_closed_epoch.max(p.epoch);
        next.active = None;
        next.prepared = None;
        next.legacy = false;
        next.encode_vault(ledger)?;
        Ok(next)
    }
    fn validate(&self, ledger: &IntentLedger) -> Result<(), ReplError> {
        if self.target.document(&ledger.document().server_id)? != *ledger.document() {
            return Err(ReplError::EpochScope);
        }
        if let Some(active) = &self.active {
            if active.target() != self.target {
                return Err(ReplError::EpochScope);
            }
            check_basis_floor(
                active.receipt().closed_epoch,
                self.minimum_new_basis_closed_epoch,
            )?;
            active.checked_entries(ledger)?;
        }
        if let Some(p) = &self.prepared {
            let active = self.active.as_ref().ok_or(ReplError::Malformed)?;
            if p.epoch
                != active
                    .receipt()
                    .closed_epoch
                    .checked_add(1)
                    .ok_or(ReplError::EpochBound)?
                || p.receipt != active.receipt().hash()
                || p.doc_id
                    != crate::epoch_id(
                        ledger.document().doc_type,
                        &ledger.document().logical_key,
                        p.epoch,
                        &active.receipt().close_record_hash,
                    )
                || p.signed.len() != active.entries.len()
                || p.branch != branch_hash(active, ledger)?
            {
                return Err(ReplError::IntentConflict);
            }
        }
        if let Some(c) = &self.completed {
            if c.target != self.target
                || c.outcome.epoch == 0
                || c.outcome.epoch > self.minimum_new_basis_closed_epoch
                || c.outcome.accepted != c.entries.len()
            {
                return Err(ReplError::EpochScope);
            }
            validate_manifest(&c.entries)?;
            let pending: BTreeMap<_, _> = ledger.pending().collect();
            for entry in &c.entries {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.contains(&entry.id))
                {
                    return Err(ReplError::IntentConflict);
                }
                if let Some(intent) = pending.get(&entry.id) {
                    if intent.author != c.author || envelope(intent)? != entry.envelope {
                        return Err(ReplError::IntentConflict);
                    }
                }
            }
        }
        Ok(())
    }
    pub fn encode_vault(&self, ledger: &IntentLedger) -> Result<Vec<u8>, ReplError> {
        self.validate(ledger)?;
        if self.legacy
            && self.prepared.is_none()
            && self.completed.is_none()
            && self.minimum_new_basis_closed_epoch == 0
        {
            return self
                .active
                .as_ref()
                .ok_or(ReplError::Malformed)?
                .encode_vault(ledger);
        }
        let mut e = Encoder::new();
        e.put_u8(2);
        put_target(&mut e, self.target)?;
        e.put_u64(self.minimum_new_basis_closed_epoch);
        match &self.active {
            None => {
                e.put_u8(0);
            }
            Some(active) => {
                e.put_u8(if self.prepared.is_some() { 2 } else { 1 });
                put(&mut e, &active.encode_vault(ledger)?)?;
                if let Some(p) = &self.prepared {
                    e.put_u64(p.epoch);
                    put(&mut e, &p.doc_id.to_be_bytes())?;
                    for hash in [p.receipt, p.before, p.branch] {
                        put(&mut e, &hash)?;
                    }
                    e.put_u32(p.signed.len() as u32);
                    for hash in &p.signed {
                        put(&mut e, hash)?;
                    }
                }
            }
        }
        match &self.completed {
            None => {
                e.put_u8(0);
            }
            Some(c) => {
                e.put_u8(1);
                put_target(&mut e, c.target)?;
                put(&mut e, c.author.as_bytes())?;
                put(&mut e, &c.outcome.basis)?;
                e.put_u64(c.outcome.epoch);
                put(&mut e, &c.outcome.doc_id.to_be_bytes())?;
                e.put_u32(c.entries.len() as u32);
                for entry in &c.entries {
                    put_entry(&mut e, entry)?;
                }
            }
        }
        let bytes = e.finish();
        let seed_bytes = self.active.as_ref().map_or(0, |a| a.base.seed.len());
        if bytes.len().saturating_sub(seed_bytes) > MAX_METADATA || bytes.len() > MAX_EXTENSION {
            return Err(ReplError::EpochBound);
        }
        Ok(bytes)
    }
    pub fn decode_vault(bytes: &[u8], ledger: &IntentLedger) -> Result<Self, ReplError> {
        if bytes.len() > MAX_EXTENSION {
            return Err(ReplError::EpochBound);
        }
        if bytes.first() == Some(&1) {
            let active = StudioOverlay::decode_vault(bytes, ledger)?;
            return Ok(Self {
                target: active.target(),
                active: Some(active),
                prepared: None,
                completed: None,
                minimum_new_basis_closed_epoch: 0,
                legacy: true,
            });
        }
        let mut d = Decoder::new(bytes);
        if byte(&mut d)? != 2 {
            return Err(ReplError::Malformed);
        }
        let target = get_target(&mut d)?;
        let minimum_new_basis_closed_epoch = number(&mut d)?;
        let tag = byte(&mut d)?;
        let (active, prepared) = match tag {
            0 => (None, None),
            1 | 2 => {
                let raw = d.get_bytes().map_err(|_| ReplError::Malformed)?;
                // Count the rest of the v2 record before allocating/reconstructing the seed.
                // The nested v1 decoder independently bounds its own seed and metadata.
                if bytes.len().saturating_sub(raw.len()) > MAX_METADATA {
                    return Err(ReplError::EpochBound);
                }
                let seed_bytes = nested_seed_len(raw)?;
                if bytes.len().saturating_sub(seed_bytes) > MAX_METADATA {
                    return Err(ReplError::EpochBound);
                }
                let a = StudioOverlay::decode_vault(raw, ledger)?;
                let p = if tag == 2 {
                    let epoch = number(&mut d)?;
                    let doc_id = u128::from_be_bytes(fixed(&mut d)?);
                    let receipt = fixed(&mut d)?;
                    let before = fixed(&mut d)?;
                    let branch = fixed(&mut d)?;
                    let n = count(&mut d)?;
                    let signed = (0..n)
                        .map(|_| fixed(&mut d))
                        .collect::<Result<Vec<_>, _>>()?;
                    Some(Prepared {
                        epoch,
                        doc_id,
                        receipt,
                        before,
                        branch,
                        signed,
                    })
                } else {
                    None
                };
                (Some(a), p)
            }
            _ => return Err(ReplError::Malformed),
        };
        let completed = match byte(&mut d)? {
            0 => None,
            1 => {
                let target = get_target(&mut d)?;
                let author = DeviceId::from_bytes(fixed(&mut d)?);
                let basis = fixed(&mut d)?;
                let epoch = number(&mut d)?;
                let doc_id = u128::from_be_bytes(fixed(&mut d)?);
                let n = count(&mut d)?;
                let entries = (0..n)
                    .map(|_| get_entry(&mut d))
                    .collect::<Result<Vec<_>, _>>()?;
                Some(Completed {
                    target,
                    author,
                    outcome: StudioHandoffOutcome {
                        basis,
                        epoch,
                        doc_id,
                        accepted: n,
                    },
                    entries,
                })
            }
            _ => return Err(ReplError::Malformed),
        };
        d.finish().map_err(|_| ReplError::Malformed)?;
        let out = Self {
            target,
            active,
            prepared,
            completed,
            minimum_new_basis_closed_epoch,
            legacy: false,
        };
        if out.encode_vault(ledger)?.as_slice() != bytes {
            return Err(ReplError::Malformed);
        }
        Ok(out)
    }
}

fn source_hash(source: &mut StudioEpoch) -> Result<[u8; 32], ReplError> {
    Ok(blake3::derive_key(
        "catcoms/studio-overlay-source-before/v1",
        &source.snapshot()?,
    ))
}
fn check_basis_floor(closed: u64, floor: u64) -> Result<(), ReplError> {
    if closed < floor {
        return Err(ReplError::EpochScope);
    }
    Ok(())
}
// Borrow the nested v1 fields to charge combined metadata before any seed allocation/replay.
fn nested_seed_len(raw: &[u8]) -> Result<usize, ReplError> {
    let mut d = Decoder::new(raw);
    if byte(&mut d)? != 1 {
        return Err(ReplError::Malformed);
    }
    byte(&mut d)?;
    for _ in 0..5 {
        d.get_bytes().map_err(|_| ReplError::Malformed)?;
    }
    let seed = d.get_bytes().map_err(|_| ReplError::Malformed)?;
    if seed.len() > MAX_CHECKPOINT_BYTES {
        return Err(ReplError::EpochBound);
    }
    Ok(seed.len())
}
fn branch_hash(active: &StudioOverlay, ledger: &IntentLedger) -> Result<[u8; 32], ReplError> {
    let mut hash = blake3::Hasher::new_derive_key("catcoms/studio-overlay-branch-ledger/v1");
    for bytes in [active.encode_vault(ledger)?, ledger.encode()?] {
        hash.update(&(bytes.len() as u64).to_be_bytes());
        hash.update(&bytes);
    }
    Ok(*hash.finalize().as_bytes())
}
fn validate_manifest(entries: &[Entry]) -> Result<(), ReplError> {
    if entries.is_empty() || entries.len() > MAX_STUDIO_OVERLAY_OPS {
        return Err(ReplError::EpochBound);
    }
    let mut seen = BTreeSet::new();
    for (i, e) in entries.iter().enumerate() {
        if e.sequence != i as u64 + 1 || !seen.insert(e.id) {
            return Err(ReplError::IntentConflict);
        }
        integer_bound(e.ts)?;
    }
    Ok(())
}
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
fn count(d: &mut Decoder<'_>) -> Result<usize, ReplError> {
    let n = d.get_u32().map_err(|_| ReplError::Malformed)? as usize;
    if n == 0 || n > MAX_STUDIO_OVERLAY_OPS {
        return Err(ReplError::EpochBound);
    }
    Ok(n)
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
fn put_entry(e: &mut Encoder, entry: &Entry) -> Result<(), ReplError> {
    put(e, &entry.id)?;
    put(e, &entry.envelope)?;
    e.put_u64(entry.sequence);
    e.put_u64(entry.ts);
    Ok(())
}
fn get_entry(d: &mut Decoder<'_>) -> Result<Entry, ReplError> {
    Ok(Entry {
        id: fixed(d)?,
        envelope: fixed(d)?,
        sequence: number(d)?,
        ts: number(d)?,
    })
}
