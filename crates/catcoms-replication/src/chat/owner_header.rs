//! Bounded signed successor headers for the pending scalable owner-evidence/v2 format.
//!
//! A parsed header and even a valid self-signature are UNTRUSTED public data. This codec cannot
//! select a Profile/root, establish tenure, verify a body/state transition, install a fork or
//! create a Current/publication capability. Those require the independently pinned active host.
//! Existing action/v1 and owner-fact/v1 encodings and consumers are deliberately unchanged.
use super::{owner_facts::OwnerFactId, ChatError, MAX_GROUP_ID_BYTES};
use catcoms_crypto::verify_with_public_bytes;
use catcoms_wire::{Decoder, Encoder};

const HEADER_DOMAIN: &[u8] = b"mewtual/chat/owner-header/v2\0";
const ENVELOPE_DOMAIN: &[u8] = b"mewtual/chat/owner-header-envelope/v2\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/owner-header-id/v2\0";
const VERSION: u16 = 2;

/// Exact maximum unsigned frame: domain, version, bounded group, six32-byte fields, sequence,
/// kind. This cap covers one encoded buffer, not allocation overhead or the actual body bytes.
pub const MAX_HEADER_SEMANTIC_BYTES: usize =
    4 + HEADER_DOMAIN.len() + 2 + 4 + MAX_GROUP_ID_BYTES + 6 * (4 + 32) + 8 + 1;
/// Exact complete envelope maximum. A consumer must also pay simultaneous borrowed/owned
/// metadata, signature verification/encoding scratch and its retained contradiction reserve.
pub const MAX_HEADER_BYTES: usize =
    4 + ENVELOPE_DOMAIN.len() + 4 + MAX_HEADER_SEMANTIC_BYTES + 4 + 64;

macro_rules! label {
    ($name:ident) => {
        /// Fixed public label only. Constructing it authenticates neither content nor authority.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);
        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }
    };
}
label!(OwnerHeaderId);
label!(OwnerBodyId);
label!(OwnerStateCommitment);
label!(OwnerTenureId);

/// Closed successor kinds. This discriminant commits the body's intended grammar; it does not
/// parse or verify that body. Profile/genesis and owner succession are not silently added here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderKind {
    Admission,
    AcceptanceAppend,
    Removal,
    ReviewDecision,
}
impl HeaderKind {
    pub(super) fn tag(self) -> u8 {
        match self {
            Self::Admission => 1,
            Self::AcceptanceAppend => 2,
            Self::Removal => 3,
            Self::ReviewDecision => 4,
        }
    }
    pub(super) fn from_tag(tag: u8) -> Result<Self, ChatError> {
        match tag {
            1 => Ok(Self::Admission),
            2 => Ok(Self::AcceptanceAppend),
            3 => Ok(Self::Removal),
            4 => Ok(Self::ReviewDecision),
            _ => Err(ChatError::Malformed),
        }
    }
}

/// Caller-owned unsigned data, not an authenticated Profile or actual MLS transition observation.
/// The future active consumer must select v2 Profile/tenure from its original immutable root;
/// sharing an OwnerFactId label type never makes a v1 Profile eligible for v2 delegation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderSpec {
    pub group: Vec<u8>,
    pub profile: OwnerFactId,
    pub issuer: [u8; 32],
    pub tenure: OwnerTenureId,
    pub parent: OwnerHeaderId,
    pub sequence: u64,
    pub kind: HeaderKind,
    pub body: OwnerBodyId,
    pub state: OwnerStateCommitment,
}
impl HeaderSpec {
    fn validate(&self) -> Result<(), ChatError> {
        if self.group.is_empty() || self.group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        if self.sequence == 0 {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
}

/// Immutable signed DATA. ID is unsigned semantic identity, not a signature-validity cache key.
/// Every newly received representation must pass its own signature check before duplicate/fork
/// handling, including an invalid-first representation whose ID equals a later valid header.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedOwnerHeader {
    spec: HeaderSpec,
    signature: [u8; 64],
    id: OwnerHeaderId,
}
impl std::fmt::Debug for SignedOwnerHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOwnerHeader")
            .field("id", &self.id)
            .field("kind", &self.spec.kind)
            .field("sequence", &self.spec.sequence)
            .finish_non_exhaustive()
    }
}
impl SignedOwnerHeader {
    /// Sign through existing key custody. Invalid grammar refuses BEFORE calling the signer;
    /// the returned signature must actually verify under this exact full issuer and frame.
    /// A successful callback cannot certify a root, a real MLS event or a covering local save.
    pub fn sign_using(
        spec: HeaderSpec,
        sign: impl FnOnce(&[u8]) -> Result<[u8; 64], ChatError>,
    ) -> Result<Self, ChatError> {
        spec.validate()?;
        let mut header = Self {
            spec,
            signature: [0; 64],
            id: OwnerHeaderId([0; 32]),
        };
        let semantic = header.semantic_bytes();
        header.signature = sign(&semantic)?;
        if !verify_with_public_bytes(&header.spec.issuer, &semantic, &header.signature) {
            return Err(ChatError::Signature);
        }
        header.id = semantic_id(&semantic);
        Ok(header)
    }

    pub fn id(&self) -> OwnerHeaderId {
        self.id
    }
    pub fn spec(&self) -> &HeaderSpec {
        &self.spec
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }

    /// Pure exact-frame signature check. A self-signed foreign Profile/tenure remains foreign;
    /// body existence, state validity and the original mounted owner pin are checked elsewhere.
    pub fn verify(&self) -> bool {
        self.spec.validate().is_ok()
            && verify_with_public_bytes(&self.spec.issuer, &self.semantic_bytes(), &self.signature)
    }

    /// Check BOTH signatures and exact successor geometry. This does not authenticate the
    /// parent/root's provenance or supported tenure, select finality or mutate any cache. The
    /// first/genesis transition and issuer/tenure changes need their separate verified consumer.
    pub fn verify_successor_of(&self, parent: &Self) -> Result<(), ChatError> {
        if !parent.verify() || !self.verify() {
            return Err(ChatError::Signature);
        }
        if self.spec.group != parent.spec.group
            || self.spec.profile != parent.spec.profile
            || self.spec.issuer != parent.spec.issuer
            || self.spec.tenure != parent.spec.tenure
        {
            return Err(ChatError::Scope);
        }
        if self.spec.parent != parent.id
            || parent.spec.sequence.checked_add(1) != Some(self.spec.sequence)
        {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }

    /// Canonical unsigned signing/CID frame, at most520 bytes. Signature variants share these
    /// bytes and ID; their individual validity must still be checked independently.
    pub fn semantic_bytes(&self) -> Vec<u8> {
        semantic_spec(&self.spec)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_HEADER_BYTES);
        e.put_bytes(ENVELOPE_DOMAIN).expect("fixed envelope domain");
        e.put_bytes(&self.semantic_bytes()).expect("bounded header");
        e.put_bytes(&self.signature).expect("fixed signature");
        e.finish()
    }

    /// Strict syntax only, NEVER trusted import. Complete bounded outer/inner framing is
    /// checked using borrowed slices before the sole variable-size field (group) is copied.
    /// Invalid signatures may decode; callers MUST verify each representation before caching.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(ChatError::Bound);
        }
        let mut outer = Decoder::new(bytes);
        if outer.get_bytes()? != ENVELOPE_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let semantic = outer.get_bytes()?;
        if semantic.len() > MAX_HEADER_SEMANTIC_BYTES {
            return Err(ChatError::Bound);
        }
        let signature = outer
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        outer.finish()?;
        let mut d = Decoder::new(semantic);
        if d.get_bytes()? != HEADER_DOMAIN {
            return Err(ChatError::Malformed);
        }
        if d.get_u16()? != VERSION {
            return Err(ChatError::Scope);
        }
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let issuer = digest(&mut d)?;
        let tenure = OwnerTenureId(digest(&mut d)?);
        let parent = OwnerHeaderId(digest(&mut d)?);
        let sequence = d.get_u64()?;
        if sequence == 0 {
            return Err(ChatError::Dependency);
        }
        let kind = HeaderKind::from_tag(d.get_u8()?)?;
        let body = OwnerBodyId(digest(&mut d)?);
        let state = OwnerStateCommitment(digest(&mut d)?);
        d.finish()?;
        Ok(Self {
            spec: HeaderSpec {
                group: group.to_vec(),
                profile,
                issuer,
                tenure,
                parent,
                sequence,
                kind,
                body,
                state,
            },
            signature,
            id: semantic_id(semantic),
        })
    }
}

fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn semantic_id(semantic: &[u8]) -> OwnerHeaderId {
    let mut hash = blake3::Hasher::new();
    hash.update(ID_DOMAIN);
    hash.update(semantic);
    OwnerHeaderId(*hash.finalize().as_bytes())
}

// One canonical encoder for the existing signed path and the opaque pre-sign plan.
// This is not public arbitrary-data signing or a HeaderSpec constructor. Callers have
// already validated the bounded specification; keep all fields here on schema changes.
pub(super) fn semantic_spec(spec: &HeaderSpec) -> Vec<u8> {
    let mut e = Encoder::with_capacity(MAX_HEADER_SEMANTIC_BYTES);
    e.put_bytes(HEADER_DOMAIN).expect("fixed header domain");
    e.put_u16(VERSION);
    e.put_bytes(&spec.group).expect("validated group");
    for digest in [
        spec.profile.as_bytes(),
        &spec.issuer,
        spec.tenure.as_bytes(),
        spec.parent.as_bytes(),
    ] {
        e.put_bytes(digest).expect("fixed digest");
    }
    e.put_u64(spec.sequence).put_u8(spec.kind.tag());
    e.put_bytes(spec.body.as_bytes()).expect("fixed body CID");
    e.put_bytes(spec.state.as_bytes()).expect("fixed state");
    e.finish()
}

#[cfg(test)]
mod tests;
