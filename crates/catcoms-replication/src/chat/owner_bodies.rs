//! Strict DATA codecs for the pending scalable owner-evidence/v2 acceptance/removal path.
//!
//! No type here establishes owner provenance, membership, governing-chain selection, historical
//! acceptance, current admission or a covering save. V2 acceptance records have NO standalone
//! signature: the future independently pinned v2 consumer must authenticate their exact CID by
//! inclusion under its selected signed tip, then check ordinary/action-specific authority.
//! Existing v1 action and signed-owner-fact encodings are unchanged and never reinterpreted.
use super::acceptance_tree::{AcceptanceLeaf, Frontier, PreparedAppend, TreeContext, TreeTip};
use super::owner_facts::OwnerFactId;
use super::owner_header::{HeaderKind, OwnerBodyId, OwnerHeaderId, SignedOwnerHeader};
use super::{ActionId, BasisId, ChatError, GrantId, SignedAction, MAX_GROUP_ID_BYTES};
use catcoms_wire::{Decoder, Encoder};

const RECORD_DOMAIN: &[u8] = b"mewtual/chat/acceptance-record/v2\0";
const RECORD_ID_DOMAIN: &[u8] = b"mewtual/chat/acceptance-record-id/v2\0";
const APPEND_DOMAIN: &[u8] = b"mewtual/chat/acceptance-append/v2\0";
const REMOVAL_DOMAIN: &[u8] = b"mewtual/chat/removal/v2\0";
const BODY_ID_DOMAIN: &[u8] = b"mewtual/chat/owner-body-id/v2\0";
const MAX_LEAVES: usize = super::acceptance_tree::MAX_APPEND_LEAVES;
const PACKED_LEAF_BYTES: usize = 16 + 32;

/// Per-encoded-buffer maxima only. These do not pay runtime metadata, actual Vec capacity,
/// proof/node output, decoded SDK heaps, cold pages or any future control/uncertainty reserve.
pub const MAX_ACCEPTANCE_RECORD_BYTES: usize =
    4 + RECORD_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES + 6 * 36 + 16 + 1 + 36;
pub const MAX_APPEND_BODY_BYTES: usize = 4
    + APPEND_DOMAIN.len()
    + 4
    + MAX_GROUP_ID_BYTES
    + 2 * 36
    + 36
    + 2 * (8 + 36)
    + 2
    + 4
    + MAX_LEAVES * PACKED_LEAF_BYTES;
pub const MAX_REMOVAL_BODY_BYTES: usize =
    4 + REMOVAL_DOMAIN.len() + 4 + MAX_GROUP_ID_BYTES + 5 * 36 + 16 + 8 + 36;

/// Untrusted semantic record label, distinct from a signed header/body ID. Public construction
/// grants neither inclusion nor approval; the selected v2 proof grammar must authenticate it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AcceptanceRecordId([u8; 32]);
impl AcceptanceRecordId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceSpec {
    pub group: Vec<u8>,
    pub context: TreeContext,
    pub channel: u128,
    pub action: ActionId,
    pub subject: [u8; 32],
    pub grant: GrantId,
    pub basis: BasisId,
    pub create: Option<ActionId>,
}

/// Immutable unsigned record. Its exact inclusion under an authentic selected tip, not this
/// object's existence or a matching action signature, is what can attest historical acceptance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptanceRecord {
    spec: AcceptanceSpec,
}
impl AcceptanceRecord {
    pub fn new(spec: AcceptanceSpec) -> Result<Self, ChatError> {
        check_group(&spec.group)?;
        Ok(Self { spec })
    }
    pub fn spec(&self) -> &AcceptanceSpec {
        &self.spec
    }
    pub fn id(&self) -> AcceptanceRecordId {
        AcceptanceRecordId(hash(RECORD_ID_DOMAIN, &self.encode()))
    }
    /// Hash-tree label only, not a v1 SignedOwnerFact or a verified historical acceptance.
    pub fn leaf(&self) -> AcceptanceLeaf {
        AcceptanceLeaf {
            channel: self.spec.channel,
            acceptance: OwnerFactId::from_bytes(*self.id().as_bytes()),
        }
    }
    /// Pure exact ACTION binding, including its real signature. Action/v1 does not directly
    /// sign owner Profile/Admission IDs. The separate delegated Grant/Basis verifier must bind
    /// those incarnations; changing context can leave this check true while changing record/
    /// tree identity. This is not historical rights, dependency or Delete-approval verification.
    /// Signature/ID checks use the existing bounded action's canonical buffer; the small record
    /// cap does not pay that action data, verification scratch or the host's per-turn work.
    pub fn matches_action(&self, action: &SignedAction) -> Result<(), ChatError> {
        if !action.verify() {
            return Err(ChatError::Signature);
        }
        let spec = action.spec();
        if spec.realm.group() != self.spec.group.as_slice()
            || spec.realm.channel() != self.spec.channel
            || action.id() != self.spec.action
            || action.signer() != &self.spec.subject
            || spec.grant != self.spec.grant
            || spec.basis != self.spec.basis
            || spec.body.create() != self.spec.create
        {
            return Err(ChatError::Scope);
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_ACCEPTANCE_RECORD_BYTES);
        e.put_bytes(RECORD_DOMAIN).expect("fixed record domain");
        put_group_context(&mut e, &self.spec.group, self.spec.context);
        e.put_u128(self.spec.channel);
        for digest in [
            self.spec.action.as_bytes(),
            &self.spec.subject,
            self.spec.grant.as_bytes(),
            self.spec.basis.as_bytes(),
        ] {
            e.put_bytes(digest).expect("fixed digest");
        }
        e.put_u8(u8::from(self.spec.create.is_some()));
        if let Some(create) = self.spec.create {
            e.put_bytes(create.as_bytes()).expect("fixed Create");
        }
        e.finish()
    }
    /// All syntax is borrowed/checked before the only variable group field is copied.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_ACCEPTANCE_RECORD_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != RECORD_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let (group, context) = group_context(&mut d)?;
        let channel = d.get_u128()?;
        let action = ActionId::from_bytes(digest(&mut d)?);
        let subject = digest(&mut d)?;
        let grant = GrantId::from_bytes(digest(&mut d)?);
        let basis = BasisId::from_bytes(digest(&mut d)?);
        let create = match d.get_u8()? {
            0 => None,
            1 => Some(ActionId::from_bytes(digest(&mut d)?)),
            _ => return Err(ChatError::Malformed),
        };
        d.finish()?;
        Ok(Self {
            spec: AcceptanceSpec {
                group: group.to_vec(),
                context,
                channel,
                action,
                subject,
                grant,
                basis,
                create,
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendSpec {
    pub group: Vec<u8>,
    pub previous_tip: OwnerHeaderId,
    pub old_tip: TreeTip,
    pub new_tip: TreeTip,
    pub leaves: Vec<AcceptanceLeaf>,
}
impl AppendSpec {
    fn validate(&self) -> Result<(), ChatError> {
        check_group(&self.group)?;
        if self.old_tip.context != self.new_tip.context {
            return Err(ChatError::Scope);
        }
        check_tip(self.old_tip)?;
        if self.leaves.is_empty()
            || self.leaves.len() > MAX_LEAVES
            || self.leaves.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(ChatError::Bound);
        }
        check_append_count(self.old_tip.count, self.new_tip.count, self.leaves.len())
    }
}

/// Canonical body data, not a selected/current log tip or cross-batch duplicate lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendBody {
    spec: AppendSpec,
}
impl AppendBody {
    pub fn new(spec: AppendSpec) -> Result<Self, ChatError> {
        spec.validate()?;
        Ok(Self { spec })
    }
    pub fn spec(&self) -> &AppendSpec {
        &self.spec
    }
    pub fn id(&self) -> OwnerBodyId {
        body_id(&self.encode())
    }
    /// Exact signature/kind/group/profile/body binding only; no owner pin, governing previous
    /// tip, frozen-cut policy or resulting public-state commitment is verified here.
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        check_header(
            header,
            HeaderKind::AcceptanceAppend,
            &self.spec.group,
            self.spec.old_tip.context.profile,
            self.id(),
        )
    }
    /// Bounded tree math without authenticating the supplied frontier, old tip dependency or
    /// owner. The actual host must reserve leaf/carry output before calling and persist it
    /// before signed publication. No already-accepted lookup or state mutation is performed.
    pub fn prepare_on_frontier(&self, frontier: &Frontier) -> Result<PreparedAppend, ChatError> {
        if frontier.tip() != self.spec.old_tip {
            return Err(ChatError::Scope);
        }
        let prepared = frontier
            .prepare_append(&self.spec.leaves)
            .map_err(|_| ChatError::Dependency)?;
        if prepared.new_tip() != self.spec.new_tip {
            return Err(ChatError::Dependency);
        }
        Ok(prepared)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_APPEND_BODY_BYTES);
        e.put_bytes(APPEND_DOMAIN).expect("fixed append domain");
        put_group_context(&mut e, &self.spec.group, self.spec.old_tip.context);
        e.put_bytes(self.spec.previous_tip.as_bytes())
            .expect("fixed previous tip");
        put_tip(&mut e, self.spec.old_tip);
        put_tip(&mut e, self.spec.new_tip);
        e.put_u16(self.spec.leaves.len() as u16);
        let mut packed = Vec::with_capacity(self.spec.leaves.len() * PACKED_LEAF_BYTES);
        for leaf in &self.spec.leaves {
            packed.extend_from_slice(&leaf.channel.to_be_bytes());
            packed.extend_from_slice(leaf.acceptance.as_bytes());
        }
        e.put_bytes(&packed).expect("bounded packed leaves");
        e.finish()
    }
    /// The complete count/framing/sort preflight precedes either owned group/leaf allocation.
    /// Raw CIDs and consistent counts never certify leaf content or historical acceptance.
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_APPEND_BODY_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != APPEND_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let (group, context) = group_context(&mut d)?;
        let previous_tip = OwnerHeaderId::from_bytes(digest(&mut d)?);
        let old_tip = tip(&mut d, context)?;
        let new_tip = tip(&mut d, context)?;
        let count = usize::from(d.get_u16()?);
        if count == 0 || count > MAX_LEAVES {
            return Err(ChatError::Bound);
        }
        let packed = d.get_bytes()?;
        if packed.len() != count * PACKED_LEAF_BYTES {
            return Err(ChatError::Malformed);
        }
        let mut previous = None;
        for leaf in leaves(packed) {
            if previous.is_some_and(|known| known >= leaf) {
                return Err(ChatError::Bound);
            }
            previous = Some(leaf);
        }
        check_tip(old_tip)?;
        check_append_count(old_tip.count, new_tip.count, count)?;
        d.finish()?;
        Ok(Self {
            spec: AppendSpec {
                group: group.to_vec(),
                previous_tip,
                old_tip,
                new_tip,
                leaves: leaves(packed).collect(),
            },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalSpec {
    pub group: Vec<u8>,
    pub subject: [u8; 32],
    pub commit: [u8; 32],
    pub before_epoch: u64,
    pub after_epoch: u64,
    pub tip_header: OwnerHeaderId,
    pub frozen_tip: TreeTip,
}
impl RemovalSpec {
    fn validate(&self) -> Result<(), ChatError> {
        check_group(&self.group)?;
        check_tip(self.frozen_tip)?;
        check_epochs(self.before_epoch, self.after_epoch)
    }
}

/// Constant-size frozen-cut DATA. Actual Remove observation, subject/incarnation, selected tip
/// and saved-current control authority still belong to the separate governing consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalBody {
    spec: RemovalSpec,
}
impl RemovalBody {
    pub fn new(spec: RemovalSpec) -> Result<Self, ChatError> {
        spec.validate()?;
        Ok(Self { spec })
    }
    pub fn spec(&self) -> &RemovalSpec {
        &self.spec
    }
    pub fn id(&self) -> OwnerBodyId {
        body_id(&self.encode())
    }
    pub fn verify_header_binding(&self, header: &SignedOwnerHeader) -> Result<(), ChatError> {
        check_header(
            header,
            HeaderKind::Removal,
            &self.spec.group,
            self.spec.frozen_tip.context.profile,
            self.id(),
        )
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::with_capacity(MAX_REMOVAL_BODY_BYTES);
        e.put_bytes(REMOVAL_DOMAIN).expect("fixed removal domain");
        put_group_context(&mut e, &self.spec.group, self.spec.frozen_tip.context);
        e.put_bytes(&self.spec.subject).expect("fixed subject");
        e.put_bytes(&self.spec.commit).expect("fixed Remove commit");
        e.put_u64(self.spec.before_epoch)
            .put_u64(self.spec.after_epoch);
        e.put_bytes(self.spec.tip_header.as_bytes())
            .expect("fixed frozen header");
        put_tip(&mut e, self.spec.frozen_tip);
        e.finish()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, ChatError> {
        if bytes.len() > MAX_REMOVAL_BODY_BYTES {
            return Err(ChatError::Bound);
        }
        let mut d = Decoder::new(bytes);
        if d.get_bytes()? != REMOVAL_DOMAIN {
            return Err(ChatError::Malformed);
        }
        let (group, context) = group_context(&mut d)?;
        let subject = digest(&mut d)?;
        let commit = digest(&mut d)?;
        let before_epoch = d.get_u64()?;
        let after_epoch = d.get_u64()?;
        let tip_header = OwnerHeaderId::from_bytes(digest(&mut d)?);
        let frozen_tip = tip(&mut d, context)?;
        check_tip(frozen_tip)?;
        check_epochs(before_epoch, after_epoch)?;
        d.finish()?;
        Ok(Self {
            spec: RemovalSpec {
                group: group.to_vec(),
                subject,
                commit,
                before_epoch,
                after_epoch,
                tip_header,
                frozen_tip,
            },
        })
    }
}

fn check_group(group: &[u8]) -> Result<(), ChatError> {
    if group.is_empty() || group.len() > MAX_GROUP_ID_BYTES {
        Err(ChatError::Bound)
    } else {
        Ok(())
    }
}
fn check_tip(tip: TreeTip) -> Result<(), ChatError> {
    if tip.count == 0 && tip.root != tip.context.empty_hash() {
        Err(ChatError::Dependency)
    } else {
        Ok(())
    }
}
fn check_append_count(old: u64, new: u64, count: usize) -> Result<(), ChatError> {
    if old.checked_add(count as u64) != Some(new) {
        Err(ChatError::Dependency)
    } else {
        Ok(())
    }
}
fn check_epochs(before: u64, after: u64) -> Result<(), ChatError> {
    if before.checked_add(1) != Some(after) {
        Err(ChatError::Dependency)
    } else {
        Ok(())
    }
}
fn put_group_context(e: &mut Encoder, group: &[u8], context: TreeContext) {
    e.put_bytes(group).expect("validated group");
    e.put_bytes(context.profile.as_bytes())
        .expect("fixed profile");
    e.put_bytes(context.admission.as_bytes())
        .expect("fixed admission");
}
fn group_context<'a>(d: &mut Decoder<'a>) -> Result<(&'a [u8], TreeContext), ChatError> {
    let group = d.get_bytes()?;
    check_group(group)?;
    Ok((
        group,
        TreeContext {
            profile: OwnerFactId::from_bytes(digest(d)?),
            admission: OwnerFactId::from_bytes(digest(d)?),
        },
    ))
}
fn put_tip(e: &mut Encoder, tip: TreeTip) {
    e.put_u64(tip.count);
    e.put_bytes(tip.root.as_bytes()).expect("fixed root");
}
fn tip(d: &mut Decoder<'_>, context: TreeContext) -> Result<TreeTip, ChatError> {
    Ok(TreeTip {
        context,
        count: d.get_u64()?,
        root: super::acceptance_tree::TreeHash::from_bytes(digest(d)?),
    })
}
fn digest(d: &mut Decoder<'_>) -> Result<[u8; 32], ChatError> {
    d.get_bytes()?.try_into().map_err(|_| ChatError::Malformed)
}
fn leaves(packed: &[u8]) -> impl Iterator<Item = AcceptanceLeaf> + '_ {
    packed
        .chunks_exact(PACKED_LEAF_BYTES)
        .map(|chunk| AcceptanceLeaf {
            channel: u128::from_be_bytes(
                chunk[..16].try_into().expect("preflighted packed channel"),
            ),
            acceptance: OwnerFactId::from_bytes(
                chunk[16..].try_into().expect("preflighted packed CID"),
            ),
        })
}
fn hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(domain);
    hash.update(bytes);
    *hash.finalize().as_bytes()
}
fn body_id(bytes: &[u8]) -> OwnerBodyId {
    OwnerBodyId::from_bytes(hash(BODY_ID_DOMAIN, bytes))
}
fn check_header(
    header: &SignedOwnerHeader,
    kind: HeaderKind,
    group: &[u8],
    profile: OwnerFactId,
    body: OwnerBodyId,
) -> Result<(), ChatError> {
    if !header.verify() {
        return Err(ChatError::Signature);
    }
    if header.spec().kind != kind
        || header.spec().group.as_slice() != group
        || header.spec().profile != profile
        || header.spec().body != body
    {
        return Err(ChatError::Scope);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
