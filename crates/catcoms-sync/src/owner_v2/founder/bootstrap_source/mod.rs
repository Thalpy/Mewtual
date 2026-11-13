//! One original founder's complete sealed source, separate from ordinary database/private-page
//! encryption. Live preparation owns the actual source; detached ciphertext and cold reads are
//! DATA only. The separately paid closed genesis seed can retain an initial SDK graph; it
//! creates no bytes-to-Fresh path, mounted key, I/O, Saved, Current or runnable scheduler.
use super::super::{snapshot, FounderSnapshotLimits};
use super::*;
use catcoms_replication::chat::owner_header::OwnerBodyId;
use catcoms_rt::CryptoRngCore;

mod codec;
mod genesis_seed;
mod s0;
pub use codec::{
    open_bootstrap_source_data, BootstrapSourceDataView, OpenedBootstrapSourceData,
    BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE, BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES,
    BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES, MAX_BOOTSTRAP_SOURCE_FRAME_BYTES,
    MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES,
};
pub use genesis_seed::{
    prepare_v2_genesis_seed, prepare_v2_genesis_seed_native_warm, DetachedV2AppendData,
    DetachedV2CreateData, OriginalV2GenesisSeedObservation, PreparedUnboundV2GenesisNode,
    PreparedV2AppendData, PreparedV2CreateData, ReturnedV2GenesisAttach, UnboundV2GenesisNode,
    UnboundV2GenesisSeed, V2AppendDataView, V2AppendPlanObservation, V2CreateDataView,
    V2CreatePacketId, V2CreateRefusal, V2CreateRequest, V2GenesisRuntimeParts,
    V2GenesisRuntimeRefusal, V2GenesisSeedData, V2GenesisSeedLimits, V2UnsignedAppendDataView,
    APPEND_DATA_BUFFER_BYTES, CREATE_DATA_BUFFER_BYTES, MAX_CREATE_ACTION_BYTES,
    MAX_CREATE_CIPHERTEXT_BYTES, MAX_CREATE_ENCRYPTED_PAYLOAD_BYTES, MAX_CREATE_ENVELOPE_BYTES,
    MAX_CREATE_PADDED_BYTES, V2_GENESIS_SEED_METADATA_BYTES,
};

const KEY_BINDING_DOMAIN: &str = "catcoms/private-chat-bootstrap-source-key-binding/v1";

/// Labels for an independently retained SourcePrepared intent, never proof of a live source.
/// All fields are DATA. `from_data` bounds the only variable field before copying it; neither
/// it nor this spec can construct a FounderBootstrapPreparation or import a decoded snapshot.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BootstrapSourceContextSpec<'a> {
    pub vault: [u8; 32],
    pub group: &'a [u8],
    pub signer: [u8; 32],
    pub fresh_attempt: [u8; 32],
    pub scope_domain: [u8; 32],
    pub scope_nonce: [u8; 32],
    pub planned_revision: u64,
    pub profile: OwnerFactId,
    pub admission: OwnerBodyId,
    pub header: OwnerHeaderId,
    pub records: PrivateChatPageId,
    pub manifest: PrivateChatPageId,
    pub limits: FounderSnapshotLimits,
    pub snapshot_digest: [u8; 32],
    pub records_plaintext_bytes: usize,
}
impl BootstrapSourceContextSpec<'_> {
    fn validate(self) -> Result<(), SyncError> {
        if self.group.is_empty()
            || self.group.len() > MAX_GROUP_ID_BYTES
            || self.scope_domain != self.vault
            || self.planned_revision == 0
            || self.records_plaintext_bytes == 0
            || self.records_plaintext_bytes > MAX_RECORD_BATCH_BYTES
        {
            return Err(unavailable());
        }
        Ok(())
    }
}
impl std::fmt::Debug for BootstrapSourceContextSpec<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapSourceContextSpec")
            .finish_non_exhaustive()
    }
}

/// Bounded immutable expected-context DATA. The offline reader's caller MUST obtain this and
/// the complete SourceId from an independently durable typed SourcePrepared intent, before the
/// first seed write. Decoding a frame and choosing its own expected context is not that boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct BootstrapSourceContextData {
    group: Vec<u8>,
    // The lifetime is genuinely static: group is always empty here. spec_data substitutes the
    // bounded owned group. No borrowed caller backing or arbitrary source object is retained.
    fixed: BootstrapSourceContextSpec<'static>,
}
impl BootstrapSourceContextData {
    pub fn from_data(spec: BootstrapSourceContextSpec<'_>) -> Result<Self, SyncError> {
        spec.validate()?;
        Ok(Self {
            group: spec.group.to_vec(),
            fixed: BootstrapSourceContextSpec {
                group: &[],
                vault: spec.vault,
                signer: spec.signer,
                fresh_attempt: spec.fresh_attempt,
                scope_domain: spec.scope_domain,
                scope_nonce: spec.scope_nonce,
                planned_revision: spec.planned_revision,
                profile: spec.profile,
                admission: spec.admission,
                header: spec.header,
                records: spec.records,
                manifest: spec.manifest,
                limits: spec.limits,
                snapshot_digest: spec.snapshot_digest,
                records_plaintext_bytes: spec.records_plaintext_bytes,
            },
        })
    }
    pub fn spec_data(&self) -> BootstrapSourceContextSpec<'_> {
        BootstrapSourceContextSpec {
            group: &self.group,
            ..self.fixed
        }
    }
}
impl std::fmt::Debug for BootstrapSourceContextData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapSourceContextData")
            .finish_non_exhaustive()
    }
}

/// Identity of COMPLETE original bootstrap ciphertext; distinct from private-page/head/file IDs.
/// `from_bytes` is only persisted-label DATA, never authenticated source or durability evidence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BootstrapSourceId([u8; 32]);
impl BootstrapSourceId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    fn of(frame: &[u8]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(b"catcoms/private-chat-bootstrap-source-id/v1\0");
        h.update(frame);
        Self(*h.finalize().as_bytes())
    }
}
impl std::fmt::Debug for BootstrapSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapSourceId").finish_non_exhaustive()
    }
}

/// Complete immutable encrypted DATA, transferable once to original Store debt BEFORE fallible
/// I/O. No creator/observer/revoker/key/Server/transport/session is retained. Offline exact repair
/// therefore does not depend on a live getter after the creator has dropped. These accessors do
/// not grant fresh write authority: the actual host must retain its own original admission.
pub struct PreparedBootstrapSource {
    frame: Zeroizing<Vec<u8>>,
    id: BootstrapSourceId,
    context: BootstrapSourceContextData,
}
impl PreparedBootstrapSource {
    pub fn as_bytes(&self) -> &[u8] {
        &self.frame
    }
    pub fn id(&self) -> BootstrapSourceId {
        self.id
    }
    pub fn context_data(&self) -> &BootstrapSourceContextData {
        &self.context
    }
}
impl std::fmt::Debug for PreparedBootstrapSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedBootstrapSource")
            .field("frame_bytes", &self.frame.len())
            .finish_non_exhaustive()
    }
}

/// Refusal before any source candidate exists returns the SAME original archive owner.
pub struct BootstrapSourceRefusal {
    original: Box<FounderArchivePreparation>,
}
impl BootstrapSourceRefusal {
    pub fn into_original(self) -> FounderArchivePreparation {
        *self.original
    }
}
impl std::fmt::Debug for BootstrapSourceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootstrapSourceRefusal")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for BootstrapSourceRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private bootstrap source is unavailable")
    }
}
impl std::error::Error for BootstrapSourceRefusal {}

/// Actual original-source owner, not constructible from context, snapshot bytes or a Server.
/// Failed preparation keeps its originals. A successfully sealed candidate is cached BEFORE
/// final negative checks; after transfer every prepare/borrow/take refuses without resealing.
pub struct FounderBootstrapPreparation {
    original: FounderArchivePreparation,
    context: BootstrapSourceContextData,
    prepared: Option<PreparedBootstrapSource>,
    key_binding: Option<Zeroizing<[u8; 32]>>,
    // Assigned ONLY by the successful one-time take. This inert ID permits a final original
    // source observation after ciphertext moved into prepaid Store custody.
    transferred: Option<BootstrapSourceId>,
}
impl std::fmt::Debug for FounderBootstrapPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderBootstrapPreparation")
            .finish_non_exhaustive()
    }
}

fn actual_spec<'a, T: MeshTransport, R: CryptoRngCore>(
    original: &'a FounderArchivePreparation,
    node: &'a ChannelSync<T, R>,
) -> Result<BootstrapSourceContextSpec<'a>, SyncError> {
    original.check_source(node)?;
    let package = original.package.as_ref().ok_or_else(unavailable)?;
    let (scope_domain, scope_nonce, planned_revision) = original
        .original
        .original
        .scope
        .planned_archive_context(&node.local_chat_scope)?;
    let private = original
        .original
        .borrow_private_snapshot_for_covering_save(node)?;
    let fresh = node.fresh_chat_admission().ok_or_else(unavailable)?;
    // check_source above binds physical original presence, full group/signer/authenticator,
    // complete cached current-source equality and all original negatives. These are observations,
    // NOT an atomic host commit permit; no replacement values enter this function.
    Ok(BootstrapSourceContextSpec {
        vault: original.vault,
        group: fresh.group(),
        signer: *fresh.signer(),
        fresh_attempt: fresh.attempt_id(),
        scope_domain,
        scope_nonce,
        planned_revision,
        profile: package.profile,
        admission: original.original.original.admission.id(),
        header: package.header,
        records: package.records.id(),
        manifest: package.manifest.id(),
        limits: private.limits,
        snapshot_digest: *private.digest.as_bytes(),
        records_plaintext_bytes: package.records_plaintext_bytes,
    })
}
impl FounderBootstrapPreparation {
    pub fn from_archive<T: MeshTransport, R: CryptoRngCore>(
        original: FounderArchivePreparation,
        node: &ChannelSync<T, R>,
    ) -> Result<Self, BootstrapSourceRefusal> {
        let context = actual_spec(&original, node).and_then(BootstrapSourceContextData::from_data);
        match context {
            Ok(context) => Ok(Self {
                original,
                context,
                prepared: None,
                key_binding: None,
                transferred: None,
            }),
            Err(_) => Err(BootstrapSourceRefusal {
                original: Box::new(original),
            }),
        }
    }

    fn check<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
    ) -> Result<(), SyncError> {
        if self.transferred.is_some() {
            return Err(unavailable());
        }
        self.check_original(node, key)
    }

    fn check_original<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
    ) -> Result<(), SyncError> {
        if actual_spec(&self.original, node)? != self.context.spec_data() {
            return Err(unavailable());
        }
        // The package was prepared FIRST. Refuse a replacement key even on the first source
        // preparation, before body allocation/RNG. Neither binding is a mounted-key permit.
        let package = self.original.package.as_ref().ok_or_else(unavailable)?;
        let archive_binding = Zeroizing::new(blake3::derive_key(super::KEY_BINDING_DOMAIN, key));
        let source_binding = Zeroizing::new(blake3::derive_key(KEY_BINDING_DOMAIN, key));
        if *package.key_binding != *archive_binding
            || self
                .key_binding
                .as_ref()
                .is_some_and(|original| **original != *source_binding)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Cache only once. Caller pays the separate source/body/cipher/frame and retained S0/SDK
    /// inventories BEFORE entry; encoded limits and the public archive fee do not pay this.
    pub fn prepare<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
        rng: &mut dyn CryptoRngCore,
    ) -> Result<&PreparedBootstrapSource, SyncError> {
        self.check(node, key)?;
        if self.prepared.is_none() {
            let body = codec::encode_original_body(&self.original, node, &self.context)?;
            let prepared = codec::seal_original_body(key, &body, self.context.clone(), rng)?;
            self.key_binding = Some(Zeroizing::new(blake3::derive_key(KEY_BINDING_DOMAIN, key)));
            self.prepared = Some(prepared);
            // Retain this exact unexposed candidate even on cancellation AFTER sealing. No
            // positive borrow or take can pass later; retry cannot silently draw another nonce.
        }
        self.borrow_prepared_for_handoff(node, key)
    }

    /// Observation only. Actual Store must hold its ORIGINAL atomic admission through the IO.
    pub fn borrow_prepared_for_handoff<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
    ) -> Result<&PreparedBootstrapSource, SyncError> {
        self.check(node, key)?;
        self.prepared.as_ref().ok_or_else(unavailable)
    }

    pub fn take_prepared_for_handoff<T: MeshTransport, R: CryptoRngCore>(
        &mut self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
    ) -> Result<PreparedBootstrapSource, SyncError> {
        self.check(node, key)?;
        let prepared = self.prepared.take().ok_or_else(unavailable)?;
        self.transferred = Some(prepared.id());
        Ok(prepared)
    }

    /// Negative-only original-source observation AFTER the complete ciphertext moved into
    /// the host's prepaid inert debt, before SourcePrepared mutation. This returns no bytes,
    /// token or permit and never reopens prepare/borrow/take. The host STILL holds original
    /// admission/head/root/debt/fee custody and repeats this after each actual writer wait.
    /// Exact offline repair after creator Drop instead uses the independently retained bytes.
    pub fn validate_transferred_source<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
        key: &[u8; 32],
        expected_context: &BootstrapSourceContextData,
        expected_id: BootstrapSourceId,
    ) -> Result<(), SyncError> {
        if self.transferred != Some(expected_id)
            || self.prepared.is_some()
            || self.key_binding.is_none()
            || &self.context != expected_context
        {
            return Err(unavailable());
        }
        self.check_original(node, key)
    }
}

#[cfg(test)]
mod tests;
