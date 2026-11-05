//! Strict public owner attestations for the INACTIVE, bounded CHECKPOINT-1 producer.
//!
//! These signatures do not independently prove private MLS transitions or physical time.
//! Decoding/signing a fact never installs a trusted root or constructs historical authority.
//! The host must acquire its expected pin independently; the forthcoming archive verifier and
//! actual MLS producer must enforce that boundary before any projection or publication.

use super::{ActionId, BasisId, ChatError, GrantId, Rights, MAX_GROUP_ID_BYTES, PROFILE};
use catcoms_crypto::verify_with_public_bytes;
use catcoms_wire::{Decoder, Encoder};

mod authority;
mod initialization;
pub use authority::{OwnerBasisFacts, OwnerProfileExpectation};

const FACT_DOMAIN: &[u8] = b"mewtual/chat/owner-fact/v1\0";
const ENVELOPE_DOMAIN: &[u8] = b"mewtual/chat/owner-fact-envelope/v1\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/owner-fact-id/v1\0";
const GRANT_DOMAIN: &[u8] = b"mewtual/chat/owner-grant-id/v1\0";
const BASIS_DOMAIN: &[u8] = b"mewtual/chat/owner-basis-id/v1\0";
const TRUST_RULE: &str = "owner-non-equivocating-founder/v1";
const RESOURCE_PROFILE: &str = "owner-evidence-CHECKPOINT-1";

/// This checkpoint is not the production retention or membership policy. Its fixed-list cut
/// and lifetime admission limits need the reviewed scalable successor before activation.
pub const MAX_SMALL_FACT_BYTES: usize = 4 * 1024;
pub const MAX_REMOVAL_BYTES: usize = 140 * 1024;
pub const MAX_CUT_ACTIONS: usize = 4096;
pub const MAX_BASIS_COVERAGE: usize = 65;
pub const MAX_RETAINED_ADMISSIONS: usize = 64;
pub const MAX_ARCHIVE_ROWS: usize = 32_768;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub const ADMISSION_RESERVED_BYTES: usize = 300 * 1024;
pub const ADMISSION_RESERVED_ROWS: usize = 7;
pub const ADMISSION_CONTROL_SLOTS: usize = 3;
pub const ACCEPTANCE_RESERVED_BYTES: usize = 8 * 1024;
pub const ACCEPTANCE_RESERVED_ROWS: usize = 2;
pub const ACCEPTANCE_CONTROL_SLOTS: usize = 2;
pub const MAX_OWNER_OPERATIONS: usize = 32;
pub const MAX_CONTROL_SLOTS: usize = 288;
pub const MAX_PUBLICATION_BYTES: usize = 1024 * 1024;

/// Semantic identity, excluding signature representation. A label is never authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OwnerFactId([u8; 32]);
impl OwnerFactId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Closed origin grammar; founding is not represented by a fake zero KeyPackage/Add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionOrigin {
    Founder {
        creation: [u8; 32],
    },
    Add {
        commit: [u8; 32],
        before_epoch: u64,
        after_epoch: u64,
        key_package: [u8; 32],
        credential: [u8; 32],
    },
}

/// Canonical signed facts. Group admission is distinct from channel-scoped permission.
/// Values remain untrusted statements until their expected profile and dependencies verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerFactBody {
    Profile {
        creation: [u8; 32],
        founder: [u8; 32],
    },
    Admission {
        subject: [u8; 32],
        origin: AdmissionOrigin,
        predecessor: OwnerFactId,
    },
    RealmGrant {
        channel: u128,
        admission: OwnerFactId,
        subject: [u8; 32],
        rights: Rights,
    },
    Basis {
        channel: u128,
        admission: OwnerFactId,
        grant: GrantId,
        subject: [u8; 32],
        covered: Vec<BasisId>,
    },
    Acceptance {
        channel: u128,
        action: ActionId,
        admission: OwnerFactId,
        subject: [u8; 32],
        grant: GrantId,
        basis: BasisId,
        create: Option<ActionId>,
    },
    Removal {
        admission: OwnerFactId,
        subject: [u8; 32],
        commit: [u8; 32],
        before_epoch: u64,
        after_epoch: u64,
        accepted: Vec<ActionId>,
    },
    JoinBootstrap {
        key_package: [u8; 32],
        subject: [u8; 32],
        welcome: [u8; 32],
        expected_founder: [u8; 32],
    },
}
impl OwnerFactBody {
    fn kind(&self) -> u8 {
        match self {
            Self::Profile { .. } => 1,
            Self::Admission { .. } => 2,
            Self::RealmGrant { .. } => 3,
            Self::Basis { .. } => 4,
            Self::Acceptance { .. } => 5,
            Self::Removal { .. } => 6,
            Self::JoinBootstrap { .. } => 7,
        }
    }
}

/// Unsigned input for a cryptographic record primitive, not an observed-MLS capability.
/// The actual owner producer must obtain transition observations and covering save custody.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerFactSpec {
    pub group: Vec<u8>,
    /// Absent only on Profile, which cannot contain its own semantic CID.
    pub profile: Option<OwnerFactId>,
    pub body: OwnerFactBody,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedOwnerFact {
    spec: OwnerFactSpec,
    issuer: [u8; 32],
    signature: [u8; 64],
    id: OwnerFactId,
}
impl std::fmt::Debug for SignedOwnerFact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOwnerFact")
            .field("kind", &self.spec.body.kind())
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl SignedOwnerFact {
    /// Sign exact canonical bytes through the caller's existing key custody (e.g. MlsDevice).
    /// This primitive confers no trust on that key and makes no claim the stated transition ran.
    /// Checking the returned signature catches accidental key/issuer substitution immediately.
    pub fn sign_using(
        spec: OwnerFactSpec,
        issuer: [u8; 32],
        sign: impl FnOnce(&[u8]) -> Result<[u8; 64], ChatError>,
    ) -> Result<Self, ChatError> {
        let mut fact = Self {
            spec,
            issuer,
            signature: [0; 64],
            id: OwnerFactId([0; 32]),
        };
        fact.validate()?;
        let semantic = fact.semantic_bytes();
        fact.id = semantic_id(&semantic);
        fact.signature = sign(&semantic)?;
        if !verify_with_public_bytes(&fact.issuer, &semantic, &fact.signature) {
            return Err(ChatError::Signature);
        }
        if fact.encode().len() > fact.maximum_encoded_bytes() {
            return Err(ChatError::Bound);
        }
        Ok(fact)
    }
    pub fn id(&self) -> OwnerFactId {
        self.id
    }
    pub fn spec(&self) -> &OwnerFactSpec {
        &self.spec
    }
    pub fn issuer(&self) -> &[u8; 32] {
        &self.issuer
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
    /// Pure signature/grammar validation. A self-signed Profile cannot install a root.
    pub fn verify(&self) -> bool {
        self.validate().is_ok()
            && verify_with_public_bytes(&self.issuer, &self.semantic_bytes(), &self.signature)
    }
    /// Derived label only; this does not validate the signature, issuer pin or permission.
    pub fn grant_id(&self) -> Option<GrantId> {
        matches!(self.spec.body, OwnerFactBody::RealmGrant { .. })
            .then(|| GrantId::from_bytes(hash_label(GRANT_DOMAIN, &self.semantic_bytes())))
    }
    /// Derived label only; admission and covered-basis dependencies still need verification.
    pub fn basis_id(&self) -> Option<BasisId> {
        matches!(self.spec.body, OwnerFactBody::Basis { .. })
            .then(|| BasisId::from_bytes(hash_label(BASIS_DOMAIN, &self.semantic_bytes())))
    }
    fn maximum_encoded_bytes(&self) -> usize {
        if matches!(self.spec.body, OwnerFactBody::Removal { .. }) {
            MAX_REMOVAL_BYTES
        } else {
            MAX_SMALL_FACT_BYTES
        }
    }
    fn validate(&self) -> Result<(), ChatError> {
        if self.spec.group.is_empty() || self.spec.group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        if matches!(self.spec.body, OwnerFactBody::Profile { .. }) != self.spec.profile.is_none() {
            return Err(ChatError::Scope);
        }
        match &self.spec.body {
            OwnerFactBody::Profile { founder, .. } if founder != &self.issuer => {
                return Err(ChatError::Authority);
            }
            OwnerFactBody::Admission {
                subject,
                predecessor,
                origin,
            } => {
                if Some(*predecessor) != self.spec.profile {
                    return Err(ChatError::Scope);
                }
                match origin {
                    AdmissionOrigin::Founder { .. } if subject != &self.issuer => {
                        return Err(ChatError::Authority);
                    }
                    AdmissionOrigin::Add {
                        before_epoch,
                        after_epoch,
                        ..
                    } => {
                        check_next_epoch(*before_epoch, *after_epoch)?;
                    }
                    _ => {}
                }
            }
            OwnerFactBody::Basis { covered, .. } => {
                if covered.len() > MAX_BASIS_COVERAGE || !strict_order(covered) {
                    return Err(ChatError::Bound);
                }
            }
            OwnerFactBody::Removal {
                before_epoch,
                after_epoch,
                accepted,
                ..
            } => {
                check_next_epoch(*before_epoch, *after_epoch)?;
                if accepted.len() > MAX_CUT_ACTIONS || !strict_order(accepted) {
                    return Err(ChatError::Bound);
                }
            }
            OwnerFactBody::JoinBootstrap {
                expected_founder, ..
            } if expected_founder != &self.issuer => {
                return Err(ChatError::Authority);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(FACT_DOMAIN).expect("fixed domain");
        e.put_u8(self.spec.body.kind());
        e.put_bytes(&self.spec.group).expect("validated group");
        e.put_u16(PROFILE);
        if let Some(profile) = self.spec.profile {
            put_digest(&mut e, profile.as_bytes());
        }
        put_digest(&mut e, &self.issuer);
        match &self.spec.body {
            OwnerFactBody::Profile { creation, founder } => {
                e.put_str(TRUST_RULE).expect("fixed trust rule");
                put_digest(&mut e, creation);
                put_digest(&mut e, founder);
                e.put_u8(0); // No portable succession support in CHECKPOINT-1.
                e.put_str(RESOURCE_PROFILE).expect("fixed resource profile");
            }
            OwnerFactBody::Admission {
                subject,
                origin,
                predecessor,
            } => {
                put_digest(&mut e, subject);
                match origin {
                    AdmissionOrigin::Founder { creation } => {
                        e.put_u8(0);
                        put_digest(&mut e, creation);
                    }
                    AdmissionOrigin::Add {
                        commit,
                        before_epoch,
                        after_epoch,
                        key_package,
                        credential,
                    } => {
                        e.put_u8(1);
                        put_digest(&mut e, commit);
                        e.put_u64(*before_epoch).put_u64(*after_epoch);
                        put_digest(&mut e, key_package);
                        put_digest(&mut e, credential);
                    }
                }
                put_digest(&mut e, predecessor.as_bytes());
            }
            OwnerFactBody::RealmGrant {
                channel,
                admission,
                subject,
                rights,
            } => {
                e.put_u128(*channel);
                put_digest(&mut e, admission.as_bytes());
                put_digest(&mut e, subject);
                e.put_u16(rights.bits());
            }
            OwnerFactBody::Basis {
                channel,
                admission,
                grant,
                subject,
                covered,
            } => {
                e.put_u128(*channel);
                put_digest(&mut e, admission.as_bytes());
                put_digest(&mut e, grant.as_bytes());
                put_digest(&mut e, subject);
                e.put_u16(covered.len() as u16);
                for basis in covered {
                    put_digest(&mut e, basis.as_bytes());
                }
            }
            OwnerFactBody::Acceptance {
                channel,
                action,
                admission,
                subject,
                grant,
                basis,
                create,
            } => {
                e.put_u128(*channel);
                put_digest(&mut e, action.as_bytes());
                put_digest(&mut e, admission.as_bytes());
                put_digest(&mut e, subject);
                put_digest(&mut e, grant.as_bytes());
                put_digest(&mut e, basis.as_bytes());
                e.put_u8(u8::from(create.is_some()));
                if let Some(create) = create {
                    put_digest(&mut e, create.as_bytes());
                }
            }
            OwnerFactBody::Removal {
                admission,
                subject,
                commit,
                before_epoch,
                after_epoch,
                accepted,
            } => {
                put_digest(&mut e, admission.as_bytes());
                put_digest(&mut e, subject);
                put_digest(&mut e, commit);
                e.put_u64(*before_epoch).put_u64(*after_epoch);
                e.put_u16(accepted.len() as u16);
                // ONE packed field is part of the reviewed size/reservation grammar. Per-CID
                // prefixes would exceed the promised maximum cut and strand retirement.
                let mut packed = Vec::with_capacity(accepted.len() * 32);
                for action in accepted {
                    packed.extend_from_slice(action.as_bytes());
                }
                e.put_bytes(&packed).expect("validated packed cut");
            }
            OwnerFactBody::JoinBootstrap {
                key_package,
                subject,
                welcome,
                expected_founder,
            } => {
                put_digest(&mut e, key_package);
                put_digest(&mut e, subject);
                put_digest(&mut e, welcome);
                put_digest(&mut e, expected_founder);
            }
        }
        e.finish()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(ENVELOPE_DOMAIN).expect("fixed domain");
        e.put_bytes(&self.semantic_bytes()).expect("bounded fact");
        e.put_bytes(&self.signature).expect("fixed signature");
        e.finish()
    }

    /// Syntax only. Import must verify every signature before duplicate/capacity handling.
    /// This parser never constructs a pin, a membership observation or an authority witness.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_REMOVAL_BYTES {
            return Err(ChatError::Bound);
        }
        let mut outer = Decoder::new(bytes);
        if outer.get_bytes()? != ENVELOPE_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let semantic = outer.get_bytes()?;
        let signature = get_signature(&mut outer)?;
        outer.finish()?;
        let mut d = Decoder::new(semantic);
        if d.get_bytes()? != FACT_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let kind = d.get_u8()?;
        if !(1..=7).contains(&kind) {
            return Err(ChatError::Malformed);
        }
        if kind != 6 && bytes.len() > MAX_SMALL_FACT_BYTES {
            return Err(ChatError::Bound);
        }
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        if d.get_u16()? != PROFILE {
            return Err(ChatError::Scope);
        }
        let profile = if kind == 1 {
            None
        } else {
            Some(OwnerFactId(get_digest(&mut d)?))
        };
        let issuer = get_digest(&mut d)?;
        let body = decode_body(kind, &mut d)?;
        d.finish()?;
        let fact = Self {
            spec: OwnerFactSpec {
                group: group.to_vec(),
                profile,
                body,
            },
            issuer,
            signature,
            id: semantic_id(semantic),
        };
        fact.validate()?;
        if fact.semantic_bytes() != semantic {
            return Err(ChatError::Malformed);
        }
        Ok(fact)
    }
}

fn decode_body(kind: u8, d: &mut Decoder<'_>) -> Result<OwnerFactBody, ChatError> {
    Ok(match kind {
        1 => {
            if d.get_str()? != TRUST_RULE {
                return Err(ChatError::Authority);
            }
            let creation = get_digest(d)?;
            let founder = get_digest(d)?;
            if d.get_u8()? != 0 || d.get_str()? != RESOURCE_PROFILE {
                return Err(ChatError::Authority);
            }
            OwnerFactBody::Profile { creation, founder }
        }
        2 => {
            let subject = get_digest(d)?;
            let origin = match d.get_u8()? {
                0 => AdmissionOrigin::Founder {
                    creation: get_digest(d)?,
                },
                1 => AdmissionOrigin::Add {
                    commit: get_digest(d)?,
                    before_epoch: d.get_u64()?,
                    after_epoch: d.get_u64()?,
                    key_package: get_digest(d)?,
                    credential: get_digest(d)?,
                },
                _ => return Err(ChatError::Malformed),
            };
            OwnerFactBody::Admission {
                subject,
                origin,
                predecessor: OwnerFactId(get_digest(d)?),
            }
        }
        3 => OwnerFactBody::RealmGrant {
            channel: d.get_u128()?,
            admission: OwnerFactId(get_digest(d)?),
            subject: get_digest(d)?,
            rights: Rights::from_bits(d.get_u16()?)?,
        },
        4 => {
            let channel = d.get_u128()?;
            let admission = OwnerFactId(get_digest(d)?);
            let grant = GrantId::from_bytes(get_digest(d)?);
            let subject = get_digest(d)?;
            let count = usize::from(d.get_u16()?);
            if count > MAX_BASIS_COVERAGE || count * 36 > d.remaining() {
                return Err(ChatError::Bound);
            }
            let mut covered = Vec::with_capacity(count);
            for _ in 0..count {
                covered.push(BasisId::from_bytes(get_digest(d)?));
            }
            OwnerFactBody::Basis {
                channel,
                admission,
                grant,
                subject,
                covered,
            }
        }
        5 => {
            let channel = d.get_u128()?;
            let action = ActionId::from_bytes(get_digest(d)?);
            let admission = OwnerFactId(get_digest(d)?);
            let subject = get_digest(d)?;
            let grant = GrantId::from_bytes(get_digest(d)?);
            let basis = BasisId::from_bytes(get_digest(d)?);
            let create = match d.get_u8()? {
                0 => None,
                1 => Some(ActionId::from_bytes(get_digest(d)?)),
                _ => return Err(ChatError::Malformed),
            };
            OwnerFactBody::Acceptance {
                channel,
                action,
                admission,
                subject,
                grant,
                basis,
                create,
            }
        }
        6 => {
            let admission = OwnerFactId(get_digest(d)?);
            let subject = get_digest(d)?;
            let commit = get_digest(d)?;
            let before_epoch = d.get_u64()?;
            let after_epoch = d.get_u64()?;
            let count = usize::from(d.get_u16()?);
            if count > MAX_CUT_ACTIONS {
                return Err(ChatError::Bound);
            }
            let packed = d.get_bytes()?;
            if packed.len() != count * 32 {
                return Err(ChatError::Malformed);
            }
            let mut accepted = Vec::with_capacity(count);
            for chunk in packed.chunks_exact(32) {
                accepted.push(ActionId::from_bytes(
                    chunk.try_into().expect("exact chunks"),
                ));
            }
            OwnerFactBody::Removal {
                admission,
                subject,
                commit,
                before_epoch,
                after_epoch,
                accepted,
            }
        }
        7 => OwnerFactBody::JoinBootstrap {
            key_package: get_digest(d)?,
            subject: get_digest(d)?,
            welcome: get_digest(d)?,
            expected_founder: get_digest(d)?,
        },
        _ => return Err(ChatError::Malformed),
    })
}
fn check_next_epoch(before: u64, after: u64) -> Result<(), ChatError> {
    if before.checked_add(1) != Some(after) {
        return Err(ChatError::Scope);
    }
    Ok(())
}
fn strict_order<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}
fn put_digest(e: &mut Encoder, digest: &[u8; 32]) {
    e.put_bytes(digest).expect("fixed digest");
}
fn get_digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn get_signature(d: &mut Decoder<'_>) -> Result<[u8; 64], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn hash_label(domain: &[u8], semantic: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(domain);
    h.update(semantic);
    *h.finalize().as_bytes()
}
fn semantic_id(semantic: &[u8]) -> OwnerFactId {
    OwnerFactId(hash_label(ID_DOMAIN, semantic))
}

#[cfg(test)]
mod tests;
