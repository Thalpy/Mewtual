//! Held native leaves across the catalog-only governance declaration. The
//! catalog can become ONLY its exact recorded candidate; mutable native leaves
//! and all original directory/marker identities remain fixed throughout.
use super::*;
use critical::governance_layout::codec::NativeTupleData;

/// Separate actual account, never spare credit from a member/SDK/old native
/// preparation. Covers held/reopened native frames, semantic/AEAD overlap,
/// marker/root reads and bounded directory/empty-admission scratch.
pub(in crate::store) const INVENTORY_FEE: usize = 12 * MAX_INDEX_FRAME
    + 12 * native::MAX_CATALOG_FRAME
    + 16 * native::MAX_MARKER_FRAME
    + 4 * EXPECTED_CURRENT_FEE
    + 16 * 1024 * 1024;

/// Physical custody only. Semantic native3 authentication is performed by the
/// closed lifecycle consumer before this inventory is used by a writer.
pub(in crate::store) struct NativeGovernanceInventory {
    expected: ExpectedCurrentInputs,
    base: operational::IndexSnapshot,
    member_marker: HeldNativeInput,
    member_parent: HeldDirectory,
    canonical: Option<HeldDirectory>,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    // Held file handles must keep the same exclusive physical session alive
    // even if a failed constructor drops its enclosing Store first.
    _session: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}
impl NativeGovernanceInventory {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        pool: &critical::read_pool::CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        let fee = pool.try_reserve(INVENTORY_FEE)?;
        let expected = ExpectedCurrentInputs::capture(store, &fee, pool)?;
        let base = operational::IndexSnapshot::capture_lifecycle_inventory(store)?;
        let canonical_path = path(store, false, "owner-state")?;
        let canonical = match fs::symlink_metadata(&canonical_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Ok(_) => Some(HeldDirectory::capture(&canonical_path)?),
            Err(e) => return Err(io(e)),
        };
        let value = Self {
            expected,
            base,
            member_marker: HeldNativeInput::capture(
                &path(
                    store,
                    true,
                    critical::member_outbox_layout::MEMBER_OUTBOX_MARKER,
                )?,
                native::MAX_MARKER_FRAME,
            )?
            .ok_or_else(denied)?,
            member_parent: HeldDirectory::capture(&path(
                store,
                true,
                critical::member_outbox_layout::MEMBER_OUTBOX_PARENT,
            )?)?,
            canonical,
            mount: store.capture_physical_mount_negative(),
            _session: store._session.retain_for_physical_work(),
            _fee: fee,
        };
        value.require_leaves(store)?;
        Ok(value)
    }
    pub(in crate::store) fn layout(
        &self,
        store: &ServerStore,
    ) -> Result<critical::archive_current_namespace_codec::CurrentLayoutData, AppError> {
        self.expected.authenticate(store)
    }
    pub(in crate::store) fn catalog(&self) -> &[u8] {
        self.base.catalog()
    }
    pub(in crate::store) fn index(&self) -> &[u8] {
        self.base.index()
    }
    pub(in crate::store) fn marker(&self) -> &[u8] {
        self.base.marker()
    }
    pub(in crate::store) fn member_marker(&self) -> &[u8] {
        self.member_marker.raw()
    }
    pub(in crate::store) fn require_captured_catalog(
        &self,
        store: &ServerStore,
        expected: &[u8; 32],
    ) -> Result<(), AppError> {
        self.require_leaves(store)?;
        if blake3::hash(self.base.catalog.raw()).as_bytes() != expected {
            return Err(denied());
        }
        self.base
            .catalog
            .require(&path(store, false, critical::Slot::Current.name())?)
    }
    /// No catalog substitution is admitted here: the original/candidate proof
    /// is a separate exact seed role owned by the governance writer.
    pub(in crate::store) fn require_leaves(&self, store: &ServerStore) -> Result<(), AppError> {
        self.expected.require(store)?;
        store.require_original_physical_mount(&self.mount)?;
        self.base.require_directories(store)?;
        operational::refuse_upgrade_debt(store)?;
        absent(&path(store, false, native::SEED_NAME)?)?;
        refuse_upgrade_debt(store)?;
        for name in [
            critical::ARCHIVE_DECLARATION_SEED,
            critical::current_group::DECLARATION_SEED,
            critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
        ] {
            absent(&path(store, false, name)?)?;
        }
        absent(&path(store, true, native::INDEX_STAGE_NAME)?)?;
        if self.base.stage.is_some() {
            return Err(denied());
        }
        self.base
            .index
            .require(&path(store, true, native::INDEX_NAME)?)?;
        self.base
            .marker
            .require(&path(store, true, native::MARKER_NAME)?)?;
        self.member_marker.require(&path(
            store,
            true,
            critical::member_outbox_layout::MEMBER_OUTBOX_MARKER,
        )?)?;
        let member = path(
            store,
            true,
            critical::member_outbox_layout::MEMBER_OUTBOX_PARENT,
        )?;
        self.member_parent.require(&member)?;
        require_empty_directory(&member)?;
        let canonical = path(store, false, "owner-state")?;
        match &self.canonical {
            Some(held) => {
                held.require(&canonical)?;
                require_empty_directory(&canonical)?;
            }
            None => absent(&canonical)?,
        }
        // Fixed small whitelist: no group namespace, native network dependency,
        // source artifact or unknown child can hide in a fresh-empty parent.
        let root = path(store, true, "")?;
        for (n, entry) in fs::read_dir(&root).map_err(io)?.enumerate() {
            if n >= 7 {
                return Err(denied());
            }
            let name = entry.map_err(io)?.file_name();
            let allowed = [
                critical::archive_bootstrap::ROOT_MARKER,
                critical::current_layout::CURRENT_MARKER,
                native::INDEX_NAME,
                native::MARKER_NAME,
                critical::member_outbox_layout::MEMBER_OUTBOX_MARKER,
                critical::member_outbox_layout::MEMBER_OUTBOX_PARENT,
                critical::governance_layout::GOVERNANCE_MARKER,
            ];
            if !allowed
                .iter()
                .any(|allowed| name == std::ffi::OsStr::new(allowed))
            {
                return Err(denied());
            }
        }
        store.require_empty_native_governance_admissions()?;
        self.base.require_directories(store)?;
        self.expected.require(store)
    }
    pub(in crate::store) fn require_tuple(
        &self,
        store: &ServerStore,
        data: NativeTupleData,
    ) -> Result<(), AppError> {
        self.require_leaves(store)?;
        if data.index_len != self.index().len() as u64
            || data.index != *blake3::hash(self.index()).as_bytes()
            || data.marker_len != self.marker().len() as u64
            || data.marker != *blake3::hash(self.marker()).as_bytes()
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(in crate::store) fn require_original_catalog(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.require_leaves(store)?;
        self.base
            .catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        absent(&path(store, false, critical::Slot::Stage.name())?)?;
        absent(&path(
            store,
            false,
            critical::governance_layout::GOVERNANCE_SEED,
        )?)?;
        absent(&path(
            store,
            true,
            critical::governance_layout::GOVERNANCE_MARKER,
        )?)
    }
}
fn require_empty_directory(path: &Path) -> Result<(), AppError> {
    if fs::read_dir(path)
        .map_err(io)?
        .next()
        .transpose()
        .map_err(io)?
        .is_some()
    {
        return Err(denied());
    }
    Ok(())
}
