//! Bounded PUBLIC resulting-state DATA for the pending v2 governor. Canonical public semantics,
//! never private ciphertext/page IDs or signature representatives, determine the commitment.
//!
//! This codec is NOT the state-transition/MLS/current-case verifier. Real selected-chain replay,
//! original MLS observations, cold lookup completeness, mounted root/writer custody, reserved
//! control sequences and current-effect issuance remain separate mandatory production gates.
use super::acceptance_tree::{TreeContext, TreeHash};
use super::owner_facts::OwnerFactId;
use super::owner_header::{
    HeaderKind, OwnerBodyId, OwnerHeaderId, OwnerStateCommitment, OwnerTenureId, SignedOwnerHeader,
};
use super::owner_profile::{AdmissionBody, SignedOwnerProfile, MAX_CURRENT_MEMBERS};
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_wire::{Decoder, Encoder};
pub mod transition;

const DOMAIN: &[u8] = b"mewtual/chat/owner-public-state/v2\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/owner-public-state-id/v2\0";
const GENESIS_DOMAIN: &[u8] = b"mewtual/chat/owner-genesis-state/v2\0";
const SLOT_BYTES: usize = 4 * 32 + 8;
/// One bounded public image buffer, excluding signatures/SDK/provider/cold-source/host fees.
/// Parsing borrows slots/group, and encoding appends packed slots directly into one output.
pub const MAX_STATE_BYTES: usize = 4
    + DOMAIN.len()
    + 2
    + 4
    + MAX_GROUP_ID_BYTES
    + 6 * 36
    + 8
    + 1
    + 8
    + 2
    + 4
    + MAX_CURRENT_MEMBERS as usize * SLOT_BYTES;
const MAX_GENESIS_STATE_BYTES: usize = 4 + GENESIS_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES + 4 * 36;

/// One CURRENT admission's public data. Historical admissions/cuts/decisions remain committed
/// by the prior-state/body fold and must be retained separately; this is no lifetime64 cap.
/// The selected tip HEADER identity is deliberately absent: for a new tip it is derived only
/// AFTER signing/verifying the resulting header, avoiding a self-CID commitment cycle. Other
/// slots retain THEIR OWN verified tip header, not the latest unrelated global header.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CurrentAdmissionSlot {
    pub admission: OwnerBodyId,
    pub subject: [u8; 32],
    pub tip_body: OwnerBodyId,
    pub accepted_count: u64,
    pub accepted_root: TreeHash,
}
impl std::fmt::Debug for CurrentAdmissionSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CurrentAdmissionSlot")
            .field("count", &self.accepted_count)
            .finish_non_exhaustive()
    }
}
impl CurrentAdmissionSlot {
    fn validate(self, profile: OwnerFactId) -> Result<(), ChatError> {
        if self.accepted_count == 0 {
            let context = TreeContext {
                profile,
                admission: OwnerFactId::from_bytes(*self.admission.as_bytes()),
            };
            if self.tip_body != self.admission || self.accepted_root != context.empty_hash() {
                return Err(ChatError::Dependency);
            }
        } else if self.tip_body == self.admission {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
}

/// Caller-owned DATA, not a trusted root, actual MLS transition or selected public head.
/// The parent state/body fold preserves history when slots leave the bounded current set.
#[derive(Clone, Copy)]
pub struct StateMeta<'a> {
    pub group: &'a [u8],
    pub profile: OwnerFactId,
    pub issuer: [u8; 32],
    pub tenure: OwnerTenureId,
    pub parent: OwnerHeaderId,
    pub previous: OwnerStateCommitment,
    pub sequence: u64,
    pub kind: HeaderKind,
    pub body: OwnerBodyId,
    pub mls_epoch: u64,
}
impl std::fmt::Debug for StateMeta<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateMeta").finish_non_exhaustive()
    }
}

/// Strict borrowed state image. Successful decode/hash/signature binding does NOT prove that
/// these slots result from applying the actual body/MLS operation to a selected prior state.
pub struct StateFrame<'a> {
    raw: &'a [u8],
    meta: StateMeta<'a>,
    packed: &'a [u8],
}
impl std::fmt::Debug for StateFrame<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateFrame")
            .field("current_slots", &(self.packed.len() / SLOT_BYTES))
            .finish_non_exhaustive()
    }
}
impl<'a> StateFrame<'a> {
    /// Full bounded borrowed syntax/count/order/empty-tree validation before any owning decode.
    /// At most2016 fixed subject comparisons enforce uniqueness; no parsed map/Vec is allocated.
    pub fn decode(raw: &'a [u8]) -> Result<Self, ChatError> {
        if raw.len() > MAX_STATE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(raw);
        if d.get_bytes()? != DOMAIN || d.get_u16()? != 2 {
            return Err(ChatError::Malformed);
        }
        let group = d.get_bytes()?;
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let issuer = digest(&mut d)?;
        let tenure = OwnerTenureId::from_bytes(digest(&mut d)?);
        let parent = OwnerHeaderId::from_bytes(digest(&mut d)?);
        let previous = OwnerStateCommitment::from_bytes(digest(&mut d)?);
        let sequence = d.get_u64()?;
        let kind = HeaderKind::from_tag(d.get_u8()?)?;
        let body = OwnerBodyId::from_bytes(digest(&mut d)?);
        let mls_epoch = d.get_u64()?;
        let count = usize::from(d.get_u16()?);
        if count == 0 || count > usize::from(MAX_CURRENT_MEMBERS) {
            return Err(ChatError::Bound);
        }
        let packed = d.get_bytes()?;
        if packed.len() != count * SLOT_BYTES {
            return Err(ChatError::Malformed);
        }
        d.finish()?;
        let result = Self {
            raw,
            meta: StateMeta {
                group,
                profile,
                issuer,
                tenure,
                parent,
                previous,
                sequence,
                kind,
                body,
                mls_epoch,
            },
            packed,
        };
        validate(result.meta, result.slots())?;
        Ok(result)
    }
    pub fn meta(&self) -> StateMeta<'a> {
        self.meta
    }
    pub fn slots(&self) -> impl ExactSizeIterator<Item = CurrentAdmissionSlot> + Clone + '_ {
        self.packed.chunks_exact(SLOT_BYTES).map(slot)
    }
    /// Domain-separated hash of canonical PUBLIC bytes only. A hash is never Current authority.
    pub fn commitment(&self) -> OwnerStateCommitment {
        state_hash(self.raw)
    }
    /// Both the actual signature and every header/state frame field must match. No body
    /// transition, cold case lookup, root provenance or actual private MLS event follows.
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        if !header.verify() {
            return Err(ChatError::Signature);
        }
        let h = header.spec();
        let m = self.meta;
        if h.group.as_slice() != m.group
            || h.profile != m.profile
            || h.issuer != m.issuer
            || h.tenure != m.tenure
            || h.parent != m.parent
            || h.sequence != m.sequence
            || h.kind != m.kind
            || h.body != m.body
        {
            return Err(ChatError::Scope);
        }
        if h.state != self.commitment() {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
    /// Exact signed successor geometry + prior-state commitment, not selected-chain finality.
    /// The consumer must ALSO apply the exact body and compare every resulting slot/epoch.
    pub fn verify_successor_binding(
        &self,
        header: &SignedOwnerHeader,
        parent: &SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        header.verify_successor_of(parent)?;
        self.verify_header_binding(header)?;
        if parent.spec().state != self.meta.previous {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
    /// Pure Profile/first-header/empty-Founder image consistency. This never proves that the
    /// Profile came from an actual pinned fresh founder, private creation or covering save.
    pub fn verify_genesis_binding(
        &self,
        profile: &SignedOwnerProfile,
        body: &AdmissionBody,
        header: &SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        profile.verify_genesis_binding(body, header)?;
        self.verify_header_binding(header)?;
        let mut slots = self.slots();
        let first = slots.next().ok_or(ChatError::Dependency)?;
        if self.meta.previous != virtual_genesis_state(profile)?
            || self.meta.mls_epoch != 0
            || slots.next().is_some()
            || first.admission != body.id()
            || first.subject != profile.spec().founder
            || first.accepted_count != 0
        {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
}

/// Encode bounded untrusted public data with ONE requested output buffer and no packed-slot
/// scratch Vec. Input ownership, signature/SDK work, runtime metadata, host/control resources
/// and cold storage have independent invoices; this per-buffer cap pays none of them.
pub fn encode_state(
    meta: StateMeta<'_>,
    slots: &[CurrentAdmissionSlot],
) -> Result<Vec<u8>, ChatError> {
    validate(meta, slots.iter().copied())?;
    let mut e = Encoder::with_capacity(MAX_STATE_BYTES);
    e.put_bytes(DOMAIN)?;
    e.put_u16(2);
    e.put_bytes(meta.group)?;
    for value in [
        meta.profile.as_bytes(),
        &meta.issuer,
        meta.tenure.as_bytes(),
        meta.parent.as_bytes(),
        meta.previous.as_bytes(),
    ] {
        e.put_bytes(value)?;
    }
    e.put_u64(meta.sequence).put_u8(meta.kind.tag());
    e.put_bytes(meta.body.as_bytes())?;
    e.put_u64(meta.mls_epoch)
        .put_u16(slots.len() as u16)
        .put_u32((slots.len() * SLOT_BYTES) as u32);
    let mut encoded = e.finish();
    for slot in slots {
        encoded.extend_from_slice(slot.admission.as_bytes());
        encoded.extend_from_slice(&slot.subject);
        encoded.extend_from_slice(slot.tip_body.as_bytes());
        encoded.extend_from_slice(&slot.accepted_count.to_be_bytes());
        encoded.extend_from_slice(slot.accepted_root.as_bytes());
    }
    debug_assert!(encoded.len() <= MAX_STATE_BYTES);
    Ok(encoded)
}

/// Virtual seq0 state label only, not a decoded/signed owner header or positive empty state.
/// Equivalent genuine Profile signature representations yield the same label, after EACH is
/// verified. Existing raw v1 Profile/fact bytes cannot enter this typed v2-only function.
pub fn virtual_genesis_state(
    profile: &SignedOwnerProfile,
) -> Result<OwnerStateCommitment, ChatError> {
    if !profile.verify() {
        return Err(ChatError::Signature);
    }
    let mut e = Encoder::with_capacity(MAX_GENESIS_STATE_BYTES);
    e.put_bytes(GENESIS_DOMAIN)?;
    e.put_bytes(&profile.spec().group)?;
    e.put_bytes(profile.id().as_bytes())?;
    e.put_bytes(profile.tenure().as_bytes())?;
    e.put_bytes(&profile.spec().founder)?;
    e.put_bytes(&profile.spec().creation)?;
    Ok(state_hash(e.as_bytes()))
}
fn state_hash(bytes: &[u8]) -> OwnerStateCommitment {
    let mut h = blake3::Hasher::new();
    h.update(ID_DOMAIN);
    h.update(bytes);
    OwnerStateCommitment::from_bytes(*h.finalize().as_bytes())
}
fn validate(
    meta: StateMeta<'_>,
    slots: impl ExactSizeIterator<Item = CurrentAdmissionSlot> + Clone,
) -> Result<(), ChatError> {
    if meta.group.is_empty()
        || meta.group.len() > MAX_GROUP_ID_BYTES
        || slots.len() == 0
        || slots.len() > usize::from(MAX_CURRENT_MEMBERS)
    {
        return Err(ChatError::Bound);
    }
    if meta.sequence == 0 {
        return Err(ChatError::Dependency);
    }
    let mut previous = None;
    let mut issuer_present = false;
    for (index, current) in slots.clone().enumerate() {
        current.validate(meta.profile)?;
        if previous.is_some_and(|id| id >= current.admission) {
            return Err(ChatError::Malformed);
        }
        if slots
            .clone()
            .take(index)
            .any(|old| old.subject == current.subject)
        {
            return Err(ChatError::AuthorityConflict);
        }
        previous = Some(current.admission);
        issuer_present |= current.subject == meta.issuer;
    }
    // Supported tenure has NO succession/issuer-removal grammar. Do not leave a removed issuer
    // governing future headers merely because a signature still verifies under its old key.
    if !issuer_present {
        return Err(ChatError::Authority);
    }
    Ok(())
}
fn slot(bytes: &[u8]) -> CurrentAdmissionSlot {
    // Only chunks_exact(SLOT_BYTES) calls this after complete outer framing/count validation.
    CurrentAdmissionSlot {
        admission: OwnerBodyId::from_bytes(bytes[..32].try_into().expect("fixed slot")),
        subject: bytes[32..64].try_into().expect("fixed slot"),
        tip_body: OwnerBodyId::from_bytes(bytes[64..96].try_into().expect("fixed slot")),
        accepted_count: u64::from_be_bytes(bytes[96..104].try_into().expect("fixed slot")),
        accepted_root: TreeHash::from_bytes(bytes[104..136].try_into().expect("fixed slot")),
    }
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}

#[cfg(test)]
mod tests;
