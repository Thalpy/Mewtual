//! First founder-local Create DATA producer. The actual operational Root Current, original
//! request custody, RetainA/AppendB/index and covering-save caller are still being paired.
//! No App authority, governor, transport frame dispatcher or publication follows from this
//! signature/encryption. The original closed SDK graph and immutable S0 are never mutated.
//! Root's authenticated reachable token -> exact original A/cipher locator survives restart;
//! a missing/corrupt committed object is Incomplete, never absence or permission to re-sign.
use super::*;
use catcoms_crypto::{seal, SealedBlob};
use catcoms_replication::chat::{
    ActionSpec, Body, ChatError, Realm, SignedAction, MAX_GROUP_ID_BYTES, MAX_TEXT_BYTES,
};
use catcoms_storage::pad::{self, OP_PAD_CEILING, OP_PAD_FLOOR};
use catcoms_wire::{DocType, Encoder};

const ENVELOPE_DOMAIN: &[u8] = b"mewtual/chat/ordinary-envelope/v2\0";
const KEY_DOMAIN: &[u8] = b"mewtual/chat/ordinary-key/v2\0";
const VERSION: u16 = 2;
/// Create/no-reply/empty-parents only; the existing action/v1 grammar is unchanged.
pub const MAX_CREATE_ACTION_BYTES: usize = 66_057;
pub const MAX_CREATE_ENVELOPE_BYTES: usize = 66_457;
pub const MAX_CREATE_PADDED_BYTES: usize = 131_076;
pub const MAX_CREATE_CIPHERTEXT_BYTES: usize = 131_092;
/// Nonce24 + ciphertext/tag16, before any independently paid Root debt framing/copies.
pub const MAX_CREATE_ENCRYPTED_PAYLOAD_BYTES: usize = 24 + MAX_CREATE_CIPHERTEXT_BYTES;
/// Named requested allocations only, NOT an SDK lease, a physical RSS bound or proof of
/// payment. The actual Root caller must reserve this separate class BEFORE invoking us and
/// retain it through all pending/complete bytes. Never borrow the SDK decode/source fee.
pub const CREATE_DATA_BUFFER_BYTES: usize = 1024 * 1024;

/// Local exact-retry input DATA. Nothing here selects a Profile, Admission, epoch, key,
/// nonce/time, global parent or Current source. UTF8 length is checked before any copy/work.
pub struct V2CreateRequest<'a> {
    pub token: [u8; 32],
    pub channel: u128,
    pub text: &'a str,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum V2CreateRefusal {
    Bound,
    Source,
    RequestMismatch,
    Terminal,
}

// Mark BEFORE every external seam/crypto call. An unwind or lost partial result cannot
// silently reinitialize this same request. No clock/RNG/sign/export/seal occurs on Complete.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Captured,
    Clock,
    Time,
    Nonce,
    Draft,
    Sign,
    Signed,
    Export,
    Seal,
    Complete,
    Transferred,
    Failed,
}
struct CreateBody {
    token: [u8; 32],
    channel: u128,
    text: Zeroizing<String>,
    original: OriginalV2GenesisSeedObservation,
    scope: OriginalInitializationObservation,
    context: BootstrapSourceContextData,
    stage: Stage,
    time: Option<u64>,
    nonce: Option<[u8; 16]>,
    action: Option<SignedAction>,
    encoded: Zeroizing<Vec<u8>>,
    sealed: Option<SealedBlob>,
    packet_id: Option<V2CreatePacketId>,
    transferred: Option<V2CreatePacketId>,
}
/// Own the same pending request on EVERY refusal. Root retains this before work and must
/// move complete bytes plus their actual fee into inert original custody BEFORE TxnA I/O.
/// There is no bytes/source setter, caller signer, new-RNG retry or decoded pending adopter.
pub struct PreparedV2CreateData {
    body: Box<CreateBody>,
}

/// Complete exact PRIVATE packet identity, distinct from semantic ActionId. It also binds
/// local source/request and the actual original ciphertext. DATA, never a Saved receipt.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct V2CreatePacketId([u8; 32]);
impl V2CreatePacketId {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
/// One-time owned DATA for original Root custody BEFORE any fallible TxnA I/O. Immutable
/// getters intentionally survive original node Drop for exact already-admitted negative
/// repair. No sender, observer, key, node/graph, Store or publication permission is retained.
pub struct DetachedV2CreateData {
    id: V2CreatePacketId,
    token: [u8; 32],
    channel: u128,
    action: SignedAction,
    encoded: Zeroizing<Vec<u8>>,
    sealed: SealedBlob,
}
/// Borrowed immutable DATA only. Root cannot expose/project/publish it before its separate
/// complete accepted-index/AppendB/public-head/manifest/publication-debt covering save.
pub struct V2CreateDataView<'a> {
    pub token: [u8; 32],
    pub action: &'a SignedAction,
    pub action_bytes: &'a [u8],
    pub channel: u128,
    pub epoch: u64,
    pub nonce: &'a [u8; 24],
    pub ciphertext: &'a [u8],
}
macro_rules! debug_private {
    ($($ty:ty),+ $(,)?) => {$ (
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
debug_private!(
    V2CreateRequest<'_>,
    PreparedV2CreateData,
    V2CreateDataView<'_>,
    V2CreatePacketId,
    DetachedV2CreateData
);

// Deliberately overpay Encoder's requested doubling and the AEAD tag-growth case. The pinned
// aead0.5.2 actually reserves P+tag up front. Keep ALL additional text/envelope clones in this
// inventory when changing this code; original host T/R/Clock and Root WAL copies are separate.
const BUFFERS: usize = 2 * MAX_TEXT_BYTES
    + 2 * MAX_CREATE_ACTION_BYTES
    + 2 * (MAX_CREATE_ACTION_BYTES - 72)
    + MAX_CREATE_ENVELOPE_BYTES
    + MAX_CREATE_PADDED_BYTES
    + 2 * MAX_CREATE_CIPHERTEXT_BYTES
    + 6 * MAX_GROUP_ID_BYTES
    + 16 * 1024;
const LAYOUT: usize = std::mem::size_of::<CreateBody>()
    + std::mem::size_of::<PreparedV2CreateData>()
    + std::mem::size_of::<DetachedV2CreateData>()
    + std::mem::size_of::<ActionSpec>()
    + std::mem::size_of::<SignedAction>()
    + std::mem::size_of::<V2CreateDataView<'static>>()
    + 3 * std::mem::size_of::<Encoder>()
    + 2 * std::mem::size_of::<blake3::Hasher>()
    + 64; // separately paid tiny weak seed identity/control block
const _: () = assert!(BUFFERS + LAYOUT <= CREATE_DATA_BUFFER_BYTES);
const _: () = assert!(ENVELOPE_DOMAIN.len() == 34 && KEY_DOMAIN.len() == 29);

pub(super) fn source<T, R>(node: &UnboundV2GenesisNode<T, R>) -> Result<(), V2CreateRefusal> {
    let s = &node.seed.body;
    let spec = s.context.spec_data();
    if !s.metadata.scope.genesis_attachment_live()
        || s.group.epoch() != 0
        || !s.group.is_active()
        || s.group.group_id_ref() != spec.group
        || s.device.public_key_bytes().as_slice() != spec.signer.as_slice()
        || s.group.member_count() != 1
        || !s.group.has_local_signer(&spec.signer)
        || !s.group.is_designated_committer(&s.device)
        || s.genesis.profile.spec().founder != spec.signer
        || s.genesis.admission.spec().subject != spec.signer
    {
        return Err(V2CreateRefusal::Source);
    }
    let policy = s.metadata.policy.as_ref().ok_or(V2CreateRefusal::Source)?;
    crate::group_policy::require_supported(policy).map_err(|_| V2CreateRefusal::Source)?;
    match (&node._parts.body.warm, &node._parts.body.fresh) {
        (None, None) => Ok(()),
        (Some(warm), Some(fresh))
            if warm.source == s.source
                && warm.context == s.context
                && warm
                    .fresh
                    .matches_genesis_restored(fresh, &s.group, &spec.signer)
                && warm.scope.matches_genesis_attachment(&s.metadata.scope) =>
        {
            Ok(())
        }
        _ => Err(V2CreateRefusal::Source),
    }
}
impl CreateBody {
    fn complete_shape(&self) -> Result<(), V2CreateRefusal> {
        let sealed = self.sealed.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        if self.action.is_none()
            || self.encoded.is_empty()
            || self.encoded.len() > MAX_CREATE_ACTION_BYTES
            || self.encoded.capacity() > 2 * MAX_CREATE_ACTION_BYTES
            || sealed.ciphertext.len() < 16
            || sealed.ciphertext.len() > MAX_CREATE_CIPHERTEXT_BYTES
            || sealed.ciphertext.capacity() > 2 * MAX_CREATE_CIPHERTEXT_BYTES
        {
            return Err(V2CreateRefusal::Bound);
        }
        Ok(())
    }
    fn require<T, R>(&self, node: &UnboundV2GenesisNode<T, R>) -> Result<(), V2CreateRefusal> {
        require_original(node, &self.original, &self.scope, &self.context)
    }
    fn same_request(&self, input: &V2CreateRequest<'_>) -> bool {
        self.token == input.token
            && self.channel == input.channel
            && self.text.as_str() == input.text
    }
    fn check<T, R>(&mut self, node: &UnboundV2GenesisNode<T, R>) -> Result<(), V2CreateRefusal> {
        if let Err(error) = self.require(node) {
            self.stage = Stage::Failed;
            return Err(error);
        }
        Ok(())
    }
}

pub(super) fn require_original<T, R>(
    node: &UnboundV2GenesisNode<T, R>,
    original: &OriginalV2GenesisSeedObservation,
    scope: &OriginalInitializationObservation,
    context: &BootstrapSourceContextData,
) -> Result<(), V2CreateRefusal> {
    source(node)?;
    original
        .require_same_seed_data(&node.seed)
        .map_err(|_| V2CreateRefusal::Source)?;
    if context != &node.seed.body.context || !scope.matches(&node.seed.body.metadata.scope) {
        return Err(V2CreateRefusal::Source);
    }
    Ok(())
}

impl<T: MeshTransport, R: CryptoRngCore> UnboundV2GenesisNode<T, R> {
    pub fn prepare_create_data(
        &mut self,
        input: V2CreateRequest<'_>,
        slot: &mut Option<PreparedV2CreateData>,
    ) -> Result<(), V2CreateRefusal> {
        if slot
            .as_ref()
            .is_some_and(|p| p.body.stage == Stage::Transferred)
        {
            return Err(V2CreateRefusal::Terminal);
        }
        if input.text.is_empty() || input.text.len() > MAX_TEXT_BYTES {
            return Err(V2CreateRefusal::Bound);
        }
        if let Some(original) = slot {
            if !original.body.same_request(&input) {
                return Err(V2CreateRefusal::RequestMismatch);
            }
            // A foreign/equal-restored source must not mutate the ORIGINAL pending slot.
            original.body.require(self)?;
        } else {
            source(self)?;
            let mut text = String::with_capacity(input.text.len());
            text.push_str(input.text);
            let scope = self
                .seed
                .body
                .metadata
                .scope
                .observe_private_initialization()
                .map_err(|_| V2CreateRefusal::Source)?;
            *slot = Some(PreparedV2CreateData {
                body: Box::new(CreateBody {
                    token: input.token,
                    channel: input.channel,
                    text: Zeroizing::new(text),
                    original: self.seed.observe_original_seed_data(),
                    scope,
                    context: self.seed.body.context.clone(),
                    stage: Stage::Captured,
                    time: None,
                    nonce: None,
                    action: None,
                    encoded: Zeroizing::new(Vec::new()),
                    sealed: None,
                    packet_id: None,
                    transferred: None,
                }),
            });
        }
        let p = &mut slot.as_mut().ok_or(V2CreateRefusal::Terminal)?.body;
        if p.stage == Stage::Complete {
            p.complete_shape()?;
            return p.require(self);
        }
        if !matches!(
            p.stage,
            Stage::Captured | Stage::Time | Stage::Draft | Stage::Signed
        ) {
            return Err(V2CreateRefusal::Terminal);
        }
        // Allocation/observer setup may outlast the earlier source observation. Retain
        // the new pending owner, then repeat originals BEFORE the first clock callback.
        p.check(self)?;
        if p.stage == Stage::Captured {
            p.stage = Stage::Clock;
            #[cfg(test)]
            note(0);
            p.time = Some(self._parts.body._clock.now_ms());
            p.stage = Stage::Time;
            #[cfg(test)]
            boundary(0);
            p.check(self)?;
        }
        if p.stage == Stage::Time {
            p.stage = Stage::Nonce;
            let mut nonce = [0; 16];
            #[cfg(test)]
            note(1);
            self._parts.body._rng.fill_bytes(&mut nonce);
            p.nonce = Some(nonce);
            p.stage = Stage::Draft;
            #[cfg(test)]
            boundary(1);
            p.check(self)?;
        }
        if p.stage == Stage::Draft {
            let realm = Realm::new(self.seed.body.group.group_id_ref(), p.channel)
                .map_err(|_| V2CreateRefusal::Bound)?;
            let labels = self
                .seed
                .body
                .genesis
                .admission
                .ordinary_labels(&realm)
                .map_err(|_| V2CreateRefusal::Source)?;
            let mut text = String::with_capacity(p.text.len());
            text.push_str(&p.text);
            let spec = ActionSpec {
                realm,
                basis: labels.basis,
                grant: labels.grant,
                nonce: p.nonce.ok_or(V2CreateRefusal::Terminal)?,
                presented_ms: p.time.ok_or(V2CreateRefusal::Terminal)?,
                parents: Vec::new(),
                body: Body::Create {
                    text,
                    reply_to: None,
                },
            };
            p.check(self)?;
            p.stage = Stage::Sign;
            #[cfg(test)]
            note(2);
            let action = SignedAction::sign_using(
                spec,
                self.seed.body.context.spec_data().signer,
                |bytes| {
                    self.seed
                        .body
                        .device
                        .sign(bytes)
                        .map_err(|_| ChatError::Signature)
                },
            );
            let action = match action {
                Ok(action) => action,
                Err(_) => {
                    p.stage = Stage::Failed;
                    return Err(V2CreateRefusal::Terminal);
                }
            };
            // Cache the actual valid signature BEFORE any later encode/source error.
            p.action = Some(action);
            p.stage = Stage::Signed;
            #[cfg(test)]
            boundary(2);
            p.check(self)?;
        }
        // An unwind AFTER signature custody cannot re-sign. Finish only deterministic
        // encoding on retry; no empty canonical action is ever admitted to AEAD.
        if p.stage == Stage::Signed && p.encoded.is_empty() {
            p.encoded =
                Zeroizing::new(p.action.as_ref().ok_or(V2CreateRefusal::Terminal)?.encode());
            if p.encoded.is_empty()
                || p.encoded.len() > MAX_CREATE_ACTION_BYTES
                || p.encoded.capacity() > 2 * MAX_CREATE_ACTION_BYTES
            {
                p.stage = Stage::Failed;
                return Err(V2CreateRefusal::Bound);
            }
        }
        p.check(self)?;
        p.stage = Stage::Export;
        #[cfg(test)]
        note(3);
        let base = self.seed.body.group.channel_secret(
            &self.seed.body.device,
            DocType::Channel,
            p.channel,
        );
        let base = match base {
            Ok(key) => Zeroizing::new(key),
            Err(_) => {
                p.stage = Stage::Failed;
                return Err(V2CreateRefusal::Terminal);
            }
        };
        #[cfg(test)]
        boundary(3);
        p.check(self)?;
        let action = p.action.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        let key = key_data(
            &base,
            self.seed.body.genesis.profile.id().as_bytes(),
            &action.spec().realm,
            0,
        );
        let plain = envelope_data(self, action, &p.encoded)?;
        let padded = Zeroizing::new(
            pad::pad(&plain, OP_PAD_FLOOR, OP_PAD_CEILING).map_err(|_| V2CreateRefusal::Bound)?,
        );
        if padded.len() > MAX_CREATE_PADDED_BYTES || padded.capacity() > MAX_CREATE_PADDED_BYTES {
            p.stage = Stage::Failed;
            return Err(V2CreateRefusal::Bound);
        }
        p.check(self)?;
        p.stage = Stage::Seal;
        #[cfg(test)]
        note(4);
        let sealed = seal(&key, &padded, &mut self._parts.body._rng);
        let sealed = match sealed {
            Ok(sealed) => sealed,
            Err(_) => {
                p.stage = Stage::Failed;
                return Err(V2CreateRefusal::Terminal);
            }
        };
        p.sealed = Some(sealed);
        let sealed = p.sealed.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        if sealed.ciphertext.len() > MAX_CREATE_CIPHERTEXT_BYTES
            || sealed.ciphertext.capacity() > 2 * MAX_CREATE_CIPHERTEXT_BYTES
        {
            p.stage = Stage::Failed;
            return Err(V2CreateRefusal::Bound);
        }
        // Bind COMPLETE packet identity while the original source is still available.
        // A later negative must not force a source callback/hash during inert extraction.
        p.packet_id = Some(packet_id_data(self.seed.body.source, p));
        p.stage = Stage::Complete;
        #[cfg(test)]
        boundary(4);
        drop(padded);
        drop(plain);
        drop(key);
        drop(base);
        p.check(self)
    }
}

fn key_data(base: &[u8; 32], profile: &[u8; 32], realm: &Realm, epoch: u64) -> Zeroizing<[u8; 32]> {
    let mut h = blake3::Hasher::new_keyed(base);
    h.update(KEY_DOMAIN);
    h.update(profile);
    h.update(realm.id().as_bytes());
    h.update(&epoch.to_be_bytes());
    Zeroizing::new(*h.finalize().as_bytes())
}
fn envelope_data<T, R>(
    node: &UnboundV2GenesisNode<T, R>,
    action: &SignedAction,
    encoded: &[u8],
) -> Result<Zeroizing<Vec<u8>>, V2CreateRefusal> {
    let mut e = Encoder::with_capacity(MAX_CREATE_ENVELOPE_BYTES);
    let s = &node.seed.body;
    let profile = s.genesis.profile.id();
    let admission = s.genesis.admission.admission_id();
    e.put_bytes(ENVELOPE_DOMAIN)
        .map_err(|_| V2CreateRefusal::Bound)?;
    e.put_u16(VERSION);
    for field in [
        s.group.group_id_ref(),
        profile.as_bytes(),
        admission.as_bytes(),
    ] {
        e.put_bytes(field).map_err(|_| V2CreateRefusal::Bound)?;
    }
    e.put_u64(0).put_u128(action.spec().realm.channel());
    e.put_bytes(encoded).map_err(|_| V2CreateRefusal::Bound)?;
    let out = e.finish();
    if out.len() > MAX_CREATE_ENVELOPE_BYTES || out.capacity() > MAX_CREATE_ENVELOPE_BYTES {
        return Err(V2CreateRefusal::Bound);
    }
    Ok(Zeroizing::new(out))
}
impl PreparedV2CreateData {
    pub fn borrow_data<'a, T, R>(
        &'a self,
        node: &'a UnboundV2GenesisNode<T, R>,
    ) -> Result<V2CreateDataView<'a>, V2CreateRefusal> {
        if self.body.stage != Stage::Complete {
            return Err(V2CreateRefusal::Terminal);
        }
        self.body.complete_shape()?;
        self.body.require(node)?;
        let sealed = self.body.sealed.as_ref().ok_or(V2CreateRefusal::Terminal)?;
        Ok(V2CreateDataView {
            token: self.body.token,
            action: self.body.action.as_ref().ok_or(V2CreateRefusal::Terminal)?,
            action_bytes: &self.body.encoded,
            channel: self.body.channel,
            epoch: 0,
            nonce: &sealed.nonce,
            ciphertext: &sealed.ciphertext,
        })
    }

    /// Check the actual original source, then only infallible field moves.
    /// Root pre-reserves custody and retains the returned complete packet plus actual fee
    /// in Options across registration refusal; it must never require this live getter for
    /// repair after owner Drop. Final B/current/publication guards remain Root-owned.
    pub fn take_for_handoff<T, R>(
        &mut self,
        node: &UnboundV2GenesisNode<T, R>,
    ) -> Result<DetachedV2CreateData, V2CreateRefusal> {
        if self.body.stage != Stage::Complete {
            return Err(V2CreateRefusal::Terminal);
        }
        self.body.complete_shape()?;
        self.body.require(node)?;
        let id = self.body.packet_id.ok_or(V2CreateRefusal::Terminal)?;
        // No allocation/callback/lock or recoverable failure after the successful take.
        let action = self
            .body
            .action
            .take()
            .expect("complete action checked before transfer");
        let sealed = self
            .body
            .sealed
            .take()
            .expect("complete ciphertext checked before transfer");
        let encoded = std::mem::replace(&mut self.body.encoded, Zeroizing::new(Vec::new()));
        self.body.transferred = Some(id);
        self.body.stage = Stage::Transferred;
        Ok(DetachedV2CreateData {
            id,
            token: self.body.token,
            channel: self.body.channel,
            action,
            encoded,
            sealed,
        })
    }

    /// Consuming negative DATA handoff, even after a late cancellation marked Failed.
    /// Complete materialized bytes and their cached packet identity are mandatory. There is
    /// no Node/source/key/export/Clock/RNG parameter or callback, and no reseal or new action.
    /// Refusal returns the SAME owner unchanged. Root retains the same shared Create payment
    /// before invoking this and through detached/residual cleanup; no save is implied here.
    pub fn into_handoff_data(mut self) -> Result<DetachedV2CreateData, (Self, V2CreateRefusal)> {
        if !matches!(self.body.stage, Stage::Complete | Stage::Failed)
            || self.body.packet_id.is_none()
            || self.body.complete_shape().is_err()
        {
            return Err((self, V2CreateRefusal::Terminal));
        }
        let id = self.body.packet_id.expect("checked cached complete packet");
        let action = self.body.action.take().expect("checked complete action");
        let sealed = self
            .body
            .sealed
            .take()
            .expect("checked complete ciphertext");
        let encoded = std::mem::replace(&mut self.body.encoded, Zeroizing::new(Vec::new()));
        Ok(DetachedV2CreateData {
            id,
            token: self.body.token,
            channel: self.body.channel,
            action,
            encoded,
            sealed,
        })
    }
    /// Unit-only negative after Root has retained the COMPLETE detached packet and fee.
    /// It cannot reopen preparation/borrow/take or prove any write/current/publication.
    pub fn validate_transferred_data<T, R>(
        &self,
        node: &UnboundV2GenesisNode<T, R>,
        expected: V2CreatePacketId,
    ) -> Result<(), V2CreateRefusal> {
        if self.body.stage != Stage::Transferred
            || self.body.transferred != Some(expected)
            || self.body.action.is_some()
            || !self.body.encoded.is_empty()
            || self.body.sealed.is_some()
        {
            return Err(V2CreateRefusal::Terminal);
        }
        self.body.require(node)
    }
}

fn packet_id_data(source: BootstrapSourceId, p: &CreateBody) -> V2CreatePacketId {
    let sealed = p.sealed.as_ref().expect("called only after complete seal");
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/v2-create-packet/v1\0");
    hash.update(source.as_bytes());
    hash.update(&p.token);
    hash.update(&p.channel.to_be_bytes());
    hash.update(&(p.encoded.len() as u64).to_be_bytes());
    hash.update(&p.encoded);
    hash.update(&sealed.nonce);
    hash.update(&(sealed.ciphertext.len() as u64).to_be_bytes());
    hash.update(&sealed.ciphertext);
    V2CreatePacketId(*hash.finalize().as_bytes())
}
impl DetachedV2CreateData {
    pub fn id_data(&self) -> V2CreatePacketId {
        self.id
    }
    pub fn view_data(&self) -> V2CreateDataView<'_> {
        V2CreateDataView {
            token: self.token,
            action: &self.action,
            action_bytes: &self.encoded,
            channel: self.channel,
            epoch: 0,
            nonce: &self.sealed.nonce,
            ciphertext: &self.sealed.ciphertext,
        }
    }
}

// Test-only observations/fault boundaries perform no production admission. The hook is
// detached from its RefCell BEFORE invocation, so actual cancellation cannot recurse into
// a held test lock. Every counted boundary still calls the genuine SDK/Clock/RNG/AEAD.
#[cfg(test)]
type BoundaryHook = (usize, Box<dyn FnOnce()>);
#[cfg(test)]
std::thread_local! {
    static ENTRIES: std::cell::Cell<[usize; 5]> = const { std::cell::Cell::new([0; 5]) };
    static AFTER: std::cell::RefCell<Option<BoundaryHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn note(index: usize) {
    ENTRIES.with(|c| {
        let mut a = c.get();
        a[index] += 1;
        c.set(a);
    });
}
#[cfg(test)]
fn boundary(index: usize) {
    let hook = AFTER.with(|cell| {
        let mut h = cell.borrow_mut();
        if h.as_ref().is_some_and(|(i, _)| *i == index) {
            h.take()
        } else {
            None
        }
    });
    if let Some((_, hook)) = hook {
        hook();
    }
}
#[cfg(test)]
pub(super) fn reset_for_test() {
    ENTRIES.with(|c| c.set([0; 5]));
    AFTER.with(|h| {
        h.borrow_mut().take();
    });
}
#[cfg(test)]
pub(super) fn entries_for_test() -> [usize; 5] {
    ENTRIES.with(std::cell::Cell::get)
}
#[cfg(test)]
pub(super) fn after_for_test(index: usize, hook: Box<dyn FnOnce()>) {
    AFTER.with(|h| *h.borrow_mut() = Some((index, hook)));
}
#[cfg(test)]
impl PreparedV2CreateData {
    pub(super) fn payload_sizes_for_test(&self) -> (bool, usize, Option<usize>) {
        (
            self.body.action.is_some(),
            self.body.encoded.len(),
            self.body.sealed.as_ref().map(|s| s.ciphertext.len()),
        )
    }
    pub(super) fn drop_ciphertext_for_test(&mut self) {
        self.body.sealed = None;
    }
}
