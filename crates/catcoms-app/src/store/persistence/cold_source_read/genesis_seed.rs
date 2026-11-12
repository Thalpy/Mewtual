//! Actual source-read account -> independently paid provider/metadata and SDK graph. This
//! owning DATA capsule cannot expose a raw owned seed while silently releasing those fees.
//! All decode/partial cleanup occurs outside admission/pin/Root/SDK host writers.
use super::*;
mod current;
use catcoms_mls::persist::{
    OwnerSdkGraphReservation, OwnerSdkRestoreAdmission, OwnerSdkRestoreLimits,
};
use catcoms_sync::owner_v2::{
    prepare_v2_genesis_seed, FounderSnapshotLimits, OriginalV2GenesisSeedObservation,
    UnboundV2GenesisSeed, V2GenesisSeedData, V2GenesisSeedLimits, V2_GENESIS_SEED_METADATA_BYTES,
};
pub(in crate::store) use current::settlement::GenesisCurrentOwnerRef;
pub use current::{
    MemberSaveJob, MemberSaveJobReport, MemberSaveNativeAdmission, PreparedColdV2GenesisCurrent,
    PreparedColdV2GenesisInitializing, PreparedColdV2GenesisPreInitializing,
    PreparedV2GenesisCurrent, PreparedV2TextHost, PreparedV2TextRead,
    UnexposedColdV2GenesisCurrent, UnexposedRepairedV2GenesisCurrent, UnexposedV2GenesisCurrent,
    V2MemberDependency, V2MemberSaveOutcome, V2MemberSaveRefusal, V2MemberSignRequest,
    V2TextHostPolicy,
};
pub(in crate::store) use current::{
    OriginalV2TextResidentObservation, OriginalV2UnsignedPlanObservation,
    PreparedV2NextUnsignedAppend, PreparedV2TextResidentProjection,
    PreparedV2UnsignedOriginalCustody, V2AdvancedAppendBasis, V2TextResidentParentData,
};
pub(in crate::store) use current::{
    PreparedV2TextSourceObserver, V2TextOwnedCurrent, V2TextOwnerRef, V2TextSourceNegativeObserver,
    V2_TEXT_SOURCE_OBSERVER_BYTES,
};
#[cfg(test)]
thread_local! { static SDK_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// Trusted local negative caps, never source authority or payment. A larger host allowance
/// cannot widen the original source's declared grammar; Sync checks both before SDK entry.
#[derive(Clone, Copy, Debug)]
pub struct GenesisSeedBudget {
    source: FounderSnapshotLimits,
    sdk: OwnerSdkRestoreLimits,
}
impl GenesisSeedBudget {
    /// Negative compatibility check for a closed fresh producer before any
    /// creation/source grant is spent. These caps do not pay for SDK work.
    pub(in crate::store) fn supports_founder_limits(
        &self,
        required: FounderSnapshotLimits,
    ) -> bool {
        self.source.max_snapshot_bytes() >= required.max_snapshot_bytes()
            && self.source.max_mls_bytes() >= required.max_mls_bytes()
            && self.source.max_provider_backing_bytes() >= required.max_provider_backing_bytes()
    }

    pub fn new(
        snapshot: usize,
        mls: usize,
        backing: usize,
        sdk_bytes: usize,
        sdk_work: usize,
    ) -> Result<Self, AppError> {
        use critical_namespaces::refused;
        let source = FounderSnapshotLimits::new(snapshot, mls, backing).map_err(|_| refused())?;
        if sdk_bytes == 0
            || sdk_bytes > 16 * 1024 * 1024
            || sdk_work == 0
            || sdk_work > 64 * 1024 * 1024
        {
            return Err(refused());
        }
        let budget = Self {
            source,
            sdk: OwnerSdkRestoreLimits {
                max_typed_bytes: sdk_bytes,
                max_work: sdk_work,
            },
        };
        budget.metadata_bytes()?;
        Ok(budget)
    }
    fn metadata_bytes(self) -> Result<usize, AppError> {
        self.source
            .max_mls_bytes()
            .checked_mul(2)
            .and_then(|n| n.checked_add(self.source.max_provider_backing_bytes()))
            .and_then(|n| n.checked_add(V2_GENESIS_SEED_METADATA_BYTES))
            .and_then(|n| n.checked_add(std::mem::size_of::<RetainedBootstrapGenesisSeed>()))
            .and_then(|n| n.checked_add(std::mem::size_of::<ReturnedBootstrapSourceData>()))
            .and_then(|n| {
                n.checked_add(crate::store::persistence::ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES)
            })
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<OriginalV2GenesisSeedObservation>() + 64)
            })
            .and_then(|n| n.checked_add(16 * 1024)) // bounded wrapper/verification/return scratch
            .ok_or_else(critical_namespaces::refused)
    }
    fn seed_limits(self) -> Result<V2GenesisSeedLimits, AppError> {
        Ok(V2GenesisSeedLimits::new(
            self.source.max_snapshot_bytes(),
            self.source.max_mls_bytes(),
            self.source.max_provider_backing_bytes(),
        )?)
    }
}

/// Fixed diagnostic class. It carries no growing SDK/I/O error, secret source text or proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenesisSeedRefusal {
    SourceUnavailable,
    CapacityUnavailable,
    SeedUnavailable,
}

/// Every refusal returns the SAME complete source and original read fee. A retry may acquire
/// NEW SDK/provider credits without another large read or another source-write unit.
pub struct ReturnedBootstrapSourceData {
    source: RetainedBootstrapSourceData,
    reason: GenesisSeedRefusal,
}
// Never box an error AFTER an actual quota refusal. Its source already owns the prepaid
// metadata Box, so the complete original can move through this fixed owning return envelope.
const _: () = assert!(std::mem::size_of::<ReturnedBootstrapSourceData>() <= 128);
impl ReturnedBootstrapSourceData {
    pub fn into_parts(self) -> (RetainedBootstrapSourceData, GenesisSeedRefusal) {
        (self.source, self.reason)
    }
}
impl std::fmt::Debug for ReturnedBootstrapSourceData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReturnedBootstrapSourceData")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// Real graph resource only: no Store/key/provider/source/sender. The SDK caches limits once
/// before locks and retains its SAME internal Arc LAST in both actual device and group.
struct GraphReservation {
    limits: OwnerSdkRestoreLimits,
    _fee: critical_namespaces::read_pool::CatalogReadReservation,
}
impl OwnerSdkGraphReservation for GraphReservation {
    fn limits(&self) -> OwnerSdkRestoreLimits {
        self.limits
    }
}

/// Opaque source + closed genesis graph/metadata and actual independent payments. No owned
/// seed extractor, mutable SDK, Server, runtime attach, Current or native result exists here.
/// The actual Current factory must carry this complete resource capsule through its node
/// transition, with separately prepaid generic runtime layouts before creating those parts.
pub struct RetainedBootstrapGenesisSeed {
    seed: UnboundV2GenesisSeed,
    resources: BootstrapGenesisResources,
}
/// The SAME supporting ownership moves with the graph into its node. Its private constructor
/// captures both exact original read and SDK-seed identities, not just equal source labels.
struct BootstrapGenesisResources {
    original_seed: OriginalV2GenesisSeedObservation,
    original_read: crate::store::persistence::OriginalBootstrapReadObservation,
    source: RetainedBootstrapSourceData,
    // LAST: graph/raw provider and seed metadata/source copies drain before this refund.
    _metadata_fee: critical_namespaces::read_pool::CatalogReadReservation,
}

/// Borrow BOTH the actual immutable closed seed and its original supporting payments. Only
/// real paid owners in this module construct this view; no raw labels/adopter/trait can do so.
/// Later Node ownership uses the identical view for final checks AFTER pairing callbacks.
#[derive(Clone, Copy)]
pub(in crate::store) struct GenesisSourceDataRef<'owner> {
    seed: &'owner UnboundV2GenesisSeed,
    resources: &'owner BootstrapGenesisResources,
}
impl<'owner> GenesisSourceDataRef<'owner> {
    pub(in crate::store) fn seed(&self) -> &'owner UnboundV2GenesisSeed {
        self.seed
    }
    pub(in crate::store) fn data(&self) -> V2GenesisSeedData<'owner> {
        self.seed.data()
    }
    pub(in crate::store) fn source(&self) -> &'owner RetainedBootstrapSourceData {
        &self.resources.source
    }
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.resources
            .original_seed
            .require_same_seed_data(self.seed)?;
        self.resources
            .original_read
            .require_same_original(self.source())?;
        self.source().require_original_negative(store)
    }
    pub(in crate::store) fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_original_negative(store)?;
        self.source().validate_original(store)
    }
}
impl std::fmt::Debug for RetainedBootstrapGenesisSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetainedBootstrapGenesisSeed")
            .finish_non_exhaustive()
    }
}
impl RetainedBootstrapGenesisSeed {
    /// Borrowed DATA only; the opaque seed has no run/Server/SDK getter. Rust borrowing keeps
    /// this capsule's separate non-SDK fees alive through every use of this reference.
    pub fn seed(&self) -> &UnboundV2GenesisSeed {
        self.borrow_source_data().seed()
    }
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.borrow_source_data().data()
    }
    pub fn source(&self) -> &RetainedBootstrapSourceData {
        self.borrow_source_data().source()
    }
    pub fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.borrow_source_data().validate_original(store)
    }
    /// Closed original-read lifetime checks for an ALREADY-held host writer. This grants no
    /// Saved/Current authority and never acquires Root/pool/SDK locks or touches the filesystem.
    /// Runtime consumers use the same check through their concrete borrowed source view.
    #[cfg(test)]
    pub(in crate::store) fn require_original_negative(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.borrow_source_data().require_original_negative(store)
    }
    pub(in crate::store) fn borrow_source_data(
        &self,
    ) -> crate::store::persistence::GenesisSourceDataRef<'_> {
        GenesisSourceDataRef {
            seed: &self.seed,
            resources: &self.resources,
        }
    }
}

impl ServerStore {
    /// Actual owning source→SDK DATA consumer. Invoke OUTSIDE all host writers: the bounded
    /// SDK decoder and every partial/refused graph can release payments only after its locks
    /// drain. Caller-supplied negative caps do not stand in for the two real debits below.
    pub fn prepare_genesis_seed(
        &self,
        source: RetainedBootstrapSourceData,
        budget: GenesisSeedBudget,
    ) -> Result<RetainedBootstrapGenesisSeed, ReturnedBootstrapSourceData> {
        let result = (|| {
            source
                .validate_original(self)
                .map_err(|_| GenesisSeedRefusal::SourceUnavailable)?;
            let pool = &self
                .archive_bootstrap_policy
                .as_ref()
                .ok_or(GenesisSeedRefusal::SourceUnavailable)?
                .pending
                .pool;
            let metadata_bytes = budget
                .metadata_bytes()
                .map_err(|_| GenesisSeedRefusal::CapacityUnavailable)?;
            // Declare this independent fee BEFORE the SDK/seed locals. All partial graphs
            // drain first on every refusal, without any host writer being held.
            let metadata = pool
                .try_reserve(metadata_bytes)
                .map_err(|_| GenesisSeedRefusal::CapacityUnavailable)?;
            let graph = pool
                .try_reserve(budget.sdk.max_typed_bytes)
                .map_err(|_| GenesisSeedRefusal::CapacityUnavailable)?;
            let sdk = OwnerSdkRestoreAdmission::new(GraphReservation {
                limits: budget.sdk,
                _fee: graph,
            })
            .map_err(|_| GenesisSeedRefusal::SeedUnavailable)?;
            let limits = budget
                .seed_limits()
                .map_err(|_| GenesisSeedRefusal::SeedUnavailable)?;
            #[cfg(test)]
            let hook = self
                .genesis_before_sdk
                .lock()
                .map_err(|_| GenesisSeedRefusal::SourceUnavailable)?
                .take();
            #[cfg(test)]
            if let Some(hook) = hook {
                hook();
            }
            // Resource acquisition can wait. Do not lend its earlier successful source
            // observation to a NEW expensive decode after the original became unavailable.
            source
                .validate_original(self)
                .map_err(|_| GenesisSeedRefusal::SourceUnavailable)?;
            #[cfg(test)]
            SDK_ENTRIES.with(|entries| entries.set(entries.get() + 1));
            let seed = prepare_v2_genesis_seed(source.data(), limits, sdk)
                .map_err(|_| GenesisSeedRefusal::SeedUnavailable)?;
            #[cfg(test)]
            let hook = self
                .genesis_after_sdk
                .lock()
                .map_err(|_| GenesisSeedRefusal::SourceUnavailable)?
                .take();
            #[cfg(test)]
            if let Some(hook) = hook {
                hook();
            }
            // A source/control/mount may change while decoding. A paid graph is not enough
            // for a stale source return, much less a Current/head/scope admission.
            source
                .validate_original(self)
                .map_err(|_| GenesisSeedRefusal::SourceUnavailable)?;
            Ok((seed, metadata))
        })();
        match result {
            Ok((seed, metadata)) => Ok(RetainedBootstrapGenesisSeed {
                resources: BootstrapGenesisResources {
                    original_seed: seed.observe_original_seed_data(),
                    original_read: source.observe_original(),
                    source,
                    _metadata_fee: metadata,
                },
                seed,
            }),
            Err(reason) => Err(ReturnedBootstrapSourceData { source, reason }),
        }
    }
}

#[cfg(test)]
mod tests;
