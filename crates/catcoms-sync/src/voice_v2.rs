//! Closed, signed voice-v2 objects. This module is deliberately separate from the released
//! call-signal transcript. It defines no transport, participant state or signing IPC oracle.
//! Native owners must additionally enforce joins, replay, confirmation and lifecycle authority
//! before using a verified object. Signature validity alone is neither liveness nor membership.

use catcoms_crypto::{verify_with_public_bytes, DeviceId, DeviceKeypair};
use catcoms_wire::{Decoder, Encoder, WireError};
use thiserror::Error;

// Public protocol objects deliberately omit even public key/address-bearing ciphertext and
// stable scope identifiers from Debug. Diagnostics use bounded stage/kind counters instead.
macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => { $(impl std::fmt::Debug for $name {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($name)).finish_non_exhaustive()
        }
    })+ };
}
redacted_debug!(
    Scope,
    JoinKey,
    OfferId,
    EncryptedKind,
    EncryptedHeader,
    Body,
    SignedObject,
    FinalScope,
    RoutePermit,
    Envelope,
    Disposition,
    Context
);

pub const MAX_ENVELOPE: usize = 48 * 1024;
/// Canonical ceiling shared with the browser crypto boundary. The former 32 KiB ceiling
/// could not fit even one retained Offer plus Answer verification in the group budget.
/// Native SDP packet factories additionally require the bounded prepaid exchange owner.
/// Encrypted endpoint/action/browser integration remains disabled until all consumers use it.
pub const MAX_SDP_PLAINTEXT: usize = 8 * 1024;
/// Root's route/authority record belongs to each actual lower flight. Reserve it before the
/// source/helper spends an attempt; ordinary-pool pressure must not strand prepaid SDP work.
pub(crate) const ROUTE_METADATA_BYTES: usize = 512;
pub const MAX_PACKET_LIFETIME_MS: u64 = 15_000;
pub const MAX_DELEGATION_LIFETIME_MS: u64 = 30_000;
pub const CLOCK_SKEW_MS: u64 = 5_000;
/// A first-seen remote object can already be issued up to one skew window in the future, then
/// remain valid through a second post-expiry window. Retention owners must account for BOTH.
pub const MAX_REMOTE_PACKET_REMAINING_MS: u64 = MAX_PACKET_LIFETIME_MS + 2 * CLOCK_SKEW_MS;
pub const MAX_REMOTE_DELEGATION_REMAINING_MS: u64 = MAX_DELEGATION_LIFETIME_MS + 2 * CLOCK_SKEW_MS;
pub mod crypto_actions;
pub mod crypto_authority;
pub mod directory;
pub mod helper;
pub mod limits;
pub mod negotiation;
pub mod packets;
pub mod presence;
pub mod resources;
pub mod runtime;
pub mod source;

/// Field-level custody: moving other fields out of a record still retires this exact pending
/// operation on consume, replacement or cache collection. Never move it into historical receipts.
#[derive(Default)]
struct RevocationOwner(CallLease);
impl Drop for RevocationOwner {
    fn drop(&mut self) {
        self.0.revoke();
    }
}

/// Native-owned revocation shared with an actor. Revocation is synchronous and irreversible;
/// lock/remount can close an old actor's authority before waiting for its command loop.
#[derive(Clone)]
pub struct CallLease(std::sync::Arc<LeaseState>);
struct LeaseState {
    live: std::sync::atomic::AtomicBool,
    ledger_claimed: std::sync::atomic::AtomicBool,
    changed: tokio::sync::watch::Sender<bool>,
}
impl Default for CallLease {
    fn default() -> Self {
        Self(std::sync::Arc::new(LeaseState {
            live: std::sync::atomic::AtomicBool::new(true),
            ledger_claimed: std::sync::atomic::AtomicBool::new(false),
            changed: tokio::sync::watch::channel(false).0,
        }))
    }
}
impl CallLease {
    pub fn is_live(&self) -> bool {
        self.0.live.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn revoke(&self) {
        self.0
            .live
            .store(false, std::sync::atomic::Ordering::Release);
        self.0.changed.send_replace(true);
    }
    pub fn is_same(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
    /// Register before checking the irreversible flag. Revocation between a driver's synchronous
    /// admission check and this wait therefore cannot strand a detached request.
    pub(crate) async fn wait_revoked(&self) {
        let mut changed = self.0.changed.subscribe();
        while self.is_live() {
            if changed.changed().await.is_err() {
                break;
            }
        }
    }
    /// One allocator for the whole native join, including across clones or failed handshakes.
    /// The claim is never refunded; losing its ledger revokes the join itself.
    fn claim_packet_ledger(&self) -> Result<(), VoiceError> {
        use std::sync::atomic::Ordering;
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        self.0
            .ledger_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| VoiceError::Unauthorized)?;
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
redacted_debug!(CallLease);

/// Affine custody of one actor/directory-issued child lease across an asynchronous native
/// reply. Dropping a canceled reply revokes the child even though the actor and directory retain
/// revocation handles. Native may retain this owner in an Arc while lending lease clones to
/// workers; neither a worker clone nor a lost invoke waiter can keep an abandoned grant live.
pub struct CallGrant {
    lease: CallLease,
    scope: Scope,
    _custody: Option<(catcoms_rt::SharedRequestKeepalive, resources::Credit)>,
}
impl CallGrant {
    /// Trusted Rust lifecycle adapter only: the lease must already have been issued by the
    /// current actor/directory. This adds reply custody; it creates no membership authorization.
    pub fn from_lease(lease: CallLease, scope: Scope) -> Self {
        Self {
            lease,
            scope,
            _custody: None,
        }
    }
    pub fn lease(&self) -> CallLease {
        self.lease.clone()
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn with_custody(
        mut self,
        room: catcoms_rt::SharedRequestKeepalive,
        credit: resources::Credit,
    ) -> Self {
        self._custody = Some((room, credit));
        self
    }
}
impl Drop for CallGrant {
    fn drop(&mut self) {
        self.lease.revoke();
    }
}
redacted_debug!(CallGrant);

/// A watcher is a current server member observing one room; it is never a participant/capture
/// capability. Its one-time state claim and revocation are independent of a native call lease.
#[derive(Clone, Default)]
pub struct WatchLease {
    lease: CallLease,
    parent: Option<CallLease>,
}
impl WatchLease {
    pub(crate) fn new(lease: CallLease) -> Self {
        Self {
            lease,
            parent: None,
        }
    }
    /// Bind observation to native consent without turning it into participation authority.
    /// Keep both owners in the typed grant: extracting just the watcher child would let queued
    /// work survive End/HMR until the actor eventually handled its stop command.
    /// The native factory must bind before lending any clones or constructing a Watcher. This
    /// trusted Rust adapter does not retrofit a parent into wrappers cloned before binding.
    pub fn with_parent(mut self, parent: CallLease) -> Result<Self, VoiceError> {
        if self.parent.is_some()
            || self
                .lease
                .0
                .ledger_claimed
                .load(std::sync::atomic::Ordering::Acquire)
            || !self.is_live()
            || !parent.is_live()
        {
            return Err(VoiceError::Unauthorized);
        }
        self.parent = Some(parent);
        Ok(self)
    }
    pub fn is_live(&self) -> bool {
        self.lease.is_live() && self.parent.as_ref().is_none_or(CallLease::is_live)
    }
    pub fn revoke(&self) {
        self.lease.revoke();
    }
    pub fn is_same(&self, other: &Self) -> bool {
        self.lease.is_same(&other.lease)
            && match (&self.parent, &other.parent) {
                (None, None) => true,
                (Some(first), Some(second)) => first.is_same(second),
                _ => false,
            }
    }
    pub(crate) async fn wait_revoked(&self) {
        if let Some(parent) = &self.parent {
            futures::future::select(
                Box::pin(self.lease.wait_revoked()),
                Box::pin(parent.wait_revoked()),
            )
            .await;
        } else {
            self.lease.wait_revoked().await;
        }
    }
    fn claim(&self) -> Result<(), VoiceError> {
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        self.lease.claim_packet_ledger()?;
        if !self.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
}
redacted_debug!(WatchLease);
const OBJECT_DOMAIN: &str = "mewtual/voice/object/v2";
const PERMIT_DOMAIN: &str = "mewtual/voice/route-permit/v2";
const PAIR_DOMAIN: &str = "mewtual/voice/pair/v2";
const VERSION: u8 = 2;
const SUITE: u8 = 1; // P-256 ECDH / HKDF-SHA256 / AES-256-GCM, 96-bit IV, 128-bit tag.

/// No payload/address/key material is interpolated into diagnostics by these errors.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VoiceError {
    #[error("malformed voice-v2 object")]
    Malformed,
    #[error("voice-v2 object exceeds its bound")]
    Bounds,
    #[error("invalid voice-v2 signature")]
    Signature,
    #[error("voice-v2 scope mismatch")]
    Scope,
    #[error("voice-v2 object expired or not yet valid")]
    Freshness,
    #[error("voice-v2 route is not authorized")]
    Unauthorized,
    #[error("voice-v2 signing failed")]
    Signing,
    #[error("voice-v2 capacity reached")]
    Capacity,
    #[error("voice-v2 packet was already received or is outside the replay window")]
    Replay,
    #[error("voice-v2 reservation is no longer usable")]
    Reservation,
    #[error("voice-v2 retry differs from the original packet")]
    RetryMismatch,
    #[error("voice-v2 sender is rate limited")]
    RateLimited,
}

/// Internal typed-object signer, shared by deterministic identity tests and the actor's MLS
/// leaf identity. Neither the signing key nor an unrestricted signing command crosses IPC.
pub trait VoiceSigner {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError>;
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError>;
}
impl VoiceSigner for DeviceKeypair {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
        Ok(self.verifying_key().to_bytes())
    }
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
        Ok(self.sign(bytes))
    }
}
impl VoiceSigner for catcoms_mls::MlsDevice {
    fn voice_public_key(&self) -> Result<[u8; 32], VoiceError> {
        self.public_key_bytes()
            .try_into()
            .map_err(|_| VoiceError::Signing)
    }
    fn sign_voice(&self, bytes: &[u8]) -> Result<[u8; 64], VoiceError> {
        self.sign(bytes).map_err(|_| VoiceError::Signing)
    }
}
impl From<WireError> for VoiceError {
    fn from(_: WireError) -> Self {
        Self::Malformed
    }
}

/// Stable MLS-group hash, immutable authenticated policy pin and channel id. An absent pin is
/// explicit direct-only legacy eligibility; it can never authorize member forwarding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    pub group: [u8; 32],
    pub policy: Option<[u8; 32]>,
    pub room: u128,
}
impl Scope {
    pub fn for_group(group: &[u8], policy: Option<[u8; 32]>, room: u128) -> Self {
        let mut hash = blake3::Hasher::new();
        hash.update(b"mewtual/voice/group/v2\0");
        hash.update(group);
        Self {
            group: *hash.finalize().as_bytes(),
            policy,
            room,
        }
    }
    fn put(&self, e: &mut Encoder) {
        fixed(e, &self.group);
        e.put_u8(u8::from(self.policy.is_some()));
        if let Some(policy) = self.policy {
            fixed(e, &policy);
        }
        e.put_u128(self.room);
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let group = array(d)?;
        let policy = match d.get_u8()? {
            0 => None,
            1 => Some(array(d)?),
            _ => return Err(VoiceError::Malformed),
        };
        Ok(Self {
            group,
            policy,
            room: d.get_u128()?,
        })
    }
}

/// Native-generated opaque identities, distinct from a stable room or device id.
pub type Nonce = [u8; 32];
pub type MessageId = [u8; 32];

/// Public endpoint key binding. Renewals retain every field here, including the native join.
/// The browser must also validate the point with raw WebCrypto import before activating it.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinKey {
    pub join: Nonce,
    pub public_key: [u8; 65],
    pub key_ref: [u8; 32],
}
impl JoinKey {
    pub fn new(join: Nonce, public_key: [u8; 65]) -> Result<Self, VoiceError> {
        let key_ref = key_reference(&public_key);
        let value = Self {
            join,
            public_key,
            key_ref,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), VoiceError> {
        if self.public_key[0] != 4 || self.key_ref != key_reference(&self.public_key) {
            return Err(VoiceError::Malformed);
        }
        Ok(())
    }
    fn put(&self, e: &mut Encoder) {
        fixed(e, &self.join);
        e.put_u8(SUITE);
        fixed(e, &self.public_key);
        fixed(e, &self.key_ref);
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let join = array(d)?;
        if d.get_u8()? != SUITE {
            return Err(VoiceError::Malformed);
        }
        let value = Self {
            join,
            public_key: array(d)?,
            key_ref: array(d)?,
        };
        value.validate()?;
        Ok(value)
    }
}
pub fn key_reference(point: &[u8; 65]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/voice/key-ref/v2\0");
    hash.update(point);
    *hash.finalize().as_bytes()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OfferId {
    pub issuer: DeviceId,
    pub message: MessageId,
}
impl OfferId {
    fn put(&self, e: &mut Encoder) {
        fixed(e, self.issuer.as_bytes());
        fixed(e, &self.message);
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        Ok(Self {
            issuer: DeviceId::from_bytes(array(d)?),
            message: array(d)?,
        })
    }
}

/// Role-specific confirmation kinds cannot be confused with presence proofs or RTC traffic.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EncryptedKind {
    ConfirmHello = 1,
    ConfirmProof = 2,
    ConfirmFinish = 3,
    Offer = 4,
    Answer = 5,
    Ice = 6,
    Bye = 7,
}
impl EncryptedKind {
    fn get(value: u8) -> Result<Self, VoiceError> {
        match value {
            1 => Ok(Self::ConfirmHello),
            2 => Ok(Self::ConfirmProof),
            3 => Ok(Self::ConfirmFinish),
            4 => Ok(Self::Offer),
            5 => Ok(Self::Answer),
            6 => Ok(Self::Ice),
            7 => Ok(Self::Bye),
            _ => Err(VoiceError::Malformed),
        }
    }
    fn max_plaintext(self) -> usize {
        match self {
            Self::Offer | Self::Answer => MAX_SDP_PLAINTEXT,
            Self::Ice => 2 * 1024,
            Self::Bye => 128,
            _ => 512,
        }
    }
}

/// AAD is produced by native from this header and the common signed fields, never rebuilt in JS.
#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedHeader {
    pub source_join: Nonce,
    pub target: DeviceId,
    pub target_join: Nonce,
    pub source_key_ref: [u8; 32],
    pub target_key_ref: [u8; 32],
    pub handshake: [u8; 32],
    pub low_to_high: bool,
    pub kind: EncryptedKind,
    pub sequence: u64,
    pub offer: Option<OfferId>,
}
impl EncryptedHeader {
    fn put(&self, e: &mut Encoder) {
        fixed(e, &self.source_join);
        fixed(e, self.target.as_bytes());
        fixed(e, &self.target_join);
        fixed(e, &self.source_key_ref);
        fixed(e, &self.target_key_ref);
        e.put_u8(SUITE);
        fixed(e, &self.handshake);
        e.put_u8(u8::from(self.low_to_high));
        e.put_u8(self.kind as u8);
        e.put_u64(self.sequence);
        e.put_u8(u8::from(self.offer.is_some()));
        if let Some(offer) = self.offer {
            offer.put(e);
        }
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        let source_join = array(d)?;
        let target = DeviceId::from_bytes(array(d)?);
        let target_join = array(d)?;
        let source_key_ref = array(d)?;
        let target_key_ref = array(d)?;
        if d.get_u8()? != SUITE {
            return Err(VoiceError::Malformed);
        }
        let handshake = array(d)?;
        let low_to_high = boolean(d)?;
        let kind = EncryptedKind::get(d.get_u8()?)?;
        let sequence = d.get_u64()?;
        let offer = match d.get_u8()? {
            0 => None,
            1 => Some(OfferId::get(d)?),
            _ => return Err(VoiceError::Malformed),
        };
        Ok(Self {
            source_join,
            target,
            target_join,
            source_key_ref,
            target_key_ref,
            handshake,
            low_to_high,
            kind,
            sequence,
            offer,
        })
    }
    fn validate(&self, issuer: DeviceId, id: MessageId) -> Result<(), VoiceError> {
        if issuer == self.target || self.sequence == 0 || self.low_to_high != (issuer < self.target)
        {
            return Err(VoiceError::Malformed);
        }
        match (self.kind, self.offer) {
            (EncryptedKind::Offer, Some(offer))
                if offer.issuer == issuer && offer.message == id =>
            {
                Ok(())
            }
            (EncryptedKind::Answer, Some(offer)) if offer.issuer == self.target => Ok(()),
            (EncryptedKind::Ice, Some(offer))
                if offer.issuer == issuer || offer.issuer == self.target =>
            {
                Ok(())
            }
            (EncryptedKind::ConfirmHello | EncryptedKind::ConfirmFinish, None)
                if self.low_to_high =>
            {
                Ok(())
            }
            (EncryptedKind::ConfirmProof, None) if !self.low_to_high => Ok(()),
            (EncryptedKind::Bye, None) => Ok(()),
            _ => Err(VoiceError::Malformed),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum Body {
    Announcement {
        key: JoinKey,
        epoch: u64,
        renewal: u64,
    },
    WatchChallenge {
        watcher: Nonce,
        target: DeviceId,
        join: Nonce,
        key_ref: [u8; 32],
        challenge: Nonce,
    },
    JoinProof {
        join: Nonce,
        key_ref: [u8; 32],
        watcher: DeviceId,
        watcher_incarnation: Nonce,
        challenge: Nonce,
        challenge_id: MessageId,
    },
    Encrypted {
        header: EncryptedHeader,
        ciphertext: Vec<u8>,
    },
}
impl Body {
    fn kind(&self) -> u8 {
        match self {
            Self::Announcement { .. } => 1,
            Self::WatchChallenge { .. } => 2,
            Self::JoinProof { .. } => 3,
            Self::Encrypted { .. } => 4,
        }
    }
    pub fn target(&self) -> Option<DeviceId> {
        match self {
            Self::Announcement { .. } => None,
            Self::WatchChallenge { target, .. } => Some(*target),
            Self::JoinProof { watcher, .. } => Some(*watcher),
            Self::Encrypted { header, .. } => Some(header.target),
        }
    }
    fn put_header(&self, e: &mut Encoder) {
        match self {
            Self::Announcement {
                key,
                epoch,
                renewal,
            } => {
                key.put(e);
                e.put_u64(*epoch);
                e.put_u64(*renewal);
            }
            Self::WatchChallenge {
                watcher,
                target,
                join,
                key_ref,
                challenge,
            } => {
                fixed(e, watcher);
                fixed(e, target.as_bytes());
                fixed(e, join);
                fixed(e, key_ref);
                fixed(e, challenge);
            }
            Self::JoinProof {
                join,
                key_ref,
                watcher,
                watcher_incarnation,
                challenge,
                challenge_id,
            } => {
                fixed(e, join);
                fixed(e, key_ref);
                fixed(e, watcher.as_bytes());
                fixed(e, watcher_incarnation);
                fixed(e, challenge);
                fixed(e, challenge_id);
            }
            Self::Encrypted { header, .. } => header.put(e),
        }
    }
    fn get(kind: u8, d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        Ok(match kind {
            1 => Self::Announcement {
                key: JoinKey::get(d)?,
                epoch: d.get_u64()?,
                renewal: d.get_u64()?,
            },
            2 => Self::WatchChallenge {
                watcher: array(d)?,
                target: DeviceId::from_bytes(array(d)?),
                join: array(d)?,
                key_ref: array(d)?,
                challenge: array(d)?,
            },
            3 => Self::JoinProof {
                join: array(d)?,
                key_ref: array(d)?,
                watcher: DeviceId::from_bytes(array(d)?),
                watcher_incarnation: array(d)?,
                challenge: array(d)?,
                challenge_id: array(d)?,
            },
            4 => {
                let header = EncryptedHeader::get(d)?;
                let bytes = d.get_bytes()?;
                if bytes.len() < 16 || bytes.len() > header.kind.max_plaintext() + 16 {
                    return Err(VoiceError::Bounds);
                }
                Self::Encrypted {
                    header,
                    ciphertext: bytes.to_vec(),
                }
            }
            _ => return Err(VoiceError::Malformed),
        })
    }
}

/// Verified signature, with private fields to prevent mutation after verification. All getters
/// return immutable views; use current native authority rather than treating this type as a grant.
#[derive(Clone, PartialEq, Eq)]
pub struct SignedObject {
    scope: Scope,
    issuer: DeviceId,
    public_key: [u8; 32],
    id: MessageId,
    issued: u64,
    expires: u64,
    body: Body,
    signature: [u8; 64],
}
impl SignedObject {
    /// Typed internal signing seam. Exposing this as an unrestricted UI command is forbidden.
    pub fn issue(
        scope: Scope,
        signer: &(impl VoiceSigner + ?Sized),
        id: MessageId,
        issued: u64,
        expires: u64,
        body: Body,
    ) -> Result<Self, VoiceError> {
        let public_key = signer.voice_public_key()?;
        let mut value = Self {
            scope,
            issuer: DeviceId::from_public_key_bytes(&public_key),
            public_key,
            id,
            issued,
            expires,
            body,
            signature: [0; 64],
        };
        value.validate()?;
        let unsigned = value.unsigned();
        value.signature = signer.sign_voice(&unsigned)?;
        if !verify_with_public_bytes(&public_key, &unsigned, &value.signature) {
            return Err(VoiceError::Signature);
        }
        Ok(value)
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub fn issuer(&self) -> DeviceId {
        self.issuer
    }
    pub fn id(&self) -> MessageId {
        self.id
    }
    pub fn issued(&self) -> u64 {
        self.issued
    }
    pub fn expires(&self) -> u64 {
        self.expires
    }
    pub fn body(&self) -> &Body {
        &self.body
    }
    pub fn public_key(&self) -> &[u8; 32] {
        &self.public_key
    }

    fn validate(&self) -> Result<(), VoiceError> {
        if self.issuer != DeviceId::from_public_key_bytes(&self.public_key) {
            return Err(VoiceError::Signature);
        }
        let lifetime = if matches!(self.body, Body::Announcement { .. }) {
            MAX_DELEGATION_LIFETIME_MS
        } else {
            MAX_PACKET_LIFETIME_MS
        };
        lifetime_valid(self.issued, self.expires, lifetime)?;
        if self.body.target() == Some(self.issuer) {
            return Err(VoiceError::Malformed);
        }
        match &self.body {
            Body::Announcement { key, renewal, .. } => {
                key.validate()?;
                if *renewal == 0 {
                    return Err(VoiceError::Malformed);
                }
            }
            Body::Encrypted { header, ciphertext } => {
                header.validate(self.issuer, self.id)?;
                if ciphertext.len() < 16 || ciphertext.len() > header.kind.max_plaintext() + 16 {
                    return Err(VoiceError::Bounds);
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn header(&self) -> Vec<u8> {
        object_header(
            self.scope,
            self.issuer,
            &self.public_key,
            &self.id,
            self.issued,
            self.expires,
            &self.body,
        )
    }

    /// Exact canonical AAD for an encrypted packet, including every immutable routing/session field.
    pub fn aad(&self) -> Result<Vec<u8>, VoiceError> {
        if !matches!(self.body, Body::Encrypted { .. }) {
            return Err(VoiceError::Malformed);
        }
        Ok(self.header())
    }
    fn unsigned(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        // The header is already injective. Append the length-prefixed ciphertext only for the
        // encrypted closed variant; no other body may smuggle additional bytes.
        let header = self.header();
        if let Body::Encrypted { ciphertext, .. } = &self.body {
            let mut bytes = header;
            e.put_bytes(ciphertext).expect("bounded ciphertext");
            bytes.extend(e.finish());
            bytes
        } else {
            header
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        fixed(&mut e, &self.unsigned());
        fixed(&mut e, &self.signature);
        e.finish()
    }
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() > MAX_ENVELOPE {
            return Err(VoiceError::Bounds);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes()?;
        let signature = array(&mut outer)?;
        outer.finish()?;
        let mut d = Decoder::new(unsigned);
        if d.get_str()? != OBJECT_DOMAIN || d.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let kind = d.get_u8()?;
        let scope = Scope::get(&mut d)?;
        let issuer = DeviceId::from_bytes(array(&mut d)?);
        let public_key = array(&mut d)?;
        let id = array(&mut d)?;
        let issued = d.get_u64()?;
        let expires = d.get_u64()?;
        let body = Body::get(kind, &mut d)?;
        d.finish()?;
        let value = Self {
            scope,
            issuer,
            public_key,
            id,
            issued,
            expires,
            body,
            signature,
        };
        value.validate()?;
        if !verify_with_public_bytes(&public_key, unsigned, &signature) {
            return Err(VoiceError::Signature);
        }
        Ok(value)
    }
    pub fn check_freshness(&self, now: u64) -> Result<(), VoiceError> {
        freshness(self.issued, self.expires, now)
    }
}

/// Native reservation's unsigned immutable encrypted header. Constructing a draft never signs
/// placeholder ciphertext; sealing must use the same native leaf identity and exact header.
#[derive(Clone)]
pub struct PacketDraft {
    scope: Scope,
    public_key: [u8; 32],
    id: MessageId,
    issued: u64,
    expires: u64,
    header: EncryptedHeader,
}
redacted_debug!(PacketDraft);
impl PacketDraft {
    pub fn new(
        scope: Scope,
        public_key: [u8; 32],
        id: MessageId,
        issued: u64,
        expires: u64,
        header: EncryptedHeader,
    ) -> Result<Self, VoiceError> {
        lifetime_valid(issued, expires, MAX_PACKET_LIFETIME_MS)?;
        header.validate(DeviceId::from_public_key_bytes(&public_key), id)?;
        Ok(Self {
            scope,
            public_key,
            id,
            issued,
            expires,
            header,
        })
    }
    pub fn aad(&self) -> Vec<u8> {
        object_header(
            self.scope,
            DeviceId::from_public_key_bytes(&self.public_key),
            &self.public_key,
            &self.id,
            self.issued,
            self.expires,
            &Body::Encrypted {
                header: self.header.clone(),
                ciphertext: Vec::new(),
            },
        )
    }
    pub fn seal(
        &self,
        signer: &(impl VoiceSigner + ?Sized),
        ciphertext: &[u8],
    ) -> Result<SignedObject, VoiceError> {
        if signer.voice_public_key()? != self.public_key {
            return Err(VoiceError::Unauthorized);
        }
        if ciphertext.len() < 16 || ciphertext.len() > self.header.kind.max_plaintext() + 16 {
            return Err(VoiceError::Bounds);
        }
        SignedObject::issue(
            self.scope,
            signer,
            self.id,
            self.issued,
            self.expires,
            Body::Encrypted {
                header: self.header.clone(),
                ciphertext: ciphertext.to_vec(),
            },
        )
    }
}

// Both the unsigned draft and signed object use this one encoder. A header reconstructed by a
// separate JS encoder (or signed around a different ciphertext) is never the AAD authority.
fn object_header(
    scope: Scope,
    issuer: DeviceId,
    public_key: &[u8; 32],
    id: &MessageId,
    issued: u64,
    expires: u64,
    body: &Body,
) -> Vec<u8> {
    let mut e = Encoder::new();
    e.put_str(OBJECT_DOMAIN).expect("constant");
    e.put_u8(VERSION);
    e.put_u8(body.kind());
    scope.put(&mut e);
    fixed(&mut e, issuer.as_bytes());
    fixed(&mut e, public_key);
    fixed(&mut e, id);
    e.put_u64(issued);
    e.put_u64(expires);
    body.put_header(&mut e);
    e.finish()
}

/// Stable key-derivation context. Renewal time/epoch/sequence/signature are intentionally absent.
/// Both signed announcements must first pass native scope, membership and fresh-join admission.
pub fn pair_transcript(a: &SignedObject, b: &SignedObject) -> Result<Vec<u8>, VoiceError> {
    if a.scope != b.scope || a.issuer == b.issuer {
        return Err(VoiceError::Scope);
    }
    let (low, high) = if a.issuer < b.issuer { (a, b) } else { (b, a) };
    let mut e = Encoder::new();
    e.put_str(PAIR_DOMAIN).expect("constant");
    e.put_u8(VERSION);
    e.put_u8(SUITE);
    a.scope.put(&mut e);
    for object in [low, high] {
        let Body::Announcement { key, .. } = &object.body else {
            return Err(VoiceError::Malformed);
        };
        fixed(&mut e, object.issuer.as_bytes());
        fixed(&mut e, &object.public_key);
        key.put(&mut e);
    }
    Ok(e.finish())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FinalScope {
    Endpoint(DeviceId),
    Room,
}
impl FinalScope {
    fn put(self, e: &mut Encoder) {
        match self {
            Self::Endpoint(device) => {
                e.put_u8(1);
                fixed(e, device.as_bytes());
            }
            Self::Room => {
                e.put_u8(2);
            }
        }
    }
    fn get(d: &mut Decoder<'_>) -> Result<Self, VoiceError> {
        match d.get_u8()? {
            1 => Ok(Self::Endpoint(DeviceId::from_bytes(array(d)?))),
            2 => Ok(Self::Room),
            _ => Err(VoiceError::Malformed),
        }
    }
}

/// A route changes independently of endpoint encryption. A helper cannot mint this issuer's
/// permit for another helper, and captured permits cannot authorize a different immutable object.
#[derive(Clone, PartialEq, Eq)]
pub struct RoutePermit {
    scope: Scope,
    issuer: DeviceId,
    public_key: [u8; 32],
    packet: [u8; 32],
    helper: DeviceId,
    final_scope: FinalScope,
    issued: u64,
    expires: u64,
    signature: [u8; 64],
}
impl RoutePermit {
    pub fn issue(
        signer: &(impl VoiceSigner + ?Sized),
        object: &SignedObject,
        helper: DeviceId,
        issued: u64,
        expires: u64,
    ) -> Result<Self, VoiceError> {
        let final_scope = object
            .body
            .target()
            .map_or(FinalScope::Room, FinalScope::Endpoint);
        let public_key = signer.voice_public_key()?;
        let mut value = Self {
            scope: object.scope,
            issuer: DeviceId::from_public_key_bytes(&public_key),
            public_key,
            packet: object.hash(),
            helper,
            final_scope,
            issued,
            expires,
            signature: [0; 64],
        };
        value.matches(object)?;
        let unsigned = value.unsigned();
        value.signature = signer.sign_voice(&unsigned)?;
        if !verify_with_public_bytes(&public_key, &unsigned, &value.signature) {
            return Err(VoiceError::Signature);
        }
        Ok(value)
    }
    pub fn helper(&self) -> DeviceId {
        self.helper
    }
    pub fn final_scope(&self) -> FinalScope {
        self.final_scope
    }
    fn validate(&self) -> Result<(), VoiceError> {
        lifetime_valid(self.issued, self.expires, MAX_PACKET_LIFETIME_MS)?;
        if self.scope.policy.is_none()
            || self.issuer != DeviceId::from_public_key_bytes(&self.public_key)
            || self.helper == self.issuer
            || self.final_scope == FinalScope::Endpoint(self.helper)
            || self.final_scope == FinalScope::Endpoint(self.issuer)
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(())
    }
    fn matches(&self, object: &SignedObject) -> Result<(), VoiceError> {
        self.validate()?;
        let expected = object
            .body
            .target()
            .map_or(FinalScope::Room, FinalScope::Endpoint);
        if self.scope != object.scope
            || self.issuer != object.issuer
            || self.public_key != object.public_key
            || self.packet != object.hash()
            || self.final_scope != expected
            || self.expires > object.expires
            || self.issued < object.issued
        {
            return Err(VoiceError::Scope);
        }
        Ok(())
    }
    fn unsigned(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_str(PERMIT_DOMAIN).expect("constant");
        e.put_u8(VERSION);
        self.scope.put(&mut e);
        fixed(&mut e, self.issuer.as_bytes());
        fixed(&mut e, &self.public_key);
        fixed(&mut e, &self.packet);
        fixed(&mut e, self.helper.as_bytes());
        self.final_scope.put(&mut e);
        e.put_u64(self.issued);
        e.put_u64(self.expires);
        e.finish()
    }
    fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        fixed(&mut e, &self.unsigned());
        fixed(&mut e, &self.signature);
        e.finish()
    }
    fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() > 1024 {
            return Err(VoiceError::Bounds);
        }
        let mut outer = Decoder::new(bytes);
        let unsigned = outer.get_bytes()?;
        let signature = array(&mut outer)?;
        outer.finish()?;
        let mut d = Decoder::new(unsigned);
        if d.get_str()? != PERMIT_DOMAIN || d.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let scope = Scope::get(&mut d)?;
        let issuer = DeviceId::from_bytes(array(&mut d)?);
        let public_key = array(&mut d)?;
        let packet = array(&mut d)?;
        let helper = DeviceId::from_bytes(array(&mut d)?);
        let final_scope = FinalScope::get(&mut d)?;
        let issued = d.get_u64()?;
        let expires = d.get_u64()?;
        d.finish()?;
        let value = Self {
            scope,
            issuer,
            public_key,
            packet,
            helper,
            final_scope,
            issued,
            expires,
            signature,
        };
        value.validate()?;
        if !verify_with_public_bytes(&public_key, unsigned, &signature) {
            return Err(VoiceError::Signature);
        }
        Ok(value)
    }
}

/// Exactly one outer transport role. There is no recursive forwarding variant.
#[derive(Clone)]
pub enum Envelope {
    Direct(SignedObject),
    ToHelper {
        object: SignedObject,
        permit: RoutePermit,
    },
    HelperDelivery {
        object: SignedObject,
        permit: RoutePermit,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Deliver,
    Forward(FinalScope),
}
// This value selects only an existing prepaid transient loan. It grants no verified scope,
// signature, pair or replay authority; every length-delimited field is borrowed without Vecs.
struct UnverifiedEncrypted {
    envelope_kind: u8,
    scope: Scope,
    issuer: DeviceId,
    header: EncryptedHeader,
}

/// Snapshot only for synchronous validation. Deferred effects must acquire fresh native/actor
/// authority at commit. `indirect` comes from policy_allows_member_mesh(), never a UI mode.
pub struct Context {
    pub scope: Scope,
    pub local: DeviceId,
    pub now: u64,
    pub epoch: u64,
    pub indirect: bool,
}

impl<T: catcoms_rt::MeshTransport, R: catcoms_rt::CryptoRngCore> crate::ChannelSync<T, R> {
    /// Current native eligibility snapshot, never a durable permission. Product callers also
    /// require a known channel; deferred protocol operations repeat this and exact lease checks.
    /// Inactive/persistence-pending pins refuse rather than masquerading as absent legacy policy.
    pub fn voice_v2_context(&self, room: u128) -> Result<Context, VoiceError> {
        if !self.group.is_active()
            || !self.group.contains_device(&self.device.device_id())
            || self
                .group_policy
                .as_ref()
                .is_some_and(|policy| policy.mode() != crate::GroupMode::PeerToPeer)
            || (self.group_policy.is_some() && !self.group_policy_active)
        {
            return Err(VoiceError::Unauthorized);
        }
        let policy = match self.group_mode() {
            crate::GroupMode::PeerToPeer => {
                Some(self.group_policy_digest().ok_or(VoiceError::Unauthorized)?)
            }
            crate::GroupMode::LegacyUnverified => None,
            _ => return Err(VoiceError::Unauthorized),
        };
        Ok(Context {
            scope: Scope::for_group(&self.group.group_id(), policy, room),
            local: self.device.device_id(),
            now: self.voice_v2_clock().now_ms(),
            epoch: self.group.epoch(),
            indirect: self.policy_allows_member_mesh(),
        })
    }
}
impl Envelope {
    fn peek_encrypted(bytes: &[u8]) -> Result<UnverifiedEncrypted, VoiceError> {
        if bytes.len() > MAX_ENVELOPE {
            return Err(VoiceError::Bounds);
        }
        let mut outer = Decoder::new(bytes);
        if outer.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let envelope_kind = outer.get_u8()?;
        if !matches!(envelope_kind, 1..=3) {
            return Err(VoiceError::Malformed);
        }
        let object = outer.get_bytes()?;
        if envelope_kind != 1 && outer.get_bytes()?.len() > 1024 {
            return Err(VoiceError::Bounds);
        }
        outer.finish()?;
        let mut signed = Decoder::new(object);
        let unsigned = signed.get_bytes()?;
        let _: [u8; 64] = array(&mut signed)?;
        signed.finish()?;
        let mut d = Decoder::new(unsigned);
        if d.get_str()? != OBJECT_DOMAIN || d.get_u8()? != VERSION || d.get_u8()? != 4 {
            return Err(VoiceError::Malformed);
        }
        let scope = Scope::get(&mut d)?;
        let issuer = DeviceId::from_bytes(array(&mut d)?);
        let _: [u8; 32] = array(&mut d)?;
        let _: MessageId = array(&mut d)?;
        d.get_u64()?;
        d.get_u64()?;
        let header = EncryptedHeader::get(&mut d)?;
        let ciphertext = d.get_bytes()?;
        if ciphertext.len() < 16 || ciphertext.len() > header.kind.max_plaintext() + 16 {
            return Err(VoiceError::Bounds);
        }
        d.finish()?;
        Ok(UnverifiedEncrypted {
            envelope_kind,
            scope,
            issuer,
            header,
        })
    }
    pub fn object(&self) -> &SignedObject {
        match self {
            Self::Direct(object)
            | Self::ToHelper { object, .. }
            | Self::HelperDelivery { object, .. } => object,
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.put_u8(VERSION);
        let (kind, object, permit) = match self {
            Self::Direct(object) => (1, object, None),
            Self::ToHelper { object, permit } => (2, object, Some(permit)),
            Self::HelperDelivery { object, permit } => (3, object, Some(permit)),
        };
        e.put_u8(kind);
        fixed(&mut e, &object.encode());
        if let Some(permit) = permit {
            fixed(&mut e, &permit.encode());
        }
        e.finish()
    }
    /// Charge the authenticated immediate requester's cheap work budget BEFORE calling decode.
    /// Decoding verifies original signatures but does not admit replay or authorize delivery.
    pub fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() > MAX_ENVELOPE {
            return Err(VoiceError::Bounds);
        }
        let mut d = Decoder::new(bytes);
        if d.get_u8()? != VERSION {
            return Err(VoiceError::Malformed);
        }
        let kind = d.get_u8()?;
        if !(1..=3).contains(&kind) {
            return Err(VoiceError::Malformed);
        }
        let object = SignedObject::decode(d.get_bytes()?)?;
        let result = match kind {
            1 => Self::Direct(object),
            2 => Self::ToHelper {
                object,
                permit: RoutePermit::decode(d.get_bytes()?)?,
            },
            3 => Self::HelperDelivery {
                object,
                permit: RoutePermit::decode(d.get_bytes()?)?,
            },
            _ => unreachable!(),
        };
        d.finish()?;
        Ok(result)
    }
    /// Requester must come from the NEW peer-bound authenticated request transcript. Do not use
    /// the released legacy call kind, a display fingerprint or an unauthenticated route claim.
    pub fn authorize(
        &self,
        requester: DeviceId,
        context: &Context,
        member: impl Fn(DeviceId) -> bool,
    ) -> Result<Disposition, VoiceError> {
        let object = self.object();
        if object.scope != context.scope {
            return Err(VoiceError::Scope);
        }
        object.check_freshness(context.now)?;
        if !member(requester)
            || !member(object.issuer)
            || !member(context.local)
            || object.issuer == context.local
            || object.body.target().is_some_and(|target| !member(target))
        {
            return Err(VoiceError::Unauthorized);
        }
        if matches!(object.body, Body::Announcement { epoch, .. } if epoch > context.epoch) {
            return Err(VoiceError::Freshness);
        }
        match self {
            Self::Direct(_) => {
                if requester != object.issuer
                    || object
                        .body
                        .target()
                        .is_some_and(|target| target != context.local)
                {
                    return Err(VoiceError::Unauthorized);
                }
                Ok(Disposition::Deliver)
            }
            Self::ToHelper { permit, .. } | Self::HelperDelivery { permit, .. } => {
                if !context.indirect || context.scope.policy.is_none() || !member(permit.helper) {
                    return Err(VoiceError::Unauthorized);
                }
                permit.matches(object)?;
                freshness(permit.issued, permit.expires, context.now)?;
                match self {
                    Self::ToHelper { .. }
                        if requester == object.issuer && permit.helper == context.local =>
                    {
                        Ok(Disposition::Forward(permit.final_scope))
                    }
                    Self::HelperDelivery { .. }
                        if requester == permit.helper
                            && permit.helper != context.local
                            && object
                                .body
                                .target()
                                .is_none_or(|target| target == context.local) =>
                    {
                        Ok(Disposition::Deliver)
                    }
                    _ => Err(VoiceError::Unauthorized),
                }
            }
        }
    }
}

fn fixed(e: &mut Encoder, bytes: &[u8]) {
    e.put_bytes(bytes).expect("bounded voice field");
}
fn array<const N: usize>(d: &mut Decoder<'_>) -> Result<[u8; N], VoiceError> {
    d.get_bytes()?.try_into().map_err(|_| VoiceError::Malformed)
}
fn boolean(d: &mut Decoder<'_>) -> Result<bool, VoiceError> {
    match d.get_u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(VoiceError::Malformed),
    }
}
fn lifetime_valid(issued: u64, expires: u64, max: u64) -> Result<(), VoiceError> {
    if expires <= issued || expires - issued > max {
        Err(VoiceError::Freshness)
    } else {
        Ok(())
    }
}
fn freshness(issued: u64, expires: u64, now: u64) -> Result<(), VoiceError> {
    if issued > now.saturating_add(CLOCK_SKEW_MS) || now > expires.saturating_add(CLOCK_SKEW_MS) {
        Err(VoiceError::Freshness)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
