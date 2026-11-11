//! Original physical namespace3 declaration and opaque historical DATA reader. Neither the
//! descriptor nor this wrapper grants a Current scope, group writer or runnable SDK/server.
use super::*;
pub(in crate::store) use text::{
    AdmittedV2TextCompletedWorkspaceRead, BoundV2TextCompletedCreatePredecessor,
    PreparedV2TextCompletedWorkspaceReadCapture, V2TextCompletedWorkspaceSourceSlot,
    V2_TEXT_COMPLETED_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};

pub(in crate::store) use text::{
    AdmittedV2TextAcceptedCreateCommit, AdmittedV2TextAcceptedCreateStage,
    SavedAcceptedV2TextAdvanceGuard, SavedAcceptedV2TextFinalGuard,
    V2_TEXT_ACCEPTED_ADVANCE_GUARD_LAYOUT_BYTES, V2_TEXT_ACCEPTED_CREATE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextAcceptedQuery, PreparedV2TextAcceptedQueryCapture, V2TextAcceptedQuerySourceSlot,
    V2_TEXT_ACCEPTED_QUERY_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextAcceptedRetirementCompletion, AdmittedV2TextAcceptedRetirementDeletion,
    V2_TEXT_RETIREMENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextIntentCommit, AdmittedV2TextIntentStage, V2_TEXT_INTENT_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextPromotionScratch, AdmittedV2TextPromotionSeedRepair,
    PreparedV2TextPromotionScratchCapture, PreparedV2TextPromotionSeedRepairCapture,
    V2TextPromotionScratchSourceSlot, V2TextPromotionSeedRepairSourceSlot,
    V2_TEXT_PROMOTION_REPAIR_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextRawSignatureCommit, AdmittedV2TextRawSignatureStage,
    V2_TEXT_RAW_SIGNATURE_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextRetainRecoveryRead, AdmittedV2TextRetainRepair, AdmittedV2TextRetainScratch,
    PreparedV2TextRetainRecoveryCapture, PreparedV2TextRetainRepairLease,
    PreparedV2TextRetainScratchLease, V2TextRetainRecoverySourceSlot,
    V2_TEXT_RETAIN_RECOVERY_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{PreparedV2TextAcceptedQueryLease, V2TextAcceptedQueryLease};
pub(in crate::store) use text::{
    SavedCompletedV2TextFinishGuard, V2_TEXT_COMPLETED_FINISH_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    SavedUnsignedV2TextSigningGuard, V2_TEXT_SIGNING_GUARD_LAYOUT_BYTES,
};
mod declaration;
mod settlement;
mod text;
pub(in crate::store) use settlement::{
    GenesisSeedSettlementGroupGuard, SettledGenesisCurrentGroupGuard,
    GENESIS_SEED_SETTLEMENT_GUARD_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextPromotionCommit, AdmittedV2TextPromotionStage, PreparedV2TextPhysicalLease,
    V2TextCurrentGroupGuard, V2_TEXT_CURRENT_GROUP_GUARD_BYTES,
    V2_TEXT_PROMOTION_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextRetainCommit, AdmittedV2TextRetainStage, V2_TEXT_RETAIN_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextTokenQuery, PreparedV2TextQueryLease, PreparedV2TextTokenQueryCapture,
    V2TextQueryLease, V2TextTokenQuerySourceSlot, V2_TEXT_TOKEN_QUERY_ADMISSION_LAYOUT_BYTES,
};
pub(in crate::store) use text::{
    AdmittedV2TextWorkspaceRead, BoundV2TextCreatePredecessor, PreparedV2TextWorkspaceReadCapture,
    V2TextWorkspaceSourceSlot, V2_TEXT_WORKSPACE_ADMISSION_LAYOUT_BYTES,
};
// The explicit ID3 wrapper alone admits the connected cold library path. Default/ID2
// openers still refuse this layout before their older cleanup paths can run.
pub(in crate::store) mod cold_read;
pub(in crate::store) use cold_read::ColdV2InitializingGroupReadGuard;
pub(in crate::store) use cold_read::{ColdV2CurrentGroupReadGuard, COLD_V2_GROUP_READ_GUARD_BYTES};
pub(in crate::store) use group_task::cold_current::{
    ColdOwnedV2CurrentGroupGuard, ColdV2CurrentProofGroupGuard,
};

pub(in crate::store) use group_task::cold_initializing::{
    ColdInitializingV2GenesisReadGroupGuard, ColdInitializingV2GenesisRepairGroupGuard,
    ColdRepairedV2GenesisCurrentGroupGuard, ColdRepairedV2GenesisCurrentProofGroupGuard,
    COLD_INITIALIZING_GROUP_GUARD_BYTES,
};
pub(in crate::store) mod group_task;
pub(in crate::store) use group_task::cold_preinitializing::{
    ColdV2GenesisVacantGroupReadGuard, DurableBornV2GenesisRow1GroupGuard,
    COLD_PREINITIALIZING_GROUP_GUARD_BYTES,
};
pub(in crate::store) use group_task::{
    CreatingV2GenesisDirectoryGuard, InitializingV2GenesisGroupGuard,
    OriginalV2GenesisGroupObservation, PreparedV2GenesisGroupNamespace, SavedV2GroupDirectory,
    VacantV2GenesisGroupGuard,
};

pub(in crate::store) use super::current_group::{
    DECLARATION_SEED as CURRENT_DECLARATION_SEED, MARKER as CURRENT_MARKER,
};
pub(in crate::store) const MAX_OPEN_BASE_PATH: usize = 4096;

/// Trusted, read-only OLD filename-association policy, not a checkpoint/SDK budget or an
/// authorization to open a head. Count BOTH current/stage files; never infer a unique-head
/// count from a partial directory walk. Scope map and index fees are separately acquired.
#[derive(Clone, Copy, Debug)]
pub struct CanonicalFilenameInventoryPolicy {
    pub(in crate::store) max_files: usize,
    pub(in crate::store) max_file_bytes: u64,
}
impl CanonicalFilenameInventoryPolicy {
    pub fn new(max_files: usize, max_file_bytes: u64) -> Result<Self, AppError> {
        let maximum = 144 * 1024 * 1024 + 1024 + 40;
        if max_files == 0
            || max_files > 2 * crate::store::chat_admission::MAX_CHAT_ADMISSION_IDENTITIES
            || max_file_bytes == 0
            || max_file_bytes > maximum
        {
            return Err(refused());
        }
        Ok(Self {
            max_files,
            max_file_bytes,
        })
    }
}

/// Explicit private physical-open configuration. It is installed BEFORE the common opener,
/// never read from a catalog, and cannot enable a Current scope or a live actor. The opaque
/// completed wrapper exposes only the actual historical paid DATA reader; Root's separate
/// Current constructor still needs its actual saved source/pin/head/row/runtime barriers.
#[derive(Clone)]
pub struct V2CurrentLayoutPolicy {
    pub(in crate::store) archive: ArchiveBootstrapCatalogPolicy,
    pub(in crate::store) canonical: CanonicalFilenameInventoryPolicy,
    pub(in crate::store) persistent_capacity: usize,
    /// The concrete namespace5 (ordinary-member outbox) handler. `None` keeps this opener's
    /// grammar exactly as before: an ID5 entry or member-outbox fixed artifact refuses before
    /// any old cleaner. Trusted Rust configuration only, never decoded from storage.
    pub(in crate::store) member_outbox: Option<super::member_outbox_layout::V2MemberOutboxPolicy>,
}
impl std::fmt::Debug for V2CurrentLayoutPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2CurrentLayoutPolicy")
            .finish_non_exhaustive()
    }
}
impl V2CurrentLayoutPolicy {
    pub fn new(
        archive: ArchiveBootstrapCatalogPolicy,
        canonical: CanonicalFilenameInventoryPolicy,
        persistent_capacity: usize,
    ) -> Result<Self, AppError> {
        // Real historical DATA reading is this wrapper's first concrete consumer. Supplying
        // namespace grammar without its actual paid reader is not a supported configuration.
        let policy = Self {
            archive,
            canonical,
            persistent_capacity,
            member_outbox: None,
        };
        policy.require_persistent_capacity()?;
        Ok(policy)
    }

    /// Install the concrete namespace5 handler for ordinary-member outbox storage. Opening
    /// with it declares and mounts ID5 (independently of governance4) and admits ID5 in this
    /// opener's catalog grammar. Its separate member pool and quotas never share credit with
    /// the archive/JOIN pool; the declaration's own transient fee stays on that pool.
    pub fn with_member_outbox(
        mut self,
        member_outbox: super::member_outbox_layout::V2MemberOutboxPolicy,
    ) -> Result<Self, AppError> {
        member_outbox.require_capacity()?;
        self.member_outbox = Some(member_outbox);
        self.require_persistent_capacity()?;
        Ok(self)
    }
    /// Repeat this trusted negative policy check at actual opening/writing, not only while
    /// constructing a cloneable configuration. It is logical fixed-file capacity, not disk
    /// free space or a saved declaration/group/Current capability.
    pub(in crate::store) fn require_persistent_capacity(&self) -> Result<(), AppError> {
        use super::archive_current_namespace_codec as codec;
        if self.archive.source_read.is_none()
            || self.persistent_capacity
                < 3 * codec::MAX_CATALOG_FRAME
                    + 4136
                    + codec::MAX_MARKER_FRAME
                    + archive_bootstrap::MAX_MARKER_FRAMED
        {
            return Err(refused());
        }
        Ok(())
    }
}

// The fixed original summary is included in the declaration's metadata debit. Keep its
// promotion/repair state inline so phase changes do not allocate beneath the writer guards.
#[allow(clippy::large_enum_variant)]
pub(in crate::store) enum DeclarationParent {
    Original(super::pending_join_codec::Summary),
    /// Exact candidate already occupies Current. This is repair DATA, not a fabricated old
    /// Summary/authority; only actual current+marker barriers may settle this branch.
    AlreadyPromoted {
        original_frame: [u8; 32],
    },
}
impl DeclarationParent {
    pub(in crate::store) fn frame_data(&self) -> [u8; 32] {
        match self {
            Self::Original(summary) => summary.framed,
            Self::AlreadyPromoted { original_frame } => *original_frame,
        }
    }
}
pub(in crate::store) struct Declaration {
    pub(in crate::store) layout: super::archive_current_namespace_codec::CurrentLayoutData,
    pub(in crate::store) parent: DeclarationParent,
    pub(in crate::store) next: super::pending_join_codec::Summary,
    pub(in crate::store) candidate: Zeroizing<Vec<u8>>,
    pub(in crate::store) marker: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed: Zeroizing<Vec<u8>>,
    pub(in crate::store) seed_file: Option<super::pending_join::CreatedStage>,
    pub(in crate::store) stage_file: Option<super::pending_join::CreatedStage>,
    pub(in crate::store) marker_file: Option<super::pending_join::CreatedStage>,
}

/// Original fixed marker object; no Current/group writer is granted by this resource owner.
pub(in crate::store) struct MountedCurrentLayout {
    pub(in crate::store) layout: super::archive_current_namespace_codec::CurrentLayoutData,
    pub(in crate::store) marker: File,
    pub(in crate::store) marker_id: catcoms_storage::kept::SessionId,
    pub(in crate::store) marker_digest: [u8; 32],
    pub(in crate::store) mount: Arc<()>,
    pub(in crate::store) negative: RequestCancellation,
    pub(in crate::store) _fee: CatalogReadReservation,
}

// Only these boxes/path selectors are part of this NEW wrapper's small initial invoice. Old
// vault/KDF/authenticated admission-map allocations keep their existing contracts; the separate
// sorted key index, transient declaration and mounted marker have independent actual fees.
const WRAPPER_FEE: usize = 2 * std::mem::size_of::<PreparedV2CurrentLayoutStore>()
    + 2 * std::mem::size_of::<ServerStore>()
    + std::mem::size_of::<V2CurrentLayoutStore>()
    + std::mem::size_of::<Declaration>()
    + 2 * (32 * 1024 + 256);
// The declaration fee separately names twelve maximum buffers and 256KiB fixed metadata.
// Pin that residual to the ACTUAL ABI of simultaneous complete parsed/summary/header owners.
const _: () = assert!(
    std::mem::size_of::<Declaration>()
        + 3 * std::mem::size_of::<super::pending_join_codec::Parsed<'static>>()
        + 4 * std::mem::size_of::<super::pending_join_codec::Summary>()
        + std::mem::size_of::<MountedCurrentLayout>()
        + 2 * std::mem::size_of::<V2CurrentLayoutPolicy>()
        + 8 * std::mem::size_of::<std::fs::File>()
        + 4 * std::mem::size_of::<std::fs::Metadata>()
        + std::mem::size_of::<blake3::Hasher>()
        // Five engine selectors live together. Two nested Root selectors and a native
        // path-conversion scratch phase are additionally named rather than hidden in ABI
        // metadata. Their data never becomes a retained caller-supplied path collection.
        + 5 * (MAX_OPEN_BASE_PATH + 256)
        + 2 * (32 * 1024 + 256)
        + 32 * 1024
        + 4 * 1024
        <= 256 * 1024
);

/// Sole still-private original physical owner. ID2 and ID3 LARGE declaration fees are serial:
/// initial common opening pays its own reads/gates; only after that phase drops do we acquire
/// an ID2 retained-candidate fee (if needed), release it after completion, then acquire ID3's.
pub struct PreparedV2CurrentLayoutStore {
    archive: Option<PreparedArchiveBootstrapStore>,
    store: Option<Box<ServerStore>>,
    declaration: Option<Box<Declaration>>,
    declaration_fee: Option<CatalogReadReservation>,
    complete: bool,
    // LAST: Store/index/marker/candidate/files and every supporting phase fee drain first.
    _wrapper_fee: CatalogReadReservation,
}
impl std::fmt::Debug for PreparedV2CurrentLayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedV2CurrentLayoutStore")
            .finish_non_exhaustive()
    }
}

/// Completed physical namespace3 + its actual historical DATA reader. No Deref/raw Store,
/// server factory, group-write, Current scope, governor, SDK or native capability is public.
pub struct V2CurrentLayoutStore {
    store: Box<ServerStore>,
    _wrapper_fee: CatalogReadReservation,
}
impl std::fmt::Debug for V2CurrentLayoutStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2CurrentLayoutStore")
            .finish_non_exhaustive()
    }
}
impl PreparedV2CurrentLayoutStore {
    pub fn open(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        policy.require_persistent_capacity()?;
        if dir.as_ref().as_os_str().len() > MAX_OPEN_BASE_PATH || !ServerStore::exists(dir.as_ref())
        {
            return Err(refused());
        }
        let fee = policy.archive.pending.pool.try_reserve(WRAPPER_FEE)?;
        let store = Box::new(ServerStore::open_with_catalog_policy(
            dir,
            passphrase,
            rng,
            Some(policy.archive.pending.pool.clone()),
            Some(policy.archive.pending.clone()),
            Some(policy.archive.clone()),
            Some(policy),
        )?);
        let archive_ready = store
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?
            .is_some();
        let (archive, store) = if archive_ready {
            (None, Some(store))
        } else {
            let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
            let phase_fee = policy
                .archive
                .pending
                .pool
                .try_reserve(archive_bootstrap::DECLARATION_FEE)?;
            (
                Some(PreparedArchiveBootstrapStore::from_private_current_open(
                    store, phase_fee,
                )?),
                None,
            )
        };
        Ok(Self {
            archive,
            store,
            declaration: None,
            declaration_fee: None,
            complete: false,
            _wrapper_fee: fee,
        })
    }

    /// Retry identical retained outputs. No RNG or reseal occurs after a candidate exists.
    pub fn retry(&mut self, rng: &mut impl CryptoRngCore) -> Result<(), AppError> {
        if let Some(archive) = self.archive.as_mut() {
            archive.retry(rng)?;
            let archive = self.archive.take().ok_or_else(refused)?;
            match archive.into_store() {
                Ok(store) => self.store = Some(Box::new(store)),
                Err(archive) => {
                    self.archive = Some(archive);
                    return Err(refused());
                }
            }
            // The old owner, candidate bytes and ID2 phase fee have all drained before the
            // first ID3 reservation below. Simultaneous namespace-declaration debt is refused.
        }
        let store = self.store.as_ref().ok_or_else(refused)?;
        if store.chat_mount.observer().is_cancelled() {
            return Err(refused());
        }
        if store
            .current_layout
            .lock()
            .map_err(|_| refused())?
            .is_none()
        {
            if self.declaration_fee.is_none() {
                let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
                self.declaration_fee = Some(
                    policy
                        .archive
                        .pending
                        .pool
                        .try_reserve(super::archive_current_namespace_codec::DECLARATION_FEE)?,
                );
            }
            if self.declaration.is_none() {
                self.declaration = Some(Box::new(store.prepare_current_layout_declaration(
                    self.declaration_fee.as_ref().ok_or_else(refused)?,
                    rng,
                )?));
            }
            let declaration = self.declaration.as_mut().ok_or_else(refused)?;
            store.write_current_layout_declaration(
                declaration,
                self.declaration_fee.as_ref().ok_or_else(refused)?,
            )?;
            store.install_current_layout_marker(declaration.layout)?;
            {
                let mut catalog = store.pending_join_catalog.lock().map_err(|_| refused())?;
                let catalog = catalog.as_mut().ok_or_else(refused)?;
                if catalog.pending.is_some()
                    || (catalog.current.framed != declaration.parent.frame_data()
                        && catalog.current.framed != declaration.next.framed)
                {
                    return Err(refused());
                }
                catalog.current = declaration.next.clone();
            }
        }
        store.initialize_current_lineage_index()?;
        store.validate_mounted_current_layout()?;
        self.complete = true;
        Ok(())
    }

    /// Owning refusal retains SAME physical Store/candidates/fees; no raw Store escapes.
    pub fn into_current_layout_store(mut self) -> Result<V2CurrentLayoutStore, Self> {
        if !self.complete
            || self
                .store
                .as_ref()
                .is_none_or(|store| store.validate_mounted_current_layout().is_err())
        {
            return Err(self);
        }
        let Some(store) = self.store.take() else {
            return Err(self);
        };
        // Destructure rather than dropping a phase fee while its exact ciphertext survives.
        let Self {
            archive: _,
            store: _,
            declaration,
            declaration_fee,
            complete: _,
            _wrapper_fee,
        } = self;
        drop(declaration);
        drop(declaration_fee);
        Ok(V2CurrentLayoutStore {
            store,
            _wrapper_fee,
        })
    }
}
impl V2CurrentLayoutStore {
    /// Real historical SourceSaved DATA read. Checks the actual mandatory namespace marker on
    /// BOTH sides of the independent paid reader; decoded layout/source labels confer nothing.
    pub fn read_saved_bootstrap_source_data(
        &self,
        group: &[u8],
        signer: [u8; 32],
    ) -> Result<crate::store::RetainedBootstrapSourceData, AppError> {
        let store = self.checked_store()?;
        let source = store.read_saved_bootstrap_source_data(group, signer)?;
        store.validate_mounted_current_layout()?;
        Ok(source)
    }
    pub(in crate::store) fn checked_store(&self) -> Result<&ServerStore, AppError> {
        self.store.validate_mounted_current_layout()?;
        Ok(&self.store)
    }
}

#[cfg(test)]
mod tests;
