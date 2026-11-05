//! Actual source-read account -> independently paid provider/metadata and SDK graph. This
//! owning DATA capsule cannot expose a raw owned seed while silently releasing those fees.
//! All decode/partial cleanup occurs outside admission/pin/Root/SDK host writers.
use super::*;
use catcoms_mls::persist::{
    OwnerSdkGraphReservation, OwnerSdkRestoreAdmission, OwnerSdkRestoreLimits,
};
use catcoms_sync::owner_v2::{
    prepare_v2_genesis_seed, FounderSnapshotLimits, UnboundV2GenesisSeed, V2GenesisSeedData,
    V2GenesisSeedLimits, V2_GENESIS_SEED_METADATA_BYTES,
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
    source: RetainedBootstrapSourceData,
    // LAST: graph/raw provider and seed metadata/source copies drain before this refund.
    _metadata_fee: critical_namespaces::read_pool::CatalogReadReservation,
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
        &self.seed
    }
    pub fn data(&self) -> V2GenesisSeedData<'_> {
        self.seed.data()
    }
    pub fn source(&self) -> &RetainedBootstrapSourceData {
        &self.source
    }
    pub fn validate_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.source.validate_original(store)
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
                seed,
                source,
                _metadata_fee: metadata,
            }),
            Err(reason) => Err(ReturnedBootstrapSourceData { source, reason }),
        }
    }
}

#[cfg(test)]
mod tests;
