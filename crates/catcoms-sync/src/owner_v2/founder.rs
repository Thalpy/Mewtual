//! Real fresh-creation -> typed V2 genesis DATA. Never reconstruct this source from a Profile,
//! an epoch-zero snapshot or a matching current roster. No Server/device/transport escapes.
use crate::*;
use catcoms_replication::chat::{
    acceptance_tree::{Frontier, TreeContext},
    owner_facts::OwnerFactId,
    owner_header::{HeaderKind, HeaderSpec, OwnerHeaderId, SignedOwnerHeader},
    owner_profile::{
        AdmissionBody, AdmissionOrigin, AdmissionSpec, ProfileSpec, SignedOwnerProfile,
    },
    owner_state::{self, CurrentAdmissionSlot, StateFrame, StateMeta},
};
mod snapshot;
pub use snapshot::{FounderPrivateSnapshotView, FounderSnapshotDigest, FounderSnapshotLimits};
mod archive_candidate;
pub use archive_candidate::{
    open_bootstrap_source_data, prepare_v2_genesis_seed, ArchiveSourceRefusal,
    BootstrapSourceContextData, BootstrapSourceContextSpec, BootstrapSourceDataView,
    BootstrapSourceId, BootstrapSourceRefusal, FounderArchivePreparation,
    FounderBootstrapPreparation, GenesisArchiveView, OpenedBootstrapSourceData,
    PreparedBootstrapSource, PreparedUnboundV2GenesisNode, ReturnedV2GenesisAttach,
    UnboundV2GenesisNode, UnboundV2GenesisSeed, V2GenesisRuntimeParts, V2GenesisRuntimeRefusal,
    V2GenesisSeedData, V2GenesisSeedLimits, BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE,
    BOOTSTRAP_SOURCE_OPEN_BUFFER_BYTES, BOOTSTRAP_SOURCE_SEAL_BUFFER_BYTES,
    GENESIS_ARCHIVE_PRIVATE_INVOICE, MAX_BOOTSTRAP_SOURCE_FRAME_BYTES,
    MAX_BOOTSTRAP_SOURCE_PLAIN_BYTES, V2_GENESIS_SEED_METADATA_BYTES,
};

/// Conservative logical invoice for the named private public-record/scope/watch/signature
/// buffers in this producer, NOT a quota debit, SDK/provider heap or active Server bound.
/// The future actual host must reserve it BEFORE source creation/prepare; this Sync-only slice
/// supplies no fabricated paid flag or mounted authority. See the exact invoice/design doc.
pub const FOUNDER_PRIVATE_INVOICE: usize = 64 * 1024;

/// Non-Clone actual preparation. All public records stay private until a checked consuming
/// freeze; observations retain only ORIGINAL negatives, no source/runtime owner or revoker.
pub struct PreparedFounder {
    origin: chat_scope::OriginalFounderObservation,
    scope: local_chat_scope::OriginalInitializationObservation,
    authenticator: [u8; 32],
    profile: SignedOwnerProfile,
    admission: AdmissionBody,
    state: Vec<u8>,
    header: SignedOwnerHeader,
    frontier: Frontier,
}
impl std::fmt::Debug for PreparedFounder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2Founder").finish_non_exhaustive()
    }
}

/// Exact frozen public data with an optional ONE original private S0 cache. Neither is a
/// Saved/current/selected-head witness or publisher. Every DATA borrow repeats original checks.
pub struct FrozenFounder {
    original: PreparedFounder,
    profile: Vec<u8>,
    admission: Vec<u8>,
    header: Vec<u8>,
    snapshot: Option<snapshot::CachedFounderSnapshot>,
}
impl std::fmt::Debug for FrozenFounder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrozenV2Founder").finish_non_exhaustive()
    }
}

/// Borrowed immutable DATA for the eventual covering saver. It cannot construct a source,
/// native lease, Current or positive projection token. The actual host MUST hold its original
/// commit admission and save BEFORE protocol/UI export; these checks are observations only.
pub struct FounderDataView<'a> {
    pub profile: &'a [u8],
    pub admission: &'a [u8],
    pub state: &'a [u8],
    pub header: &'a [u8],
    pub frontier: &'a Frontier,
    /// Derived only AFTER signing/verifying this actual genesis header. This label is never
    /// fed back into its own state image, and is not selected-finality/covering-save authority.
    pub initial_tip_header: OwnerHeaderId,
}
impl std::fmt::Debug for FounderDataView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FounderDataView").finish_non_exhaustive()
    }
}

fn unavailable() -> SyncError {
    SyncError::ChatScopeUnavailable
}

/// Inventory emptiness checks are O(1), before signing/encoding. Actual MLS member/committer
/// observations can traverse SDK roster state: call ONLY after original fresh-source binding
/// has refused absent, restored, Welcome and advanced sources. This producer is then the actual
/// original one-founder epoch0 context. Never scan/encode history merely to refuse it. SDK
/// work and heap for those observations remain outside this private public-record invoice.
/// Constructor routing secret, file key/default tenure and initial policy are allowed.
fn untouched<T: MeshTransport, R: CryptoRngCore>(
    node: &ChannelSync<T, R>,
) -> Result<(), SyncError> {
    if node.group.epoch() != 0
        || node.group.member_count() != 1
        || !node.group.is_active()
        || node.group.designated_committer() != Some(node.device.device_id())
        || !node.group.has_local_signer(&node.device.public_key_bytes())
        || node.initial_owner_evidence.is_some()
        || node.owner_operation.is_some()
        || node.owner_direct_request.is_some()
        || !node.docs.is_empty()
        || !node.ledger.is_empty()
        || !node.durable_chat.initialization_empty()
        || !node.commit_log.is_empty()
        || !node.pending_commits.is_empty()
        || !node.past_keys.is_empty()
        || !node.outbox.is_empty()
        || !node.managed_chat_outbox.is_empty()
        || !node.eviction_outbox.is_empty()
        || !node.unevict_outbox.is_empty()
        || node.pending.is_some()
        || !node.welcome_outbox.is_empty()
        || !node.add_request_queue.is_empty()
        || !node.admit_results.is_empty()
        || !node.direct_admit_results.is_empty()
        || !node.admit_result_outbox.is_empty()
        || !node.outgoing_add_requests.is_empty()
        || !node.device_add_queue.is_empty()
        || !node.device_admit_results.is_empty()
        || !node.device_admit_outbox.is_empty()
        || !node.outgoing_device_adds.is_empty()
        || !node.companion_devices.is_empty()
        || !node.revoked_devices.is_empty()
        || !node.admitted_devices.is_empty()
        || !node.admin_roster.is_empty()
        || node.roster_gen != 0
        || !node.peer_records.is_empty()
        || node.control_subscribed
        || !node.routing_subs.is_empty()
        || node.routing_subscription_pending.is_some()
        || node.voice_runtime.is_some()
        || !node.pending_dm_invites.is_empty()
        || !node.pending_call_signals.is_empty()
        || !node.forwarded_joins.is_empty()
        || !node.forwarded_join_attempts.is_empty()
        || !node.delivery_receipt_outbox.is_empty()
        || !node.delivery_receipt_targets.is_empty()
        || !node.catchup_queue.is_empty()
        || node.catchup_inflight.is_some()
        || node.pending_catchup.is_some()
    {
        return Err(SyncError::OwnerInitializationUnsupported);
    }
    Ok(())
}

impl<T: MeshTransport, R: CryptoRngCore> ChannelSync<T, R> {
    /// Produce only the actual original founder's immutable V2 genesis records. This method
    /// takes no decoded Profile, creation scalar, snapshot, copied fresh proof or saved boolean.
    /// A host using a native scope must reserve/stage/attach it FIRST, so freeze observes the
    /// same original reservation. Preparing unmanaged DATA then changing scope cannot rebind.
    pub fn prepare_owner_v2_founder(&self) -> Result<PreparedFounder, SyncError> {
        let source = self.fresh_chat_admission().ok_or_else(unavailable)?;
        let origin = source
            .observe_original_founder(&self.group, &self.device)
            .ok_or_else(unavailable)?;
        untouched(self)?;
        let scope = self.local_chat_scope.observe_private_initialization()?;
        let group = source.group();
        if group.is_empty() || group.len() > 256 {
            return Err(SyncError::OwnerInitializationUnsupported);
        }
        let founder = *source.signer();
        let mut creation = blake3::Hasher::new();
        creation.update(b"mewtual/chat/founder-creation/v2\0");
        creation.update(&(group.len() as u32).to_be_bytes());
        creation.update(group);
        creation.update(&founder);
        creation.update(&source.attempt_id());
        let creation = *creation.finalize().as_bytes();
        let profile = SignedOwnerProfile::sign_using(
            ProfileSpec {
                group: group.to_vec(),
                founder,
                creation,
            },
            |bytes| {
                self.device
                    .sign(bytes)
                    .map_err(|_| catcoms_replication::chat::ChatError::Signature)
            },
        )
        .map_err(|_| unavailable())?;
        let admission = AdmissionBody::new(AdmissionSpec {
            group: group.to_vec(),
            profile: profile.id(),
            subject: founder,
            origin: AdmissionOrigin::Founder { creation },
        })
        .map_err(|_| unavailable())?;
        let frontier = Frontier::empty(TreeContext {
            profile: profile.id(),
            admission: admission.admission_id(),
        });
        let tip = frontier.tip();
        let body = admission.id();
        let image = owner_state::encode_state(
            StateMeta {
                group,
                profile: profile.id(),
                issuer: founder,
                tenure: profile.tenure(),
                parent: profile.genesis_parent(),
                previous: owner_state::virtual_genesis_state(&profile)
                    .map_err(|_| unavailable())?,
                sequence: 1,
                kind: HeaderKind::Admission,
                body,
                mls_epoch: 0,
            },
            &[CurrentAdmissionSlot {
                admission: body,
                subject: founder,
                tip_body: body,
                accepted_count: 0,
                accepted_root: tip.root,
            }],
        )
        .map_err(|_| unavailable())?;
        let state = StateFrame::decode(&image)
            .map_err(|_| unavailable())?
            .commitment();
        let header = SignedOwnerHeader::sign_using(
            HeaderSpec {
                group: group.to_vec(),
                profile: profile.id(),
                issuer: founder,
                tenure: profile.tenure(),
                parent: profile.genesis_parent(),
                sequence: 1,
                kind: HeaderKind::Admission,
                body,
                state,
            },
            |bytes| {
                self.device
                    .sign(bytes)
                    .map_err(|_| catcoms_replication::chat::ChatError::Signature)
            },
        )
        .map_err(|_| unavailable())?;
        let result = PreparedFounder {
            origin,
            scope,
            authenticator: self.group.epoch_authenticator_id(),
            profile,
            admission,
            state: image,
            header,
            frontier,
        };
        result.validate_original(self)?;
        Ok(result)
    }
}
impl PreparedFounder {
    /// Inert semantic label only; native/root/current authority is not created by reading it.
    pub fn profile_id(&self) -> OwnerFactId {
        self.profile.id()
    }

    pub fn validate_original<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<(), SyncError> {
        let source = node.fresh_chat_admission().ok_or_else(unavailable)?;
        if !self.origin.matches(source, &node.group, &node.device)
            || !self.scope.matches(&node.local_chat_scope)
            || self.authenticator != node.group.epoch_authenticator_id()
            || self.profile.spec().group != node.group.group_id()
            || self.profile.spec().founder.as_slice() != node.device.public_key_bytes()
        {
            return Err(unavailable());
        }
        untouched(node)?;
        StateFrame::decode(&self.state)
            .and_then(|image| {
                image.verify_genesis_binding(&self.profile, &self.admission, &self.header)
            })
            .map_err(|_| unavailable())?;
        let tip = self.frontier.tip();
        if tip.context
            != (TreeContext {
                profile: self.profile.id(),
                admission: self.admission.admission_id(),
            })
            || tip.count != 0
            || tip.root != tip.context.empty_hash()
            || self.frontier.hashes().next().is_some()
        {
            return Err(unavailable());
        }
        // Signature/image work is not an atomic native admission. Repeat ORIGINAL negatives
        // immediately before returning DATA; the actual saver still needs its held host gate.
        if !self.origin.matches(source, &node.group, &node.device)
            || !self.scope.matches(&node.local_chat_scope)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Encode exact public frames once, preserving this original source/scope and signatures.
    /// This is a trusted-host DATA freeze, never a covering save or permission to publish.
    pub fn freeze<T: MeshTransport, R: CryptoRngCore>(
        self,
        node: &ChannelSync<T, R>,
    ) -> Result<FrozenFounder, SyncError> {
        self.validate_original(node)?;
        let frozen = FrozenFounder {
            profile: self.profile.encode(),
            admission: self.admission.encode(),
            header: self.header.encode(),
            original: self,
            snapshot: None,
        };
        frozen.original.validate_original(node)?;
        Ok(frozen)
    }
}
impl FrozenFounder {
    /// Borrow this exact source's bounded public DATA for an eventual covering save. A saved
    /// host must still supply original native creation/fee/commit custody before exposing it.
    pub fn borrow_for_covering_save<T: MeshTransport, R: CryptoRngCore>(
        &self,
        node: &ChannelSync<T, R>,
    ) -> Result<FounderDataView<'_>, SyncError> {
        self.original.validate_original(node)?;
        // Each retained representation must still be the exact one originally signed. Current
        // callers get no mutable frame access; keep this check at the future saver boundary so
        // a later codec/refactor cannot accept an invalid same-CID signature representative.
        if self.profile != self.original.profile.encode()
            || self.admission != self.original.admission.encode()
            || self.header != self.original.header.encode()
        {
            return Err(unavailable());
        }
        self.check_cached_source(node)?;
        self.original.validate_original(node)?;
        Ok(FounderDataView {
            profile: &self.profile,
            admission: &self.admission,
            state: &self.original.state,
            header: &self.header,
            frontier: &self.original.frontier,
            initial_tip_header: self.original.header.id(),
        })
    }
}

#[cfg(test)]
mod tests;
