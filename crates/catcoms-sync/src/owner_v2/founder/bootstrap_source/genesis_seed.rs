//! Closed initial V2 DATA/resource reconstruction. No legacy synchronizer, runnable scheduler,
//! Fresh decoder, SDK getter, Current receipt or positive publication capability is exposed.
use super::*;
use crate::{
    chat_scope::{OriginalFounderObservation, VerifiedFreshChatAdmission},
    durable_chat::DurableChatState,
    local_chat_scope::{
        LocalChatScope, ManagedChatScopeReservation, OriginalInitializationObservation,
    },
    owner_tenure::OwnerTenure,
};
use catcoms_mls::{persist::OwnerSdkRestoreAdmission, GroupPolicy, MlsDevice, ServerGroup};
use catcoms_replication::chat::{
    acceptance_tree::TreeTip,
    owner_state::{StateFrame, MAX_STATE_BYTES},
};
use catcoms_rt::Clock;
use std::sync::Arc;

/// Negative closed-source allowances. These cannot reserve provider/metadata/SDK memory or
/// authenticate a source. The actual host must retain those separate fees for the node lifetime.
#[derive(Clone, Copy, Debug)]
pub struct V2GenesisSeedLimits(FounderSnapshotLimits);
impl V2GenesisSeedLimits {
    pub fn new(snapshot: usize, mls: usize, backing: usize) -> Result<Self, SyncError> {
        FounderSnapshotLimits::new(snapshot, mls, backing).map(Self)
    }
}

/// Named logical requested metadata/copy inventory, not a resource lease or physical RSS cap.
/// The caller's original provider/source/metadata fees must survive until the corresponding
/// seed/node allocations drain. SDK has its own actual lease in BOTH device/group. Runtime
/// parts, arbitrary host T/R and their Box/Arc allocations are additional caller obligations.
pub const V2_GENESIS_SEED_METADATA_BYTES: usize = std::mem::size_of::<SeedBody>()
    + std::mem::size_of::<s0::Fields<'static>>()
    + crate::MAX_ROUTING_SECRETS * std::mem::size_of::<(u64, Zeroizing<[u8; 32]>)>()
    + crate::initialization_snapshot::MAX_INITIAL_RENDEZVOUS
        * (std::mem::size_of::<(String, Zeroizing<Vec<u8>>)>() + 1024 + 128)
    + codec::VERIFIED_GENESIS_LAYOUT
    + MAX_STATE_BYTES
    + MAX_HEADER_ROW_BYTES
    + MAX_HEADER_BYTES
    + MAX_PROFILE_BYTES
    + MAX_ADMISSION_BYTES
    + 12 * MAX_GROUP_ID_BYTES
    + 4101
    + 98
    + 512
    + 16 * 1024; // policy/tenure/scope/verification and fixed key scratch

/// Empty ledger/docs/commits/peers/results/admin/obligation inventories are proved by the whole
/// closed parser. They are not allocated legacy maps or an invitation to run old dispatch.
struct ClosedEmptyInventories;
struct GenesisMetadata {
    routing_label: u64,
    routing: Vec<(u64, Zeroizing<[u8; 32]>)>,
    _file_wrap_key: Zeroizing<[u8; 32]>,
    rendezvous: Vec<(String, Zeroizing<Vec<u8>>)>,
    _tenure: OwnerTenure,
    policy: Option<GroupPolicy>,
    durable_version: u8,
    pending_version: u8,
    scope: LocalChatScope,
    _empty: ClosedEmptyInventories,
}
struct SeedBody {
    source: BootstrapSourceId,
    context: BootstrapSourceContextData,
    genesis: codec::VerifiedGenesisData,
    metadata: GenesisMetadata,
    // Both actual graph objects carry the SAME SDK lease last in their own field layout.
    device: MlsDevice,
    group: ServerGroup,
}

/// Opaque unbound initial graph + complete bounded metadata, never a runnable Server.
/// Private preallocated body makes later owning pairing refusals allocation-free.
pub struct UnboundV2GenesisSeed {
    body: Box<SeedBody>,
}

/// Borrowed verified public/label DATA. Neither this view nor its fields certify a saved root,
/// actual source custody, Current scope, publication, runtime liveness or a production issuer.
pub struct V2GenesisSeedData<'a> {
    pub source: BootstrapSourceId,
    pub context: &'a BootstrapSourceContextData,
    pub profile: &'a SignedOwnerProfile,
    pub admission: &'a AdmissionBody,
    pub header: &'a SignedOwnerHeader,
    pub state: &'a [u8],
    pub tip: TreeTip,
    pub scope: &'a crate::local_chat_scope::ChatScopeLabel,
    pub routing_label: u64,
    pub routing_slots: usize,
    pub rendezvous_rows: usize,
    pub has_policy: bool,
    pub durable_version: u8,
    pub pending_version: u8,
}
impl std::fmt::Debug for V2GenesisSeedData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2GenesisSeedData").finish_non_exhaustive()
    }
}
impl SeedBody {
    fn data(&self) -> V2GenesisSeedData<'_> {
        V2GenesisSeedData {
            source: self.source,
            context: &self.context,
            profile: &self.genesis.profile,
            admission: &self.genesis.admission,
            header: &self.genesis.header,
            state: &self.genesis.image,
            tip: self.genesis.frontier.tip(),
            // The only constructor below decodes an actual mandatory Active scope. No API can
            // remove that label, including failed/negative-only reservation attachment.
            scope: self
                .metadata
                .scope
                .genesis_label_data()
                .expect("closed genesis scope"),
            routing_label: self.metadata.routing_label,
            routing_slots: self.metadata.routing.len(),
            rendezvous_rows: self.metadata.rendezvous.len(),
            has_policy: self.metadata.policy.is_some(),
            durable_version: self.metadata.durable_version,
            pending_version: self.metadata.pending_version,
        }
    }
}

fn unavailable() -> SyncError {
    SyncError::OwnerInitializationUnsupported
}

/// Reconstruct ONLY verified initial epoch0/sole-founder DATA. Complete shared S0 framing and
/// all configured caps precede SDK entry; the actual bounded loader installs its row/type policy
/// before SIGNER. Source is borrowed unchanged on every error. No T/R/Clock is accepted here.
///
/// Caller must hold SDK plus raw provider/backing/payload, metadata, and opened-source fees
/// independently. Only the SDK payment is retained by this API; the actual host wrapper must
/// keep the others through seed/node Drop. Call outside Store/canonical writers so partial
/// graph cleanup cannot refund a host pool while that same writer/pool is held.
pub fn prepare_v2_genesis_seed(
    source: &OpenedBootstrapSourceData,
    limits: V2GenesisSeedLimits,
    sdk: OwnerSdkRestoreAdmission,
) -> Result<UnboundV2GenesisSeed, SyncError> {
    let view = source.view_data();
    let spec = view.context.spec_data();
    if spec.limits.max_snapshot_bytes() > limits.0.max_snapshot_bytes()
        || spec.limits.max_mls_bytes() > limits.0.max_mls_bytes()
        || spec.limits.max_provider_backing_bytes() > limits.0.max_provider_backing_bytes()
    {
        return Err(unavailable());
    }
    let fields = s0::fields(view.snapshot, spec)?;
    // Recheck actual retained public representatives and EXACT image, never synthesize another.
    StateFrame::decode(&source.genesis.image)
        .and_then(|state| {
            state.verify_genesis_binding(
                &source.genesis.profile,
                &source.genesis.admission,
                &source.genesis.header,
            )
        })
        .map_err(|_| unavailable())?;
    #[cfg(test)]
    SDK_ENTRIES.with(|n| n.set(n.get() + 1));
    let (device, group) = catcoms_mls::persist::restore_server_owner_checkpoint_bounded(
        fields.mls,
        // Host allowance covers these original negative caps; a larger configured account
        // must not silently widen the original source's declared provider/payload grammar.
        spec.limits.max_mls_bytes(),
        spec.limits.max_provider_backing_bytes(),
        sdk,
    )?;
    if group.group_id_ref() != spec.group
        || device.public_key_bytes().as_slice() != spec.signer.as_slice()
        || group.epoch() != 0
        || group.member_count() != 1
        || !group.is_active()
        || !group.has_local_signer(&spec.signer)
        || !group.is_designated_committer(&device)
    {
        return Err(unavailable());
    }
    let tenure = OwnerTenure::decode(fields.tenure, &group)?;
    let policy = crate::group_policy::decode_pin(fields.policy)?;
    if let Some(policy) = &policy {
        crate::group_policy::require_supported(policy)?;
        policy.verify_pin(&group)?;
    }
    // Reconstruct only exact bounded initial metadata. Directly push zeroizing secret slots,
    // avoiding an intermediate ordinary Vec of routing secrets from the legacy decoder.
    let mut routing = catcoms_wire::Decoder::new(fields.routing);
    let routing_label = routing.get_u64().map_err(|_| unavailable())?;
    let count = routing.get_u32().map_err(|_| unavailable())? as usize;
    let mut secrets = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = routing.get_u64().map_err(|_| unavailable())?;
        let secret = routing
            .get_bytes()
            .map_err(|_| unavailable())?
            .try_into()
            .map_err(|_| unavailable())?;
        secrets.push((slot, Zeroizing::new(secret)));
    }
    routing.finish().map_err(|_| unavailable())?;
    let mut rendezvous = Vec::with_capacity(fields.rendezvous_count);
    for (address, value) in fields.rendezvous.iter().flatten() {
        rendezvous.push(((*address).to_owned(), Zeroizing::new(value.to_vec())));
    }
    // Closed parser proved no saved-local work exists. Decode the actual original scope using
    // the same semantic checker as legacy, but do not construct/bind/run a ChannelSync.
    let mut empty_chat = DurableChatState::default();
    let scope = LocalChatScope::decode(fields.scope, &group, &device, &mut empty_chat)?;
    let metadata = GenesisMetadata {
        routing_label,
        routing: secrets,
        _file_wrap_key: Zeroizing::new(*fields.file_wrap_key),
        rendezvous,
        _tenure: tenure,
        policy,
        durable_version: fields.durable_version,
        pending_version: fields.pending_version,
        scope,
        _empty: ClosedEmptyInventories,
    };
    Ok(UnboundV2GenesisSeed {
        body: Box::new(SeedBody {
            source: source.id,
            context: view.context.clone(),
            genesis: source.genesis.clone(),
            metadata,
            device,
            group,
        }),
    })
}

impl UnboundV2GenesisSeed {
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.body.data()
    }

    /// Allocation-free NEGATIVE DATA consistency against the exact opaque opened source.
    /// Compare whole source/context and full retained public representatives, not just CIDs.
    /// This performs no encode, signature/SDK operation, callback, pool access or mutation.
    ///
    /// Success is NOT SourceSaved, original file/mount liveness, a resource payment, Current
    /// or permission to publish. App must separately validate its actual paid source capture
    /// against the already-held original Root and repeat its concrete admission checks.
    pub fn check_opened_source_data(
        &self,
        source: &OpenedBootstrapSourceData,
    ) -> Result<(), SyncError> {
        if self.body.source != source.id
            || self.body.context != *source.view_data().context
            || self.body.genesis.profile != source.genesis.profile
            || self.body.genesis.admission != source.genesis.admission
            || self.body.genesis.header != source.genesis.header
            || self.body.genesis.image != source.genesis.image
            || self.body.genesis.frontier != source.genesis.frontier
        {
            return Err(unavailable());
        }
        Ok(())
    }

    /// Negative/DATA attachment only, never publication. The actual host still checks its
    /// original mounted Saved/Current/pin/head capture. A replacement owner cannot renew this.
    pub fn prepare_scope(
        &mut self,
        original: &impl ManagedChatScopeReservation,
    ) -> Result<(), SyncError> {
        self.body.metadata.scope.attach_genesis_reservation(
            &self.body.group,
            &self.body.device,
            original,
            self.body.context.spec_data().planned_revision,
        )
    }
    /// Checked no-allocation pairing. Wrong source, dead original or replacement reservation
    /// returns BOTH same owners. The resulting prepared object remains DATA, not App Current.
    pub fn prepare_runtime<T: MeshTransport, R: CryptoRngCore>(
        self,
        parts: V2GenesisRuntimeParts<T, R>,
    ) -> Result<PreparedUnboundV2GenesisNode<T, R>, ReturnedV2GenesisAttach<T, R>> {
        let matched = self.body.metadata.scope.genesis_attachment_live()
            && match (&parts.body.warm, &parts.body.fresh) {
                (None, None) => true,
                (Some(warm), Some(fresh)) => {
                    warm.source == self.body.source
                        && warm.context == self.body.context
                        && warm.fresh.matches_genesis_restored(
                            fresh,
                            &self.body.group,
                            &self.body.context.spec_data().signer,
                        )
                        && warm
                            .scope
                            .matches_genesis_attachment(&self.body.metadata.scope)
                }
                _ => false,
            };
        if !matched {
            return Err(ReturnedV2GenesisAttach { seed: self, parts });
        }
        Ok(PreparedUnboundV2GenesisNode { seed: self, parts })
    }
}

struct WarmOrigin {
    source: BootstrapSourceId,
    context: BootstrapSourceContextData,
    fresh: OriginalFounderObservation,
    scope: OriginalInitializationObservation,
}
struct RuntimeBody<T, R> {
    _transport: Arc<T>,
    _rng: R,
    _clock: Arc<dyn Clock + Send>,
    warm: Option<WarmOrigin>,
    // Own the actual original FreshOriginLife; observations alone must not keep it alive.
    fresh: Option<VerifiedFreshChatAdmission>,
}
/// Opaque runtime DATA with no scheduler or transport/Fresh getter. The private Box is allocated
/// before checked pairing, under a separate host runtime-layout allowance (T/R are arbitrary).
pub struct V2GenesisRuntimeParts<T, R> {
    body: Box<RuntimeBody<T, R>>,
}
impl<T: MeshTransport, R: CryptoRngCore> V2GenesisRuntimeParts<T, R> {
    /// Named requested runtime-Box + maximum warm context-group copy. DATA only, not a fee.
    /// Actual T/Clock Arc allocations already exist and are independently owned/paid by caller.
    pub fn allocation_bytes() -> usize {
        std::mem::size_of::<RuntimeBody<T, R>>() + MAX_GROUP_ID_BYTES
    }

    /// Additional owning-refusal Box layout for warm extraction, outside host writers. The
    /// originals' graph/cache allocations remain live under their existing separate fees.
    pub fn warm_refusal_allocation_bytes() -> usize {
        std::mem::size_of::<(FounderBootstrapPreparation, ChannelSync<T, R>)>()
    }

    /// Cold DATA input only: never infer Fresh life from a historical attempt label. The caller
    /// supplies already allocated original runtime Arcs; the actual Root cold capture and
    /// Current host authorize them later. This creates only the explicitly documented Box.
    pub fn cold_data(transport: Arc<T>, rng: R, clock: Arc<dyn Clock + Send>) -> Self {
        Self {
            body: Box::new(RuntimeBody {
                _transport: transport,
                _rng: rng,
                _clock: clock,
                warm: None,
                fresh: None,
            }),
        }
    }
}

/// Original source+sync owning refusal. Boxing is done outside writers, never by final pairing.
pub struct V2GenesisRuntimeRefusal<T: MeshTransport, R: CryptoRngCore> {
    originals: Box<(FounderBootstrapPreparation, ChannelSync<T, R>)>,
}
impl<T: MeshTransport, R: CryptoRngCore> V2GenesisRuntimeRefusal<T, R> {
    pub fn into_originals(self) -> (FounderBootstrapPreparation, ChannelSync<T, R>) {
        *self.originals
    }
}
impl FounderBootstrapPreparation {
    /// Consume actual unchanged transferred source/runtime AFTER App's own saved-source checks.
    /// No replacement T/R/Clock and no fresh creation occurs. On refusal return both SAME original
    /// objects; on success old SDK/cache/source allocations drain outside all host writers while
    /// the actual original Fresh lifetime moves into the parts. Root separately keeps its plan
    /// sender alive. This resource/DATA handoff does not certify a saved source or Current row.
    pub fn into_genesis_runtime_parts<T: MeshTransport, R: CryptoRngCore>(
        self,
        original: ChannelSync<T, R>,
        expected_id: BootstrapSourceId,
    ) -> Result<V2GenesisRuntimeParts<T, R>, V2GenesisRuntimeRefusal<T, R>> {
        let observations = (|| {
            if self.transferred != Some(expected_id)
                || self.prepared.is_some()
                || self.key_binding.is_none()
                || actual_spec(&self.original, &original)? != self.context.spec_data()
            {
                return Err(unavailable());
            }
            let fresh = original
                .fresh_chat_admission()
                .and_then(|source| {
                    source.observe_original_founder(&original.group, &original.device)
                })
                .ok_or_else(unavailable)?;
            let scope = original.local_chat_scope.observe_private_initialization()?;
            Ok((fresh, scope))
        })();
        let (fresh, scope) = match observations {
            Ok(pair) => pair,
            Err(_) => {
                return Err(V2GenesisRuntimeRefusal {
                    originals: Box::new((self, original)),
                })
            }
        };
        let warm = WarmOrigin {
            source: expected_id,
            context: self.context.clone(),
            fresh,
            scope,
        };
        // All fallible checks finished before taking any original field. ChannelSync has no
        // custom Drop; destructuring moves the exact three runtime owners and Fresh token.
        let ChannelSync {
            transport,
            rng,
            clock,
            fresh_chat_admission,
            ..
        } = original;
        Ok(V2GenesisRuntimeParts {
            body: Box::new(RuntimeBody {
                _transport: transport,
                _rng: rng,
                _clock: clock,
                warm: Some(warm),
                fresh: fresh_chat_admission,
            }),
        })
    }
}

/// Own both same inputs on checked pairing refusal; no Box/allocation is made at this edge.
pub struct ReturnedV2GenesisAttach<T, R> {
    seed: UnboundV2GenesisSeed,
    parts: V2GenesisRuntimeParts<T, R>,
}
impl<T, R> ReturnedV2GenesisAttach<T, R> {
    pub fn into_originals(self) -> (UnboundV2GenesisSeed, V2GenesisRuntimeParts<T, R>, SyncError) {
        // This checked DATA edge has one fixed refusal class. Do not retain a potentially
        // growing general SyncError inside the no-allocation owning error envelope.
        (self.seed, self.parts, unavailable())
    }
}
/// Exact checked ownership pairing, still DATA. Root's actual saved capture/pin/head/row/mount
/// and final negative check are mandatory before a private App Current can own the node.
pub struct PreparedUnboundV2GenesisNode<T, R> {
    seed: UnboundV2GenesisSeed,
    parts: V2GenesisRuntimeParts<T, R>,
}
impl<T, R> PreparedUnboundV2GenesisNode<T, R> {
    /// Infallible field moves only: no allocation, callback, SDK/clock/RNG or transport action.
    pub fn into_node(self) -> UnboundV2GenesisNode<T, R> {
        UnboundV2GenesisNode {
            seed: self.seed,
            _parts: self.parts,
        }
    }
}
/// Complete private genesis graph/metadata/runtime ownership. Intentionally has no run,
/// authoring, mutable SDK, Server conversion or positive issuer API. New governor is separate.
pub struct UnboundV2GenesisNode<T, R> {
    seed: UnboundV2GenesisSeed,
    _parts: V2GenesisRuntimeParts<T, R>,
}
impl<T, R> UnboundV2GenesisNode<T, R> {
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.seed.data()
    }
}

macro_rules! sanitized_debug {
    ($($name:ident),* $(,)?) => {$(
        impl<T: MeshTransport, R: CryptoRngCore> std::fmt::Debug for $name<T, R> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        }
    )*};
}
sanitized_debug!(
    V2GenesisRuntimeParts,
    V2GenesisRuntimeRefusal,
    ReturnedV2GenesisAttach,
    PreparedUnboundV2GenesisNode,
    UnboundV2GenesisNode
);
impl std::fmt::Debug for UnboundV2GenesisSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnboundV2GenesisSeed")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
std::thread_local! {
    static SDK_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
mod tests;
