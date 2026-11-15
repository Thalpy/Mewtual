use super::*;
const INVITE: &[u8] = b"mewtual/chat/join-invite/v2\0";
const REQUEST: &[u8] = b"mewtual/chat/join-request/v2\0";
const WELCOME: &[u8] = b"mewtual/chat/join-welcome/v2\0";

/// Caller-owned untrusted invitation fields. Anchor is accepted separately by
/// the actual user/host; neither a signature nor this struct supplies that trust.
#[derive(Clone)]
pub struct InviteBindingSpecData {
    pub anchor: GenesisAnchorBindingData,
    pub inviter: [u8; 32],
    pub nonce: [u8; 16],
    pub expires_at_ms: u64,
    pub policy: [u8; 32],
    pub public_routing_plan: [u8; 32],
}
impl InviteBindingSpecData {
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        self.anchor.validate()?;
        if self.nonce == [0; 16] {
            return Err(ChatError::Malformed);
        }
        nonzero(&[self.inviter, self.policy, self.public_routing_plan])
    }
    pub(super) fn signer(&self) -> &[u8; 32] {
        &self.inviter
    }
    pub(super) fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = semantic(INVITE);
        field(&mut e, &self.anchor.encode());
        field(&mut e, &self.inviter);
        field(&mut e, &self.nonce);
        e.put_u64(self.expires_at_ms);
        field(&mut e, &self.policy);
        field(&mut e, &self.public_routing_plan);
        e.finish()
    }
    pub(super) fn decode_semantic(bytes: &[u8]) -> Result<Self, ChatError> {
        let mut d = input(bytes, INVITE)?;
        let anchor = d.get_bytes()?;
        let inviter = fixed(&mut d)?;
        let nonce = fixed(&mut d)?;
        let expires_at_ms = d.get_u64()?;
        let policy = fixed(&mut d)?;
        let public_routing_plan = fixed(&mut d)?;
        d.finish()?;
        let value = Self {
            anchor: GenesisAnchorBindingData::decode(anchor)?,
            inviter,
            nonce,
            expires_at_ms,
            policy,
            public_routing_plan,
        };
        value.validate()?;
        Ok(value)
    }
}

/// Request-specific commitments. The wire hash, OpenMLS KP reference and MLS
/// credential digest are DIFFERENT domains and must all match the actual SDK.
#[derive(Clone)]
pub struct JoinRequestBindingSpecData {
    pub invite: [u8; 32],
    pub anchor: [u8; 32],
    pub joiner: [u8; 32],
    pub request_token: [u8; 32],
    pub key_package_wire: [u8; 32],
    pub key_package_reference: [u8; 32],
    pub credential: [u8; 32],
}
impl JoinRequestBindingSpecData {
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        nonzero(&self.fields())
    }
    pub(super) fn signer(&self) -> &[u8; 32] {
        &self.joiner
    }
    fn fields(&self) -> [[u8; 32]; 7] {
        [
            self.invite,
            self.anchor,
            self.joiner,
            self.request_token,
            self.key_package_wire,
            self.key_package_reference,
            self.credential,
        ]
    }
    pub(super) fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = semantic(REQUEST);
        for value in self.fields() {
            field(&mut e, &value);
        }
        e.finish()
    }
    pub(super) fn decode_semantic(bytes: &[u8]) -> Result<Self, ChatError> {
        let mut d = input(bytes, REQUEST)?;
        let value = Self {
            invite: fixed(&mut d)?,
            anchor: fixed(&mut d)?,
            joiner: fixed(&mut d)?,
            request_token: fixed(&mut d)?,
            key_package_wire: fixed(&mut d)?,
            key_package_reference: fixed(&mut d)?,
            credential: fixed(&mut d)?,
        };
        d.finish()?;
        value.validate()?;
        Ok(value)
    }
}

/// Admitter signature, not a forwarding endpoint signature. The original public
/// routing plan and ACTUAL sealed transfer stay distinct; no circular invite.
#[derive(Clone)]
pub struct WelcomeBindingSpecData {
    pub invite: [u8; 32],
    pub anchor: [u8; 32],
    pub request_binding: [u8; 32],
    pub key_package_wire: [u8; 32],
    pub key_package_reference: [u8; 32],
    pub admitter: [u8; 32],
    pub add_header: [u8; 32],
    pub admission: [u8; 32],
    pub welcome: [u8; 32],
    pub policy: [u8; 32],
    pub public_routing_plan: [u8; 32],
    pub sealed_routing_transfer: [u8; 32],
}
impl WelcomeBindingSpecData {
    /// Pure bounded unsigned DATA for an owning caller to freeze and durably
    /// cover BEFORE its separately authorized native key edge. This is the
    /// SAME canonical preimage used by SignedWelcomeBindingData::sign_using;
    /// it grants no current membership, key-use or response-release authority.
    pub fn signing_bytes_data(&self) -> Result<Vec<u8>, ChatError> {
        self.validate()?;
        Ok(self.semantic_bytes())
    }
    pub(super) fn validate(&self) -> Result<(), ChatError> {
        nonzero(&self.fields())
    }
    pub(super) fn signer(&self) -> &[u8; 32] {
        &self.admitter
    }
    fn fields(&self) -> [[u8; 32]; 12] {
        [
            self.invite,
            self.anchor,
            self.request_binding,
            self.key_package_wire,
            self.key_package_reference,
            self.admitter,
            self.add_header,
            self.admission,
            self.welcome,
            self.policy,
            self.public_routing_plan,
            self.sealed_routing_transfer,
        ]
    }
    pub(super) fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = semantic(WELCOME);
        for value in self.fields() {
            field(&mut e, &value);
        }
        e.finish()
    }
    pub(super) fn decode_semantic(bytes: &[u8]) -> Result<Self, ChatError> {
        let mut d = input(bytes, WELCOME)?;
        let value = Self {
            invite: fixed(&mut d)?,
            anchor: fixed(&mut d)?,
            request_binding: fixed(&mut d)?,
            key_package_wire: fixed(&mut d)?,
            key_package_reference: fixed(&mut d)?,
            admitter: fixed(&mut d)?,
            add_header: fixed(&mut d)?,
            admission: fixed(&mut d)?,
            welcome: fixed(&mut d)?,
            policy: fixed(&mut d)?,
            public_routing_plan: fixed(&mut d)?,
            sealed_routing_transfer: fixed(&mut d)?,
        };
        d.finish()?;
        value.validate()?;
        Ok(value)
    }
}
