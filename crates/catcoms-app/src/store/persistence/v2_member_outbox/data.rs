//! Member-outbox DATA: bounded wire helpers, identities, stable context and request digests.
//!
//! Nothing here touches the filesystem or grants authority. A context, token or digest is
//! commitment DATA: it names what a stored entry is bound to. It never proves membership, a
//! current Admission, signing permission or publication permission. In particular
//! [`MemberSignedOriginal`] is custody of an already-signed original, not a save admission.
use crate::AppError;
use catcoms_replication::chat::owner_facts::OwnerFactId;
use catcoms_replication::chat::owner_header::OwnerTenureId;
use catcoms_replication::chat::{
    ActionId, ActionSpec, Body, SignedAction, MAX_GROUP_ID_BYTES, MAX_PARENTS, MAX_TEXT_BYTES,
};
use catcoms_wire::{Decoder, Encoder};
use zeroize::Zeroizing;

/// The single on-disk version of every member-outbox format in this module. A future format
/// must choose a new tag/version that old readers refuse, never reinterpret version 1.
pub(super) const VERSION: u16 = 1;

/// The one generic refusal for malformed, foreign or unsupported stored DATA. Typed refusals a
/// caller must act on (pressure, conflict, incomplete state) are [`MemberOutboxRefusal`]s.
pub(super) fn refused() -> AppError {
    AppError::Invalid("Member outbox state is unavailable or unsupported".into())
}

pub(super) fn field<'a>(d: &mut Decoder<'a>, maximum: usize) -> Result<&'a [u8], AppError> {
    let value = d.get_bytes().map_err(|_| refused())?;
    if value.len() > maximum {
        return Err(refused());
    }
    Ok(value)
}

pub(super) fn b32(d: &mut Decoder<'_>) -> Result<[u8; 32], AppError> {
    field(d, 32)?.try_into().map_err(|_| refused())
}

pub(super) fn u8_of(d: &mut Decoder<'_>) -> Result<u8, AppError> {
    d.get_u8().map_err(|_| refused())
}

pub(super) fn u16_of(d: &mut Decoder<'_>) -> Result<u16, AppError> {
    d.get_u16().map_err(|_| refused())
}

pub(super) fn u32_of(d: &mut Decoder<'_>) -> Result<u32, AppError> {
    d.get_u32().map_err(|_| refused())
}

pub(super) fn u64_of(d: &mut Decoder<'_>) -> Result<u64, AppError> {
    d.get_u64().map_err(|_| refused())
}

pub(super) fn put(e: &mut Encoder, value: &[u8]) -> Result<(), AppError> {
    e.put_bytes(value).map_err(|_| refused())?;
    Ok(())
}

/// Write a closed domain tag followed by [`VERSION`].
pub(super) fn tag(e: &mut Encoder, domain: &[u8]) -> Result<(), AppError> {
    put(e, domain)?;
    e.put_u16(VERSION);
    Ok(())
}

/// Start a strict decode: bounded input, exact domain tag and version.
pub(super) fn input<'a>(
    raw: &'a [u8],
    maximum: usize,
    domain: &[u8],
) -> Result<Decoder<'a>, AppError> {
    if raw.is_empty() || raw.len() > maximum {
        return Err(refused());
    }
    let mut d = Decoder::new(raw);
    if field(&mut d, domain.len())? != domain || u16_of(&mut d)? != VERSION {
        return Err(refused());
    }
    Ok(d)
}

/// Strict EOF: a canonical decode maps each byte string to at most one value.
pub(super) fn end(d: Decoder<'_>) -> Result<(), AppError> {
    d.finish().map_err(|_| refused())
}

pub(super) fn finish(e: Encoder, maximum: usize) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if e.is_empty() || e.len() > maximum {
        return Err(refused());
    }
    Ok(Zeroizing::new(e.finish()))
}

macro_rules! member_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub(in crate::store) struct $name([u8; 32]);
        impl $name {
            pub(in crate::store) const fn from_bytes(value: [u8; 32]) -> Self {
                Self(value)
            }
            pub(in crate::store) const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                // Identities are private correlation handles; logs get the type only.
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    };
}

member_id!(
    /// Stable member context identity (the "Cid"). It names the namespace directory and is
    /// bound into every stored object, so an object copied between namespaces is refused.
    /// It is a commitment, not proof that the caller is that member.
    MemberOutboxContextId
);
member_id!(
    /// Caller-chosen request token. Two saves under one token are one request: the first save
    /// wins and every later save or lookup with different request DATA is a conflict.
    MemberRequestToken
);
member_id!(
    /// Digest of the exact request DATA (context, token, channel, parents, body).
    MemberRequestDigest
);
member_id!(
    /// Identity of one exact selected `head.bin` file, including its random nonce. Two heads
    /// with equal plaintext have different IDs, so a stale owner cannot mistake a replacement.
    MemberHeadId
);

const CONTEXT_DOMAIN: &[u8] = b"catcoms/member-outbox-context/v1\0";
const REQUEST_DOMAIN: &[u8] = b"catcoms/member-outbox-request/v1\0";
const HEAD_ID_DOMAIN: &[u8] = b"catcoms/member-outbox-head-id/v1\0";

impl MemberHeadId {
    pub(super) fn of_file(raw: &[u8]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(HEAD_ID_DOMAIN);
        h.update(raw);
        Self(*h.finalize().as_bytes())
    }
}

/// The caller's typed stable context inputs. Every field is DATA supplied by the primary's
/// member-source integration; this module checks only shape. The transport epoch and the
/// per-entry Admission are deliberately absent: an epoch change must not move the namespace,
/// and a rejoin under a new Admission keeps the SAME namespace (its entries record their own
/// original Admission instead).
pub(in crate::store) struct MemberOutboxContextSpec<'a> {
    pub(in crate::store) vault: [u8; 32],
    pub(in crate::store) layout: [u8; 32],
    pub(in crate::store) pin: [u8; 32],
    pub(in crate::store) profile: OwnerFactId,
    pub(in crate::store) tenure: OwnerTenureId,
    pub(in crate::store) group_nonce: [u8; 32],
    pub(in crate::store) device: [u8; 32],
    pub(in crate::store) group: &'a [u8],
}

/// Owned validated stable context. Not Clone: the owner retains the one copy it was opened
/// with, so a later caller cannot swap the context an already-open namespace is checked against.
pub(in crate::store) struct MemberOutboxContextData {
    pub(super) vault: [u8; 32],
    pub(super) layout: [u8; 32],
    pub(super) pin: [u8; 32],
    pub(super) profile: OwnerFactId,
    pub(super) tenure: OwnerTenureId,
    pub(super) group_nonce: [u8; 32],
    pub(super) device: [u8; 32],
    pub(super) group: Vec<u8>,
    pub(super) id: MemberOutboxContextId,
}
impl std::fmt::Debug for MemberOutboxContextData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberOutboxContextData")
            .finish_non_exhaustive()
    }
}
impl MemberOutboxContextData {
    pub(in crate::store) fn from_spec(spec: MemberOutboxContextSpec<'_>) -> Result<Self, AppError> {
        if spec.group.is_empty() || spec.group.len() > MAX_GROUP_ID_BYTES {
            return Err(refused());
        }
        let mut h = blake3::Hasher::new();
        h.update(CONTEXT_DOMAIN);
        h.update(&VERSION.to_be_bytes());
        h.update(&spec.vault);
        h.update(&spec.layout);
        h.update(&spec.pin);
        h.update(spec.profile.as_bytes());
        h.update(spec.tenure.as_bytes());
        h.update(&spec.group_nonce);
        h.update(&spec.device);
        // Length-prefixed so a group suffix can never be confused with a following field.
        h.update(&(spec.group.len() as u32).to_be_bytes());
        h.update(spec.group);
        Ok(Self {
            vault: spec.vault,
            layout: spec.layout,
            pin: spec.pin,
            profile: spec.profile,
            tenure: spec.tenure,
            group_nonce: spec.group_nonce,
            device: spec.device,
            group: spec.group.to_vec(),
            id: MemberOutboxContextId(*h.finalize().as_bytes()),
        })
    }
    pub(in crate::store) fn id(&self) -> MemberOutboxContextId {
        self.id
    }
    /// The namespace directory leaf, `mo-<64 lowercase hex>`. No caller-supplied name exists.
    pub(super) fn namespace_leaf(&self) -> String {
        let mut name = String::with_capacity(67);
        name.push_str("mo-");
        push_hex(&mut name, self.id.as_bytes());
        name
    }
}

pub(super) fn push_hex(out: &mut String, bytes: &[u8; 32]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 15) as usize] as char);
    }
}

/// Request DATA a caller presents to look up an earlier save before authoring a new action.
///
/// `parents` are the request's exact original causal parents. A retry must present the SAME
/// parents it originally authored with; recomputing them from newer tips is a different
/// request and conflicts. This is the spec's "different text/kind/target/parents" rule.
pub(in crate::store) struct MemberRequestData<'a> {
    pub(in crate::store) token: MemberRequestToken,
    pub(in crate::store) channel: u128,
    pub(in crate::store) parents: &'a [ActionId],
    pub(in crate::store) body: &'a Body,
}
impl MemberRequestData<'_> {
    /// Bounded digest. Oversized DATA is refused rather than hashed, so an absurd lookup
    /// cannot be mistaken for a conflict with a real stored request.
    pub(super) fn digest(
        &self,
        context: MemberOutboxContextId,
    ) -> Result<MemberRequestDigest, AppError> {
        request_digest(context, self.token, self.channel, self.parents, self.body)
    }
}

/// The request digest of a SIGNED original: derived from its own signed spec, never from a
/// separately supplied description, so a stored receipt cannot disagree with its action.
pub(super) fn signed_request_digest(
    context: MemberOutboxContextId,
    token: MemberRequestToken,
    spec: &ActionSpec,
) -> Result<MemberRequestDigest, AppError> {
    request_digest(
        context,
        token,
        spec.realm.channel(),
        &spec.parents,
        &spec.body,
    )
}

fn request_digest(
    context: MemberOutboxContextId,
    token: MemberRequestToken,
    channel: u128,
    parents: &[ActionId],
    body: &Body,
) -> Result<MemberRequestDigest, AppError> {
    if parents.len() > MAX_PARENTS {
        return Err(refused());
    }
    let mut h = blake3::Hasher::new();
    h.update(REQUEST_DOMAIN);
    h.update(&VERSION.to_be_bytes());
    h.update(context.as_bytes());
    h.update(token.as_bytes());
    h.update(&channel.to_be_bytes());
    h.update(&(parents.len() as u32).to_be_bytes());
    for parent in parents {
        h.update(parent.as_bytes());
    }
    // Closed per-kind encoding; every variable field is length-prefixed.
    match body {
        Body::Create { text, reply_to } => {
            if text.len() > MAX_TEXT_BYTES {
                return Err(refused());
            }
            h.update(&[1]);
            h.update(&(text.len() as u32).to_be_bytes());
            h.update(text.as_bytes());
            match reply_to {
                None => h.update(&[0]),
                Some(target) => h.update(&[1]).update(target.as_bytes()),
            };
        }
        Body::Edit { create, text } => {
            if text.len() > MAX_TEXT_BYTES {
                return Err(refused());
            }
            h.update(&[2]);
            h.update(create.as_bytes());
            h.update(&(text.len() as u32).to_be_bytes());
            h.update(text.as_bytes());
        }
        Body::Delete { create } => {
            h.update(&[3]).update(create.as_bytes());
        }
        Body::Join { create } => {
            h.update(&[4]).update(create.as_bytes());
        }
    }
    Ok(MemberRequestDigest(*h.finalize().as_bytes()))
}

/// Custody of one exact signed original plus the Admission it was signed under.
///
/// Not Clone. Production construction belongs to the primary's warm-member owner, from the
/// actual original Draft, V2 ordinary signing Plan, its observation and cached raw64
/// (`complete_cached_raw64_data`) plus the Plan's historical Admission ID. That constructor is
/// deliberately NOT in this module. Holding this value is DATA custody only: completing a
/// cached signature after cancellation renews no signing or publication authority, and saving
/// it still needs a separate [`super::transaction::MemberOutboxSaveAdmission`].
pub(in crate::store) struct MemberSignedOriginal {
    action: SignedAction,
    admission: OwnerFactId,
}
impl std::fmt::Debug for MemberSignedOriginal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberSignedOriginal")
            .finish_non_exhaustive()
    }
}
impl MemberSignedOriginal {
    /// Durability-only fixture. It wraps a genuinely signed action for storage tests; it is
    /// cfg(test) and supplies no membership, Admission or signing authority of any kind.
    #[cfg(test)]
    pub(in crate::store) fn durability_fixture_for_test(
        action: SignedAction,
        admission: OwnerFactId,
    ) -> Self {
        Self { action, admission }
    }
    pub(in crate::store) fn action(&self) -> &SignedAction {
        &self.action
    }
    /// The historical Admission the original was signed under. Immutable DATA; it says
    /// nothing about whether that Admission is still current or OPEN.
    pub(in crate::store) fn admission(&self) -> OwnerFactId {
        self.admission
    }
}

/// What a successful save returns and what lookup recovers: the exact receipt of one entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::store) struct MemberReceiptData {
    pub(in crate::store) context: MemberOutboxContextId,
    pub(in crate::store) token: MemberRequestToken,
    pub(in crate::store) ordinal: u64,
    pub(in crate::store) entry: catcoms_storage::private_chat_page::PrivateChatPageId,
    pub(in crate::store) request: MemberRequestDigest,
    pub(in crate::store) action: ActionId,
    pub(in crate::store) original: catcoms_replication::chat::cold_actions::OriginalActionDigest,
    pub(in crate::store) admission: OwnerFactId,
}
