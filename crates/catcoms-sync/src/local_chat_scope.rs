//! Managed local authoring lifetimes. Saved labels are metadata, never runtime permits.
//!
//! The vault host owns independent durable lineage and final storage admission. This module
//! only binds a validated observer to the exact MLS group/device and carries that ORIGINAL
//! observer through lower chat publication. Legacy no-tail operation remains unchanged until
//! the host explicitly stages the managed format; a scope-bearing raw restore is always unbound.
use super::*;
use catcoms_replication::doc::{SavedLocalChatOp, SavedLocalChatRecovery};
use catcoms_rt::{RequestAuthority, RequestCancellation};
use tokio::sync::watch;
mod initialization_origin;
pub(crate) use initialization_origin::OriginalInitializationObservation;

const TAG: &str = "catcoms/local-chat-scope/v1";
const MAX_ORIGINAL_BYTES: usize = 256 * 1024;
// Pending publication is already limited to 32. Bound the tail before decoding any payload.
const MAX_TAIL_BYTES: usize = 512 + durable_chat::MAX_CHAT_PENDING * (MAX_ORIGINAL_BYTES + 256);

/// Trusted cross-crate host seam. The app implements this for its opaque Current-admission
/// result, whose fields/observer can only come from the actual mounted inventory. It is not a
/// wire interface: never implement it for decoded labels, raw scalars or a fresh watch channel.
/// Sync does not authenticate arbitrary Rust implementations. A new production implementation
/// changes this authorization boundary and requires review; fake implementations are test-only.
pub trait ManagedChatScopeReservation {
    fn group(&self) -> &[u8];
    fn signer(&self) -> &[u8; 32];
    fn domain(&self) -> [u8; 32];
    fn nonce(&self) -> [u8; 32];
    /// Runtime identity only. Retaining this dummy allocation cannot keep its owner alive.
    fn owner_identity(&self) -> Arc<()>;
    fn reservation_observer(&self) -> RequestCancellation;
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation>;
}
/// Negative-only original lifetime target. Never select a newer owner by logical key.
pub trait ManagedChatScopeRevocation: Send + Sync {
    fn revoke(&self);
}
/// Only a durably Current Store result implements the stronger publication contract.
pub trait ManagedChatScopeBinding: ManagedChatScopeReservation {
    fn publication_authority(&self) -> RequestCancellation;
}

/// Saved local scope label. This is safe to inspect and cannot construct a live permit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatScopeLabel {
    domain: [u8; 32],
    nonce: [u8; 32],
    retired: bool,
    initializing: bool,
}
impl ChatScopeLabel {
    pub fn domain(&self) -> [u8; 32] {
        self.domain
    }
    pub fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    pub fn retired(&self) -> bool {
        self.retired
    }
    pub fn initializing(&self) -> bool {
        self.initializing
    }
}

/// An immutable accepted obligation keeps its old request identity. Only its publication
/// lifetime is associated with the new scope; no receipt, token or signed operation is rewritten.
#[derive(Clone)]
pub(super) struct PublicationAlias {
    pub token: [u8; 16],
    pub binding: [u8; 32],
    pub context: [u8; 32],
    pub channel: u128,
    pub change: ChangeHash,
    pub action: durable_chat::ChatAction,
    pub target: Option<[u8; 32]>,
    pub sealed_hash: [u8; 32],
    pub original: SavedLocalChatOp,
}

struct Runtime {
    owner: watch::Sender<bool>,
    original: RequestCancellation,
    publication: RequestCancellation,
}
struct Reservation {
    identity: Arc<()>,
    observer: RequestCancellation,
    revoker: Arc<dyn ManagedChatScopeRevocation>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.owner.send_replace(true);
    }
}
#[derive(Debug)]
struct PublicationAuthority {
    original: RequestCancellation,
    local: watch::Receiver<bool>,
}
#[async_trait::async_trait]
impl RequestAuthority for PublicationAuthority {
    fn authorized(&self) -> bool {
        !self.original.is_cancelled() && !*self.local.borrow() && self.local.has_changed().is_ok()
    }
    async fn revoked(&self) {
        let mut original = self.original.clone();
        let mut local = self.local.clone();
        while self.authorized() {
            tokio::select! {
                _ = original.cancelled() => return,
                changed = local.changed() => if changed.is_err() { return; },
            }
        }
    }
}

#[derive(Default)]
pub(super) struct LocalChatScope {
    required: bool,
    label: Option<ChatScopeLabel>,
    aliases: BTreeMap<[u8; 16], PublicationAlias>,
    // Derived bounded migration work. At most 32 original scans are advanced per staging call,
    // each with the replication layer's fixed operation/byte budget, outside final save leases.
    pub(super) recovery: BTreeMap<[u8; 16], SavedLocalChatRecovery>,
    runtime: Option<Runtime>,
    reservation: Option<Reservation>,
    revision: u64,
}
impl LocalChatScope {
    // Negative budget gate only. An initial private owner has an Active saved label, but no
    // publication binding, legacy aliases or retained migration work. Current issuance remains
    // the Store's separate durable barrier; this predicate cannot bind or publish the scope.
    pub(super) fn initialization_snapshot_allowed(&self) -> bool {
        self.label
            .as_ref()
            .is_some_and(|label| !label.retired && !label.initializing)
            && self.aliases.is_empty()
            && self.recovery.is_empty()
            && self.runtime.is_none()
    }

    pub(super) fn managed(&self) -> bool {
        self.required || self.label.is_some()
    }
    pub(super) fn live(&self) -> bool {
        self.label
            .as_ref()
            .is_some_and(|label| !label.retired && !label.initializing)
            && self
                .reservation
                .as_ref()
                .is_some_and(|reservation| !reservation.observer.is_cancelled())
            && self.runtime.as_ref().is_some_and(|runtime| {
                !runtime.original.is_cancelled() && !runtime.publication.is_cancelled()
            })
    }
    pub(super) fn authority(&self) -> Option<RequestCancellation> {
        self.live()
            .then(|| self.runtime.as_ref().unwrap().publication.clone())
    }
    pub(super) fn retire(&mut self) {
        if let Some(reservation) = &self.reservation {
            reservation.revoker.revoke();
        }
        if let Some(runtime) = &self.runtime {
            runtime.owner.send_replace(true);
        }
        if let Some(label) = &mut self.label {
            if !label.retired {
                label.retired = true;
                label.initializing = false;
                self.revision = self.revision.saturating_add(1);
            }
        }
    }
    pub(super) fn prune_alias(&mut self, token: &[u8; 16]) {
        if self.aliases.remove(token).is_some() {
            self.revision = self.revision.saturating_add(1);
        }
    }

    pub(super) fn encode(
        &self,
        group: &[u8],
        signer: &[u8],
        chat: &durable_chat::DurableChatState,
    ) -> Result<Option<Vec<u8>>, SyncError> {
        let Some(label) = &self.label else {
            return Ok(None);
        };
        let aliases: Vec<_> = self
            .aliases
            .values()
            .filter(|alias| chat.publication_alias_live(alias))
            .collect();
        let mut e = Encoder::new();
        let bad = || SyncError::Malformed;
        e.put_str(TAG).map_err(|_| bad())?;
        e.put_bytes(group).map_err(|_| bad())?;
        e.put_bytes(signer).map_err(|_| bad())?;
        e.put_bytes(&label.domain).map_err(|_| bad())?;
        e.put_bytes(&label.nonce).map_err(|_| bad())?;
        e.put_u8(if label.retired {
            1
        } else if label.initializing {
            2
        } else {
            0
        });
        e.put_u32(aliases.len() as u32);
        for alias in aliases {
            e.put_bytes(&alias.token).map_err(|_| bad())?;
            e.put_bytes(&alias.binding).map_err(|_| bad())?;
            e.put_bytes(&alias.context).map_err(|_| bad())?;
            e.put_u128(alias.channel);
            e.put_bytes(&alias.change.0).map_err(|_| bad())?;
            e.put_u8(match alias.action {
                durable_chat::ChatAction::Send => 0,
                durable_chat::ChatAction::Edit => 1,
                durable_chat::ChatAction::Delete => 2,
            });
            if let Some(target) = alias.target {
                e.put_bytes(&target).map_err(|_| bad())?;
            }
            e.put_bytes(&alias.sealed_hash).map_err(|_| bad())?;
            e.put_bytes(&alias.original.encode()).map_err(|_| bad())?;
        }
        let encoded = e.finish();
        if encoded.len() > MAX_TAIL_BYTES {
            return Err(bad());
        }
        Ok(Some(encoded))
    }

    /// Borrow the first-initialization subset before generic restore can allocate aliases or
    /// load MLS. Exact original group/key labels are scope metadata, never a live binding.
    pub(crate) fn preflight_owner_initialization(
        bytes: &[u8],
        group: &[u8],
        signer: &[u8],
    ) -> Result<(), SyncError> {
        let bad = || SyncError::Malformed;
        if bytes.len() > 512 {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        let mut decoder = Decoder::new(bytes);
        if decoder.get_str().map_err(|_| bad())? != TAG
            || decoder.get_bytes().map_err(|_| bad())? != group
            || decoder.get_bytes().map_err(|_| bad())? != signer
            || decoder.get_bytes().map_err(|_| bad())?.len() != 32
            || decoder.get_bytes().map_err(|_| bad())?.len() != 32
        {
            return Err(bad());
        }
        match decoder.get_u8().map_err(|_| bad())? {
            0 => {} // the candidate is privately staged Active BEFORE its canonical save
            1 | 2 => return Err(SyncError::OwnerInitializationUnsupported),
            _ => return Err(bad()),
        }
        let aliases = decoder.get_u32().map_err(|_| bad())? as usize;
        // Each alias requires at least five framed32-byte fields plus16B channel, action,
        // encoded-original framing and its remaining metadata. Use a conservative minimum;
        // this path rejects ALL nonempty aliases rather than dropping accepted originals.
        if aliases > decoder.remaining() / 201 {
            return Err(bad());
        }
        if aliases != 0 {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        decoder.finish().map_err(|_| bad())
    }

    pub(super) fn decode(
        bytes: &[u8],
        group: &ServerGroup,
        device: &MlsDevice,
        chat: &mut durable_chat::DurableChatState,
    ) -> Result<Self, SyncError> {
        let bad = || SyncError::Malformed;
        if bytes.len() > MAX_TAIL_BYTES {
            return Err(bad());
        }
        let mut d = Decoder::new(bytes);
        if d.get_str().map_err(|_| bad())? != TAG
            || d.get_bytes().map_err(|_| bad())? != group.group_id()
            || d.get_bytes().map_err(|_| bad())? != device.public_key_bytes()
        {
            return Err(bad());
        }
        let domain = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let nonce = d
            .get_bytes()
            .map_err(|_| bad())?
            .try_into()
            .map_err(|_| bad())?;
        let (retired, initializing) = match d.get_u8().map_err(|_| bad())? {
            0 => (false, false),
            1 => (true, false),
            2 => (false, true),
            _ => return Err(bad()),
        };
        let count = d.get_u32().map_err(|_| bad())? as usize;
        // Minimum fixed framing before any original allocation; never trust count as capacity.
        if count > durable_chat::MAX_CHAT_PENDING
            || count > d.remaining() / 156
            || (initializing && count != 0)
        {
            return Err(bad());
        }
        let mut aliases = BTreeMap::new();
        let mut previous = None;
        for _ in 0..count {
            let token = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            if previous.is_some_and(|previous| previous >= token) {
                return Err(bad());
            }
            previous = Some(token);
            let binding = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let context = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let channel = d.get_u128().map_err(|_| bad())?;
            let change = ChangeHash(
                d.get_bytes()
                    .map_err(|_| bad())?
                    .try_into()
                    .map_err(|_| bad())?,
            );
            let action = match d.get_u8().map_err(|_| bad())? {
                0 => durable_chat::ChatAction::Send,
                1 => durable_chat::ChatAction::Edit,
                2 => durable_chat::ChatAction::Delete,
                _ => return Err(bad()),
            };
            let target = if action == durable_chat::ChatAction::Send {
                None
            } else {
                Some(
                    d.get_bytes()
                        .map_err(|_| bad())?
                        .try_into()
                        .map_err(|_| bad())?,
                )
            };
            let sealed_hash = d
                .get_bytes()
                .map_err(|_| bad())?
                .try_into()
                .map_err(|_| bad())?;
            let original = d.get_bytes().map_err(|_| bad())?;
            if original.len() > MAX_ORIGINAL_BYTES {
                return Err(bad());
            }
            let alias = PublicationAlias {
                token,
                binding,
                context,
                channel,
                change,
                action,
                target,
                sealed_hash,
                original: SavedLocalChatOp::decode(original, device, channel, change)?,
            };
            if !chat.publication_alias_live(&alias) {
                return Err(bad());
            }
            aliases.insert(token, alias);
        }
        d.finish().map_err(|_| bad())?;
        let mut scope = Self {
            label: Some(ChatScopeLabel {
                domain,
                nonce,
                retired,
                initializing,
            }),
            required: true,
            aliases,
            runtime: None,
            reservation: None,
            revision: 0,
            recovery: BTreeMap::new(),
        };
        let context = scope.context(group, device)?;
        for alias in scope.aliases.values() {
            if alias.context == context {
                return Err(bad());
            }
            chat.hydrate_publication_alias(alias, context)?;
        }
        if !initializing && !retired {
            chat.validate_scope_publications(context)?;
        }
        // A saved Active label is never a runtime permit, even with current own membership.
        if !group.is_active() || !group.has_local_signer(&device.public_key_bytes()) {
            scope.retire();
        }
        Ok(scope)
    }

    fn context(&self, group: &ServerGroup, device: &MlsDevice) -> Result<[u8; 32], SyncError> {
        let label = self.label.as_ref().ok_or(SyncError::ChatScopeUnavailable)?;
        let mut hash = blake3::Hasher::new();
        hash.update(b"catcoms/chat-send-context/v2");
        let group = group.group_id();
        hash.update(&(group.len() as u64).to_be_bytes());
        hash.update(&group);
        let key = device.public_key_bytes();
        hash.update(&(key.len() as u64).to_be_bytes());
        hash.update(&key);
        hash.update(&label.domain);
        hash.update(&label.nonce);
        Ok(*hash.finalize().as_bytes())
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Mandatory managed-vault classification, including a captured legacy NO-TAIL snapshot.
    /// The mounted host calls this BEFORE any await, networking or owner run. This closes local
    /// positive entry points while independent lineage and saved-before-exposure initialization
    /// are resolved. It cannot authorize anything and there is no inverse escape hatch.
    pub fn require_managed_chat_scope(&mut self) {
        self.local_chat_scope.required = true;
    }

    pub fn chat_scope_label(&self) -> Option<&ChatScopeLabel> {
        self.local_chat_scope.label.as_ref()
    }
    pub fn chat_scope_revision(&self) -> u64 {
        self.local_chat_scope.revision
    }

    /// Attach original negative custody before an Initializing instance can run or reach a
    /// background writer. This cannot publish or change scope labels. This Actor cannot refresh
    /// its runtime owner identity after a physical mount replacement or later admission.
    pub fn attach_managed_chat_reservation(
        &mut self,
        source: &impl ManagedChatScopeReservation,
    ) -> Result<(), SyncError> {
        let domain = source.domain();
        let nonce = source.nonce();
        let group = source.group().to_vec();
        let signer = *source.signer();
        let identity = source.owner_identity();
        let observer = source.reservation_observer();
        let revoker = source.revocation();
        if self
            .chat_scope_label()
            .is_none_or(|label| label.domain != domain || label.nonce != nonce || label.retired)
            || group != self.group.group_id()
            || signer.as_slice() != self.device.public_key_bytes()
            || !self.group.is_active()
            || !self.group.has_local_signer(&self.device.public_key_bytes())
            || observer.is_cancelled()
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if let Some(original) = &self.local_chat_scope.reservation {
            return if Arc::ptr_eq(&original.identity, &identity)
                && !original.observer.is_cancelled()
            {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        self.local_chat_scope.reservation = Some(Reservation {
            identity,
            observer,
            revoker,
        });
        Ok(())
    }

    /// Persistable non-authorizing identity while original recovery spans owner turns. The host
    /// first reserves independent Initializing lineage, then saves this exact label before any
    /// background/negative writer is exposed. A restart resumes this nonce; it never reboots a
    /// tail-less legacy source hash or grants positive work from this phase.
    pub fn reserve_managed_chat_scope(
        &mut self,
        domain: [u8; 32],
        nonce: [u8; 32],
    ) -> Result<(), SyncError> {
        if !self.group.is_active()
            || !self.group.has_local_signer(&self.device.public_key_bytes())
            || self.group.group_id().is_empty()
            || self.group.group_id().len() > 256
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        if let Some(label) = self.chat_scope_label() {
            return if label.domain == domain && label.nonce == nonce && !label.retired {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        self.local_chat_scope.required = true;
        self.local_chat_scope.label = Some(ChatScopeLabel {
            domain,
            nonce,
            retired: false,
            initializing: true,
        });
        self.local_chat_scope.revision = self.local_chat_scope.revision.saturating_add(1);
        Ok(())
    }

    /// Trusted vault-host metadata staging, NOT issuance. The host must first durably reserve
    /// independent lineage, save this exact latest candidate, then commit Current before binding
    /// an observer or exposing context. Decoded labels alone never call this method automatically.
    /// Old accepted publication is retained exactly; unaccepted/stopped requests gain no alias.
    pub fn stage_managed_chat_scope(
        &mut self,
        domain: [u8; 32],
        nonce: [u8; 32],
    ) -> Result<(), SyncError> {
        self.reserve_managed_chat_scope(domain, nonce)?;
        if self
            .chat_scope_label()
            .is_some_and(|label| !label.initializing)
        {
            return Ok(());
        }
        if self.durable_chat_needs_repair() {
            return Err(SyncError::ChatPreparationPending);
        }
        // Compute all exact originals first. An ambiguous/missing historical envelope refuses
        // promotion without partially stopping or rewriting accepted durable obligations.
        let aliases = self.capture_legacy_publication_aliases()?;
        self.local_chat_scope.label = Some(ChatScopeLabel {
            domain,
            nonce,
            retired: false,
            initializing: false,
        });
        self.local_chat_scope.required = true;
        self.local_chat_scope.aliases = aliases;
        self.local_chat_scope.recovery.clear();
        self.local_chat_scope.revision = self.local_chat_scope.revision.saturating_add(1);
        let context = self.local_chat_scope.context(&self.group, &self.device)?;
        for alias in self.local_chat_scope.aliases.values() {
            self.durable_chat
                .hydrate_publication_alias(alias, context)?;
        }
        Ok(())
    }

    /// Attach ONLY the original mounted-store Current-admission observer. This is an explicit
    /// trusted-host seam; callers must validate independent lineage and exact scope before use.
    /// It cannot be called by a decoder. A revoked runtime cannot be renewed on the same Actor:
    /// restore a new owner under validated custody instead of reviving its old queued requests.
    pub fn bind_managed_chat_scope(
        &mut self,
        binding: &impl ManagedChatScopeBinding,
    ) -> Result<(), SyncError> {
        let domain = binding.domain();
        let nonce = binding.nonce();
        let group = binding.group().to_vec();
        let signer = *binding.signer();
        let original = binding.publication_authority();
        let Some(label) = self.chat_scope_label() else {
            return Err(SyncError::ChatScopeUnavailable);
        };
        if label.domain != domain
            || label.nonce != nonce
            || label.retired
            || label.initializing
            || original.is_cancelled()
            || group != self.group.group_id()
            || signer.as_slice() != self.device.public_key_bytes()
            || !self.group.is_active()
            || !self.group.has_local_signer(&self.device.public_key_bytes())
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        self.attach_managed_chat_reservation(binding)?;
        if self.local_chat_scope.runtime.is_some() {
            return if self.local_chat_scope.live() {
                Ok(())
            } else {
                Err(SyncError::ChatScopeUnavailable)
            };
        }
        let (owner, local) = watch::channel(false);
        let publication = RequestCancellation::new(local.clone(), None).with_authority(Arc::new(
            PublicationAuthority {
                original: original.clone(),
                local,
            },
        ));
        self.local_chat_scope.runtime = Some(Runtime {
            owner,
            original,
            publication,
        });
        Ok(())
    }

    /// Immediate process-local revocation follows actual MLS state before any later publication
    /// suspension. The vault host still owes durable external retirement before Leave/removal
    /// acknowledgment; a local label cannot fence a captured older server snapshot by itself.
    pub(super) fn observe_chat_scope_membership(&mut self) {
        if !self.group.is_active() || !self.group.has_local_signer(&self.device.public_key_bytes())
        {
            // Validate the original authenticator at attachment, not on unrelated epochs before
            // first initialization. Actual removal ends that attached admission incarnation.
            self.fresh_chat_admission = None;
            self.local_chat_scope.retire();
        }
    }

    /// Local revocation only. The vault host must separately durably retire external lineage
    /// before acknowledging Leave, and keep failed cleanup truthful. Generic shutdown does not
    /// call this method because normal restart preserves this admission's lifetime.
    pub fn retire_managed_chat_scope(&mut self) {
        self.local_chat_scope.required = true;
        self.fresh_chat_admission = None;
        self.local_chat_scope.retire();
    }

    pub(super) fn managed_chat_context(&self) -> Result<[u8; 32], SyncError> {
        if !self.local_chat_scope.live()
            || !self.group.is_active()
            || !self.group.has_local_signer(&self.device.public_key_bytes())
        {
            return Err(SyncError::ChatScopeUnavailable);
        }
        self.local_chat_scope.context(&self.group, &self.device)
    }
}
