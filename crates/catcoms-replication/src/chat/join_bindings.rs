//! Cryptographic DATA bindings for the proposed production V2 join.
//!
//! These frames contain no valid Legacy invite, grant no mounted pin/selected
//! prefix/MLS/Source/native authority, and have no transport registration. Every
//! representation needs its own signature check before duplicate handling.
//! The complete owning join and namespace6 vertical remain separate work.
use super::{ChatError, MAX_GROUP_ID_BYTES};
use catcoms_crypto::verify_with_public_bytes;
use catcoms_wire::{Decoder, Encoder};

mod specs;
pub use specs::{InviteBindingSpecData, JoinRequestBindingSpecData, WelcomeBindingSpecData};

const VERSION: u16 = 2;
const ENVELOPE: &[u8] = b"mewtual/chat/join-binding-envelope/v2\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/join-binding-id/v2\0";
const ANCHOR: &[u8] = b"mewtual/chat/join-genesis-anchor/v2\0";
/// Complete envelope cap, not an accounting or native-admission invoice.
pub const MAX_JOIN_BINDING_BYTES: usize = 1024;
/// Hash input is independently bounded before work. Callers still own/pay for
/// actual network buffers, simultaneous encodings, SDK work and retained DATA.
pub const MAX_JOIN_PAYLOAD_BYTES: usize = 512 * 1024;

/// Complete untrusted genesis selection DATA, with the same fields as Core's
/// FounderGenesisAnchorData. Decoding this is NOT a trusted-HOST constructor.
#[derive(Clone, PartialEq, Eq)]
pub struct GenesisAnchorBindingData {
    pub group: Vec<u8>,
    pub founder: [u8; 32],
    pub profile: [u8; 32],
    pub admission: [u8; 32],
    pub tenure: [u8; 32],
    pub header: [u8; 32],
    pub state: [u8; 32],
}
impl GenesisAnchorBindingData {
    fn validate(&self) -> Result<(), ChatError> {
        if self.group.is_empty() || self.group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        nonzero(&[
            self.founder,
            self.profile,
            self.admission,
            self.tenure,
            self.header,
            self.state,
        ])
    }
    fn encode(&self) -> Vec<u8> {
        let mut e = semantic(ANCHOR);
        field(&mut e, &self.group);
        for value in [
            self.founder,
            self.profile,
            self.admission,
            self.tenure,
            self.header,
            self.state,
        ] {
            field(&mut e, &value);
        }
        e.finish()
    }
    fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        let mut d = input(bytes, ANCHOR)?;
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let founder = fixed(&mut d)?;
        let profile = fixed(&mut d)?;
        let admission = fixed(&mut d)?;
        let tenure = fixed(&mut d)?;
        let header = fixed(&mut d)?;
        let state = fixed(&mut d)?;
        d.finish()?;
        // All framing, widths and EOF precede the only variable-size copy.
        let value = Self {
            group: group.to_vec(),
            founder,
            profile,
            admission,
            tenure,
            header,
            state,
        };
        value.validate()?;
        Ok(value)
    }
    /// Public correlation digest only; not a V2GroupPinId or root provenance.
    pub fn commitment_data(&self) -> Result<[u8; 32], ChatError> {
        self.validate()?;
        Ok(*blake3::hash(&self.encode()).as_bytes())
    }
}

/// Different payload domains must never alias the SDK's KP hash reference or
/// each other. In particular the public route plan is not its sealed transfer.
#[derive(Clone, Copy, Debug)]
pub enum JoinPayloadKindData {
    KeyPackageWire,
    Policy,
    RoutingPlan,
    RoutingTransfer,
    Welcome,
}
impl JoinPayloadKindData {
    /// Hash only the exact bytes in this kind's domain. No parsing, decryption,
    /// SDK validation or authorization follows from a matching digest.
    pub fn commitment_data(self, bytes: &[u8]) -> Result<[u8; 32], ChatError> {
        if bytes.is_empty() || bytes.len() > MAX_JOIN_PAYLOAD_BYTES {
            return Err(ChatError::Bound);
        }
        let tag: &[u8] = match self {
            Self::KeyPackageWire => b"mewtual/chat/join-key-package-wire/v2\0",
            Self::Policy => b"mewtual/chat/join-policy/v2\0",
            Self::RoutingPlan => b"mewtual/chat/join-routing-plan/v2\0",
            Self::RoutingTransfer => b"mewtual/chat/join-routing-transfer/v2\0",
            Self::Welcome => b"mewtual/chat/join-welcome-bytes/v2\0",
        };
        let mut h = blake3::Hasher::new();
        h.update(tag);
        h.update(&(bytes.len() as u64).to_be_bytes());
        h.update(bytes);
        Ok(*h.finalize().as_bytes())
    }
}

fn nonzero(values: &[[u8; 32]]) -> Result<(), ChatError> {
    if values.contains(&[0; 32]) {
        Err(ChatError::Malformed)
    } else {
        Ok(())
    }
}
fn field(e: &mut Encoder, value: &[u8]) {
    e.put_bytes(value).expect("validated small binding field");
}
fn fixed<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn semantic(domain: &[u8]) -> Encoder {
    let mut e = Encoder::with_capacity(MAX_JOIN_BINDING_BYTES);
    field(&mut e, domain);
    e.put_u16(VERSION);
    e
}
fn input<'a>(bytes: &'a [u8], domain: &[u8]) -> Result<Decoder<'a>, ChatError> {
    if bytes.len() > MAX_JOIN_BINDING_BYTES {
        return Err(ChatError::Bound);
    }
    let mut d = Decoder::new(bytes);
    if d.get_bytes()? != domain {
        return Err(ChatError::Malformed);
    }
    if d.get_u16()? != VERSION {
        return Err(ChatError::Scope);
    }
    Ok(d)
}
fn id(semantic: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(ID_DOMAIN);
    h.update(semantic);
    *h.finalize().as_bytes()
}

// Closed named types rather than an extensible public signing/decoder trait.
// The callback shape follows SignedOwnerHeader; it supplies a signature only.
macro_rules! signed_binding {
    ($name:ident, $spec:ident) => {
        #[doc = "Immutable signed DATA only; no current membership or native authority."]
        pub struct $name {
            spec: $spec,
            signature: [u8; 64],
        }
        impl $name {
            /// Validate grammar BEFORE calling existing key custody. The signer
            /// must still own its actual native/source authorization separately.
            pub fn sign_using(
                spec: $spec,
                sign: impl FnOnce(&[u8]) -> Result<[u8; 64], ChatError>,
            ) -> Result<Self, ChatError> {
                spec.validate()?;
                let bytes = spec.semantic_bytes();
                let signature = sign(&bytes)?;
                if !verify_with_public_bytes(spec.signer(), &bytes, &signature) {
                    return Err(ChatError::Signature);
                }
                Ok(Self { spec, signature })
            }
            pub fn spec_data(&self) -> &$spec {
                &self.spec
            }
            /// Semantic identity is not a signature-validity cache key.
            pub fn id_data(&self) -> [u8; 32] {
                id(&self.spec.semantic_bytes())
            }
            pub fn verify(&self) -> bool {
                self.spec.validate().is_ok()
                    && verify_with_public_bytes(
                        self.spec.signer(),
                        &self.spec.semantic_bytes(),
                        &self.signature,
                    )
            }
            pub fn encode(&self) -> Vec<u8> {
                let mut e = Encoder::with_capacity(MAX_JOIN_BINDING_BYTES);
                field(&mut e, ENVELOPE);
                field(&mut e, &self.spec.semantic_bytes());
                field(&mut e, &self.signature);
                e.finish()
            }
            /// Bounded syntax only. Every representation MUST pass verify()
            /// before duplicate/link handling, including one sharing an ID.
            pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
                if bytes.len() > MAX_JOIN_BINDING_BYTES {
                    return Err(ChatError::Bound);
                }
                let mut d = Decoder::new(bytes);
                if d.get_bytes()? != ENVELOPE {
                    return Err(ChatError::Malformed);
                }
                let semantic = d.get_bytes()?;
                let signature = fixed(&mut d)?;
                d.finish()?;
                let spec = $spec::decode_semantic(semantic)?;
                Ok(Self { spec, signature })
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}
signed_binding!(SignedInviteBindingData, InviteBindingSpecData);
signed_binding!(SignedJoinRequestBindingData, JoinRequestBindingSpecData);
signed_binding!(SignedWelcomeBindingData, WelcomeBindingSpecData);

impl SignedInviteBindingData {
    /// Admission-time comparison only. A completed exact Welcome can arrive
    /// later; do not retroactively revoke history by calling this on replay.
    pub fn expired_at_data(&self, now_ms: u64) -> bool {
        now_ms >= self.spec.expires_at_ms
    }
    /// Match public bodies before using their hints. This grants no dial,
    /// policy or membership permission and parses neither body's grammar.
    pub fn check_public_bodies_data(
        &self,
        policy: &[u8],
        routing_plan: &[u8],
    ) -> Result<(), ChatError> {
        if !self.verify() {
            return Err(ChatError::Signature);
        }
        if JoinPayloadKindData::Policy.commitment_data(policy)? != self.spec.policy
            || JoinPayloadKindData::RoutingPlan.commitment_data(routing_plan)?
                != self.spec.public_routing_plan
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}
impl SignedJoinRequestBindingData {
    /// Exact wire bytes only. The DIFFERENT OpenMLS reference/credential and
    /// actual subject still require independent validation by the original SDK.
    pub fn check_key_package_wire_data(&self, bytes: &[u8]) -> Result<(), ChatError> {
        if !self.verify() {
            return Err(ChatError::Signature);
        }
        if JoinPayloadKindData::KeyPackageWire.commitment_data(bytes)? != self.spec.key_package_wire
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    /// Cryptographic DATA links only. Full package/credential/reference and
    /// current inviter authorization require the original owning SDK/host.
    pub fn check_invite_links_data(
        &self,
        invite: &SignedInviteBindingData,
    ) -> Result<(), ChatError> {
        if !self.verify() || !invite.verify() {
            return Err(ChatError::Signature);
        }
        if self.spec.invite != invite.id_data()
            || self.spec.anchor != invite.spec.anchor.commitment_data()?
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}
impl SignedWelcomeBindingData {
    /// Exact opaque response bytes, not successful Welcome processing. Keep
    /// public routing plans and actual sealed routing transfers in distinct domains.
    pub fn check_payloads_data(
        &self,
        welcome: &[u8],
        sealed_routing_transfer: &[u8],
    ) -> Result<(), ChatError> {
        if !self.verify() {
            return Err(ChatError::Signature);
        }
        if JoinPayloadKindData::Welcome.commitment_data(welcome)? != self.spec.welcome
            || JoinPayloadKindData::RoutingTransfer.commitment_data(sealed_routing_transfer)?
                != self.spec.sealed_routing_transfer
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    /// Self-consistent exact Add links only. This does NOT verify selected
    /// parent/state, the actual MLS event, mounted pin, save or delivery.
    pub fn check_links_data(
        &self,
        invite: &SignedInviteBindingData,
        request: &SignedJoinRequestBindingData,
        profile: &super::owner_profile::SignedOwnerProfile,
        admission: &super::owner_profile::AdmissionBody,
        header: &super::owner_header::SignedOwnerHeader,
    ) -> Result<(), ChatError> {
        use super::owner_profile::AdmissionOrigin;
        if !self.verify() {
            return Err(ChatError::Signature);
        }
        request.check_invite_links_data(invite)?;
        admission.verify_profile_binding(profile, header)?;
        let AdmissionOrigin::Add {
            key_package,
            credential,
            ..
        } = admission.spec().origin
        else {
            return Err(ChatError::Scope);
        };
        let anchor = &invite.spec.anchor;
        if self.spec.invite != invite.id_data()
            || self.spec.request_binding != request.id_data()
            || self.spec.anchor != request.spec.anchor
            || self.spec.key_package_wire != request.spec.key_package_wire
            || self.spec.key_package_reference != request.spec.key_package_reference
            || key_package != request.spec.key_package_reference
            || credential != request.spec.credential
            || admission.spec().subject != request.spec.joiner
            || self.spec.add_header != *header.id().as_bytes()
            || self.spec.admission != *admission.admission_id().as_bytes()
            || self.spec.admitter != header.spec().issuer
            || profile.spec().founder != anchor.founder
            || profile.spec().group != anchor.group
            || *profile.id().as_bytes() != anchor.profile
            || *profile.tenure().as_bytes() != anchor.tenure
            || self.spec.policy != invite.spec.policy
            || self.spec.public_routing_plan != invite.spec.public_routing_plan
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
}

macro_rules! redacted_spec_debug {
    ($($ty:ty),* $(,)?) => {$(impl std::fmt::Debug for $ty { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.debug_struct(stringify!($ty)).finish_non_exhaustive() } })*};
}
redacted_spec_debug!(
    GenesisAnchorBindingData,
    InviteBindingSpecData,
    JoinRequestBindingSpecData,
    WelcomeBindingSpecData
);

#[cfg(test)]
mod tests;
