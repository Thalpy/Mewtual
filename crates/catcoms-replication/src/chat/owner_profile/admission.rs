use super::*;
use crate::chat::owner_bodies::body_id;
use crate::chat::owner_header::OwnerBodyId;
use crate::chat::{BasisId, GrantId, Realm, SignedAction};

const DOMAIN: &[u8] = b"mewtual/chat/admission/v2\0";
const GRANT_DOMAIN: &[u8] = b"mewtual/chat/ordinary-grant/v2\0";
const BASIS_DOMAIN: &[u8] = b"mewtual/chat/ordinary-basis/v2\0";
pub const MAX_ADMISSION_BYTES: usize =
    4 + DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES + 2 * 36 + 1 + 3 * 36 + 16 + 3;

/// Public observation labels, not an opaque actual MLS Add/creation observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionSpec {
    pub group: Vec<u8>,
    pub profile: OwnerFactId,
    pub subject: [u8; 32],
    pub origin: AdmissionOrigin,
}
impl AdmissionSpec {
    fn validate(&self) -> Result<(), ChatError> {
        if self.group.is_empty() || self.group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        if let AdmissionOrigin::Add {
            before_epoch,
            after_epoch,
            ..
        } = self.origin
        {
            if before_epoch.checked_add(1) != Some(after_epoch) {
                return Err(ChatError::Dependency);
            }
        }
        Ok(())
    }
}
/// Unsigned body DATA. Only the exact selected signed Admission header under an independently
/// pinned Profile/tenure and governing chain can certify it; these helpers do not create that.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmissionBody {
    spec: AdmissionSpec,
}
/// Derived stable labels only, never ordinary authority or cross-author Delete approval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrdinaryLabels {
    pub grant: GrantId,
    pub basis: BasisId,
}
impl AdmissionBody {
    pub fn new(spec: AdmissionSpec) -> Result<Self, ChatError> {
        spec.validate()?;
        Ok(Self { spec })
    }
    pub fn spec(&self) -> &AdmissionSpec {
        &self.spec
    }
    pub fn id(&self) -> OwnerBodyId {
        body_id(&self.encode())
    }
    /// The v2 incarnation label is this exact body CID, not the full device key alone.
    pub fn admission_id(&self) -> OwnerFactId {
        OwnerFactId::from_bytes(*self.id().as_bytes())
    }
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        if !header.verify() {
            return Err(ChatError::Signature);
        }
        if header.spec().kind != HeaderKind::Admission
            || header.spec().group != self.spec.group
            || header.spec().profile != self.spec.profile
            || header.spec().body != self.id()
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    /// Self-consistent Profile/owner/tenure/body binding ONLY. Known-parent/common-prefix
    /// selection and ORIGINAL mounted root provenance remain mandatory before authority use.
    pub fn verify_profile_binding(
        &self,
        profile: &SignedOwnerProfile,
        header: &SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        if matches!(self.spec.origin, AdmissionOrigin::Founder { .. }) {
            return profile.verify_genesis_binding(self, header);
        }
        if !profile.verify() {
            return Err(ChatError::Signature);
        }
        self.verify_header_binding(header)?;
        if self.spec.profile != profile.id()
            || self.spec.group != profile.spec().group
            || header.spec().issuer != profile.spec().founder
            || header.spec().tenure != profile.tenure()
        {
            return Err(ChatError::Scope);
        }
        if header.spec().sequence <= 1 {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
    /// Exact v2 delegation labels for one realm, without owner refresh or parent-coverage cycles.
    /// The later verifier must first authenticate the original v2 Profile/Admission and derive
    /// each ACTUAL action dependency's minimal coverage; no witness is issued by these labels.
    pub fn ordinary_labels(&self, realm: &Realm) -> Result<OrdinaryLabels, ChatError> {
        if realm.group() != self.spec.group.as_slice() {
            return Err(ChatError::Scope);
        }
        let admission = self.admission_id();
        let realm = realm.id();
        let mut grant = Encoder::new();
        grant.put_bytes(GRANT_DOMAIN).expect("fixed grant domain");
        for field in [
            &self.spec.group[..],
            self.spec.profile.as_bytes(),
            admission.as_bytes(),
            &self.spec.subject,
            realm.as_bytes(),
        ] {
            grant.put_bytes(field).expect("bounded field");
        }
        grant.put_u16(MEMBER_RIGHTS).put_u8(1);
        let grant = GrantId::from_bytes(*blake3::hash(grant.as_bytes()).as_bytes());
        let mut basis = Encoder::new();
        basis.put_bytes(BASIS_DOMAIN).expect("fixed basis domain");
        for field in [
            &self.spec.group[..],
            self.spec.profile.as_bytes(),
            admission.as_bytes(),
            &self.spec.subject,
            realm.as_bytes(),
            grant.as_bytes(),
        ] {
            basis.put_bytes(field).expect("bounded field");
        }
        basis.put_u16(MEMBER_RIGHTS).put_u8(1);
        Ok(OrdinaryLabels {
            grant,
            basis: BasisId::from_bytes(*blake3::hash(basis.as_bytes()).as_bytes()),
        })
    }
    /// Real action signature plus exact group/full subject/derived Grant/Basis labels. This
    /// neither authenticates this unsigned Admission nor grants projection or moderation rights.
    /// Its action canonical buffer/work is separate from the490-byte body cap.
    pub fn matches_action_delegation(&self, action: &SignedAction) -> Result<(), ChatError> {
        if !action.verify() {
            return Err(ChatError::Signature);
        }
        let labels = self.ordinary_labels(&action.spec().realm)?;
        if action.signer() != &self.spec.subject
            || action.spec().grant != labels.grant
            || action.spec().basis != labels.basis
        {
            return Err(ChatError::Authority);
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_ADMISSION_BYTES);
        e.put_bytes(DOMAIN).expect("fixed admission domain");
        e.put_bytes(&self.spec.group).expect("validated group");
        e.put_bytes(self.spec.profile.as_bytes())
            .expect("fixed profile");
        e.put_bytes(&self.spec.subject).expect("fixed subject");
        match self.spec.origin {
            AdmissionOrigin::Founder { creation } => {
                e.put_u8(0);
                e.put_bytes(&creation).expect("fixed creation");
            }
            AdmissionOrigin::Add {
                commit,
                before_epoch,
                after_epoch,
                key_package,
                credential,
            } => {
                e.put_u8(1);
                e.put_bytes(&commit).expect("fixed commit");
                e.put_u64(before_epoch).put_u64(after_epoch);
                e.put_bytes(&key_package).expect("fixed KP");
                e.put_bytes(&credential).expect("fixed credential");
            }
        }
        e.put_u16(MEMBER_RIGHTS).put_u8(1);
        e.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_ADMISSION_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != DOMAIN {
            return Err(ChatError::Malformed);
        }
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let profile = OwnerFactId::from_bytes(digest(&mut d)?);
        let subject = digest(&mut d)?;
        let origin = match d.get_u8()? {
            0 => AdmissionOrigin::Founder {
                creation: digest(&mut d)?,
            },
            1 => {
                let commit = digest(&mut d)?;
                let before_epoch = d.get_u64()?;
                let after_epoch = d.get_u64()?;
                if before_epoch.checked_add(1) != Some(after_epoch) {
                    return Err(ChatError::Dependency);
                }
                AdmissionOrigin::Add {
                    commit,
                    before_epoch,
                    after_epoch,
                    key_package: digest(&mut d)?,
                    credential: digest(&mut d)?,
                }
            }
            _ => return Err(ChatError::Malformed),
        };
        if d.get_u16()? != MEMBER_RIGHTS || d.get_u8()? != 1 {
            return Err(ChatError::Authority);
        }
        d.finish()?;
        Ok(Self {
            spec: AdmissionSpec {
                group: group.to_vec(),
                profile,
                subject,
                origin,
            },
        })
    }
}
