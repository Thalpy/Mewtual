use super::*;
use catcoms_crypto::{verify_with_public_bytes, DeviceKeypair};
use catcoms_wire::{Decoder, Encoder};

const ACTION_DOMAIN: &[u8] = b"mewtual/chat/action/v1\0";
const REALM_DOMAIN: &[u8] = b"mewtual/chat/realm/v1\0";
const ID_DOMAIN: &[u8] = b"mewtual/chat/action-id/v1\0";
const ENVELOPE_DOMAIN: &[u8] = b"mewtual/chat/envelope/v1\0";

/// Deterministic namespace. Authority/Genesis statement CIDs, owner keys and random
/// cutover nonces deliberately do not choose a different namespace on two replicas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Realm {
    group: Vec<u8>,
    channel: u128,
}
impl Realm {
    pub fn new(group: &[u8], channel: u128) -> Result<Self, ChatError> {
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        Ok(Self {
            group: group.to_vec(),
            channel,
        })
    }
    pub fn group(&self) -> &[u8] {
        &self.group
    }
    pub fn channel(&self) -> u128 {
        self.channel
    }
    pub fn profile(&self) -> u16 {
        PROFILE
    }
    pub fn id(&self) -> RealmId {
        let mut e = Encoder::new();
        e.put_bytes(REALM_DOMAIN).expect("fixed domain");
        self.encode_into(&mut e);
        RealmId(*blake3::hash(e.as_bytes()).as_bytes())
    }
    pub(super) fn encode_into(&self, e: &mut Encoder) {
        e.put_bytes(&self.group).expect("bounded group");
        e.put_u128(self.channel);
        e.put_u16(PROFILE);
    }
    pub(super) fn decode_from(d: &mut Decoder<'_>) -> Result<Self, ChatError> {
        let group = d.get_bytes()?;
        if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
            return Err(ChatError::Bound);
        }
        let channel = d.get_u128()?;
        if d.get_u16()? != PROFILE {
            return Err(ChatError::Scope);
        }
        Self::new(group, channel)
    }
}

/// Closed semantic operations. Join only joins causal knowledge for one exact Create;
/// it changes no text/visibility and permits bounded trees instead of truncating heads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Create {
        text: String,
        reply_to: Option<ActionId>,
    },
    Edit {
        create: ActionId,
        text: String,
    },
    Delete {
        create: ActionId,
    },
    Join {
        create: ActionId,
    },
}
impl Body {
    pub fn create(&self) -> Option<ActionId> {
        match self {
            Self::Create { .. } => None,
            Self::Edit { create, .. } | Self::Delete { create } | Self::Join { create } => {
                Some(*create)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionSpec {
    pub realm: Realm,
    pub basis: BasisId,
    pub grant: GrantId,
    /// Retry metadata is local. Equal nonces with different signed bodies are NOT duplicates.
    /// All-zero is a valid nonce; a nonce grants neither uniqueness nor authorization.
    pub nonce: [u8; 16],
    /// Presentation only. Causal resolution and authorization never compare wall clocks.
    pub presented_ms: u64,
    pub parents: Vec<ActionId>,
    pub body: Body,
}

/// A canonical semantic body plus its exact device signature. A valid signature alone
/// does not establish the subject's historical permission in this realm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedAction {
    spec: ActionSpec,
    signer: [u8; 32],
    signature: [u8; 64],
}
impl SignedAction {
    pub fn sign(key: &DeviceKeypair, spec: ActionSpec) -> Result<Self, ChatError> {
        let mut action = Self {
            spec,
            signer: key.verifying_key().to_bytes(),
            signature: [0; 64],
        };
        action.validate()?;
        action.signature = key.sign(&action.semantic_bytes());
        Ok(action)
    }
    pub fn spec(&self) -> &ActionSpec {
        &self.spec
    }
    pub fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
    pub fn id(&self) -> ActionId {
        let mut h = blake3::Hasher::new();
        h.update(ID_DOMAIN);
        h.update(&self.semantic_bytes());
        ActionId(*h.finalize().as_bytes())
    }
    pub fn envelope_hash(&self) -> EnvelopeHash {
        let mut h = blake3::Hasher::new();
        h.update(ENVELOPE_DOMAIN);
        h.update(&self.encode());
        EnvelopeHash(*h.finalize().as_bytes())
    }
    pub fn verify(&self) -> bool {
        self.validate().is_ok()
            && verify_with_public_bytes(&self.signer, &self.semantic_bytes(), &self.signature)
    }
    /// Canonical unsigned bytes. The semantic ID excludes signature representation, so
    /// two genuinely valid signature variants cannot manufacture a second Create.
    pub fn semantic_bytes(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(ACTION_DOMAIN).expect("fixed domain");
        self.spec.realm.encode_into(&mut e);
        e.put_bytes(&self.signer).expect("fixed signer");
        e.put_bytes(self.spec.basis.as_bytes())
            .expect("fixed basis");
        e.put_bytes(self.spec.grant.as_bytes())
            .expect("fixed grant");
        e.put_bytes(&self.spec.nonce).expect("fixed nonce");
        e.put_u64(self.spec.presented_ms);
        e.put_u16(self.spec.parents.len() as u16);
        for parent in &self.spec.parents {
            e.put_bytes(parent.as_bytes()).expect("fixed parent");
        }
        match &self.spec.body {
            Body::Create { text, reply_to } => {
                e.put_u8(0);
                e.put_str(text).expect("bounded text");
                e.put_u8(u8::from(reply_to.is_some()));
                if let Some(reply) = reply_to {
                    e.put_bytes(reply.as_bytes()).expect("fixed reply");
                }
            }
            Body::Edit { create, text } => {
                e.put_u8(1);
                e.put_bytes(create.as_bytes()).expect("fixed create");
                e.put_str(text).expect("bounded text");
            }
            Body::Delete { create } => {
                e.put_u8(2);
                e.put_bytes(create.as_bytes()).expect("fixed create");
            }
            Body::Join { create } => {
                e.put_u8(3);
                e.put_bytes(create.as_bytes()).expect("fixed create");
            }
        }
        e.finish()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_bytes(&self.semantic_bytes()).expect("bounded action");
        e.put_bytes(&self.signature).expect("fixed signature");
        e.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_ACTION_BYTES {
            return Err(ChatError::Bound);
        }
        let mut outer = Decoder::new(bytes);
        let semantic = outer.get_bytes()?;
        let signature = outer
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        outer.finish()?;
        let mut d = Decoder::new(semantic);
        if d.get_bytes()? != ACTION_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let realm = Realm::decode_from(&mut d)?;
        let signer = array(&mut d)?;
        let basis = BasisId(array(&mut d)?);
        let grant = GrantId(array(&mut d)?);
        let nonce = d
            .get_bytes()?
            .try_into()
            .map_err(|_| ChatError::Malformed)?;
        let presented_ms = d.get_u64()?;
        let count = usize::from(d.get_u16()?);
        if count > MAX_PARENTS {
            return Err(ChatError::Bound);
        }
        let mut parents = Vec::with_capacity(count);
        for _ in 0..count {
            parents.push(ActionId(array(&mut d)?));
        }
        let body = match d.get_u8()? {
            0 => {
                let text = text(&mut d)?;
                let reply_to = match d.get_u8()? {
                    0 => None,
                    1 => Some(ActionId(array(&mut d)?)),
                    _ => return Err(ChatError::Malformed),
                };
                Body::Create { text, reply_to }
            }
            1 => Body::Edit {
                create: ActionId(array(&mut d)?),
                text: text(&mut d)?,
            },
            2 => Body::Delete {
                create: ActionId(array(&mut d)?),
            },
            3 => Body::Join {
                create: ActionId(array(&mut d)?),
            },
            _ => return Err(ChatError::Malformed),
        };
        d.finish()?;
        let action = Self {
            spec: ActionSpec {
                realm,
                basis,
                grant,
                nonce,
                presented_ms,
                parents,
                body,
            },
            signer,
            signature,
        };
        action.validate()?;
        // No alias encodings or future profiles enter a different signature/CID interpretation.
        if action.semantic_bytes() != semantic {
            return Err(ChatError::Malformed);
        }
        Ok(action)
    }
    fn validate(&self) -> Result<(), ChatError> {
        if self.spec.parents.len() > MAX_PARENTS
            || self.spec.parents.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ChatError::Bound);
        }
        match &self.spec.body {
            Body::Create { text, reply_to } => {
                if text.is_empty() || text.len() > MAX_TEXT_BYTES {
                    return Err(ChatError::Bound);
                }
                // A Create has no global chat head frontier: only its exact optional reply.
                if self.spec.parents.as_slice() != reply_to.as_slice() {
                    return Err(ChatError::Dependency);
                }
            }
            Body::Edit { text, .. } if text.len() > MAX_TEXT_BYTES => return Err(ChatError::Bound),
            _ => {}
        }
        if !matches!(self.spec.body, Body::Create { .. }) && self.spec.parents.is_empty() {
            return Err(ChatError::Dependency);
        }
        Ok(())
    }
    pub(super) fn dependencies(&self) -> Vec<ActionId> {
        let mut dependencies = self.spec.parents.clone();
        if let Some(create) = self.spec.body.create() {
            if let Err(at) = dependencies.binary_search(&create) {
                dependencies.insert(at, create);
            }
        }
        dependencies
    }
}
fn array(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn text(d: &mut Decoder<'_>) -> Result<String, ChatError> {
    let text = d.get_str()?;
    if text.len() > MAX_TEXT_BYTES {
        return Err(ChatError::Bound);
    }
    Ok(text.to_owned())
}
