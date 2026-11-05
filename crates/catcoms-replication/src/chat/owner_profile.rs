//! Fixed-founder v2 Profile and stable ordinary Admission labels, not a root acquisition API.
//!
//! All outputs are signed/unsigned DATA. A downloaded Profile cannot choose its own trusted root.
//! Actual fresh provenance, original mounted pin, governing common prefix, semantic dependencies,
//! projection admission and covering save must precede production authority/publication.
//! No opaque positive authority token is constructed by this codec.
use super::owner_facts::OwnerFactId;
use super::owner_header::{HeaderKind, OwnerHeaderId, OwnerTenureId, SignedOwnerHeader};
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_crypto::verify_with_public_bytes;
use catcoms_wire::{Decoder, Encoder};

mod admission;
pub use admission::{
    AdmissionBody, AdmissionOrigin, AdmissionSpec, OrdinaryLabels, MAX_ADMISSION_BYTES,
};

const DOMAIN: &[u8] = b"mewtual/chat/owner-profile/v2\0";
const ENVELOPE_DOMAIN: &[u8] = b"mewtual/chat/owner-profile-envelope/v2\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/owner-profile-id/v2\0";
const TENURE_DOMAIN: &[u8] = b"mewtual/chat/owner-tenure/v2\0";
const GENESIS_DOMAIN: &[u8] = b"mewtual/chat/owner-genesis/v2\0";
// FROZEN wire/delegation meaning for this v2 profile. Expanding the general Member role later
// must not silently expand an existing signature's permissions or change its Grant/Basis IDs.
const MEMBER_RIGHTS: u16 = 23;
/// Current slots only. Historical Admission/Remove/rejoin records have no lifetime64 limit;
/// retaining them requires the separate cold archive and mandatory-control payer before use.
pub const MAX_CURRENT_MEMBERS: u16 = 64;
pub const MAX_PROFILE_SEMANTIC_BYTES: usize =
    4 + DOMAIN.len() + 2 + 4 + MAX_GROUP_ID_BYTES + 2 * 36 + 11;
pub const MAX_PROFILE_BYTES: usize =
    4 + ENVELOPE_DOMAIN.len() + 4 + MAX_PROFILE_SEMANTIC_BYTES + 4 + 64;

/// Untrusted actual-creation label. Only the private producer can establish that this creation
/// really occurred; a signature or a matching hash does not create that runtime observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSpec {
    pub group: Vec<u8>,
    pub founder: [u8; 32],
    pub creation: [u8; 32],
}
impl ProfileSpec {
    fn validate(&self) -> Result<(), ChatError> {
        if self.group.is_empty() || self.group.len() > MAX_GROUP_ID_BYTES {
            Err(ChatError::Bound)
        } else {
            Ok(())
        }
    }
}

/// Canonical self-signed v2 Profile DATA. Its policy is closed: original non-equivocating
/// finalizer, no succession, ordinary MEMBER rights only, member-created realms and exact v2
/// record/tree/header grammar. A v1 root/roster cannot be silently relabelled as this Profile.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedOwnerProfile {
    spec: ProfileSpec,
    signature: [u8; 64],
    id: OwnerFactId,
}
impl std::fmt::Debug for SignedOwnerProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedOwnerProfile")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl SignedOwnerProfile {
    /// Uses the actual existing key custodian without exporting its private key. Invalid
    /// grammar refuses first; a returned signature must verify under the exact full founder.
    pub fn sign_using(
        spec: ProfileSpec,
        sign: impl FnOnce(&[u8]) -> Result<[u8; 64], ChatError>,
    ) -> Result<Self, ChatError> {
        spec.validate()?;
        let mut profile = Self {
            spec,
            signature: [0; 64],
            id: OwnerFactId::from_bytes([0; 32]),
        };
        let semantic = profile.semantic_bytes();
        profile.signature = sign(&semantic)?;
        if !verify_with_public_bytes(&profile.spec.founder, &semantic, &profile.signature) {
            return Err(ChatError::Signature);
        }
        profile.id = OwnerFactId::from_bytes(hash(ID_DOMAIN, &[&semantic]));
        Ok(profile)
    }
    pub fn id(&self) -> OwnerFactId {
        self.id
    }
    pub fn spec(&self) -> &ProfileSpec {
        &self.spec
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
    /// Signature only. Actual independently captured root provenance remains mandatory.
    pub fn verify(&self) -> bool {
        self.spec.validate().is_ok()
            && verify_with_public_bytes(&self.spec.founder, &self.semantic_bytes(), &self.signature)
    }
    /// Deterministic initial-tenure LABEL, not a current-owner/succession capability.
    pub fn tenure(&self) -> OwnerTenureId {
        OwnerTenureId::from_bytes(hash(
            TENURE_DOMAIN,
            &[self.id.as_bytes(), &self.spec.founder, &self.spec.creation],
        ))
    }
    /// Virtual sequence-zero parent LABEL. No sequence-zero header is decoded or trusted.
    pub fn genesis_parent(&self) -> OwnerHeaderId {
        OwnerHeaderId::from_bytes(hash(
            GENESIS_DOMAIN,
            &[
                self.id.as_bytes(),
                self.tenure().as_bytes(),
                &self.spec.founder,
                &self.spec.creation,
            ],
        ))
    }
    /// Complete crypto/data binding for the first Founder Admission and header. Still NOT
    /// mounted root provenance, actual creation, resulting-state validity or a durable save.
    /// In particular, genesis cannot be established from a header without its exact body.
    pub fn verify_genesis_binding(
        &self,
        body: &AdmissionBody,
        header: &SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        if !self.verify() {
            return Err(ChatError::Signature);
        }
        body.verify_header_binding(header)?;
        if body.spec().group != self.spec.group
            || body.spec().profile != self.id
            || body.spec().subject != self.spec.founder
            || header.spec().issuer != self.spec.founder
            || header.spec().tenure != self.tenure()
        {
            return Err(ChatError::Scope);
        }
        if header.spec().sequence != 1
            || header.spec().parent != self.genesis_parent()
            || header.spec().kind != HeaderKind::Admission
        {
            return Err(ChatError::Dependency);
        }
        match body.spec().origin {
            AdmissionOrigin::Founder { creation } if creation == self.spec.creation => Ok(()),
            _ => Err(ChatError::Dependency),
        }
    }
    pub fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_PROFILE_SEMANTIC_BYTES);
        e.put_bytes(DOMAIN).expect("fixed profile domain");
        e.put_u16(2);
        e.put_bytes(&self.spec.group).expect("validated group");
        e.put_bytes(&self.spec.founder).expect("fixed founder");
        e.put_bytes(&self.spec.creation).expect("fixed creation");
        // Acceptance rule1 is LOG inclusion, not the only projection route: body grammar2
        // separately defines exact reviewed approval after a cut, without rewriting that cut.
        e.put_u8(1)
            .put_u8(0)
            .put_u16(MEMBER_RIGHTS)
            .put_u8(1)
            .put_u8(1)
            .put_u8(2)
            .put_u8(2)
            .put_u8(2)
            .put_u16(MAX_CURRENT_MEMBERS);
        e.finish()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_PROFILE_BYTES);
        e.put_bytes(ENVELOPE_DOMAIN)
            .expect("fixed profile envelope");
        e.put_bytes(&self.semantic_bytes())
            .expect("bounded profile");
        e.put_bytes(&self.signature).expect("fixed signature");
        e.finish()
    }
    /// Complete borrowed syntax/policy preflight before the one owned group copy. Invalid
    /// signatures may decode; verify each representation before any duplicate/authority use.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_PROFILE_BYTES {
            return Err(ChatError::Bound);
        }
        let mut outer = Decoder::new(bytes);
        if outer.get_bytes()? != ENVELOPE_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let semantic = outer.get_bytes()?;
        if semantic.len() > MAX_PROFILE_SEMANTIC_BYTES {
            return Err(ChatError::Bound);
        }
        let signature = outer
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        outer.finish()?;
        let mut d = Decoder::new(semantic);
        if d.get_bytes()? != DOMAIN {
            return Err(ChatError::Malformed);
        }
        if d.get_u16()? != 2 {
            return Err(ChatError::Scope);
        }
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let founder = digest(&mut d)?;
        let creation = digest(&mut d)?;
        if d.get_u8()? != 1
            || d.get_u8()? != 0
            || d.get_u16()? != MEMBER_RIGHTS
            || d.get_u8()? != 1
            || d.get_u8()? != 1
            || d.get_u8()? != 2
            || d.get_u8()? != 2
            || d.get_u8()? != 2
            || d.get_u16()? != MAX_CURRENT_MEMBERS
        {
            return Err(ChatError::Authority);
        }
        d.finish()?;
        Ok(Self {
            spec: ProfileSpec {
                group: group.to_vec(),
                founder,
                creation,
            },
            signature,
            id: OwnerFactId::from_bytes(hash(ID_DOMAIN, &[semantic])),
        })
    }
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn hash(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(domain);
    for field in fields {
        hash.update(field);
    }
    *hash.finalize().as_bytes()
}

#[cfg(test)]
mod tests;
