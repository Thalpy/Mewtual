//! The fixed native file roles that stay unchanged across a founder-only
//! catalog declaration. The parent must independently authenticate and hold
//! the exact governance claim, Catalog Stage, joined pins and ID8 owner.
use super::*;

pub(in crate::store) const REVIEW_INVENTORY_FEE: usize = 3 * native::MAX_CATALOG_FRAME
    + 4 * MAX_INDEX_FRAME
    + 4 * native::MAX_MARKER_FRAME
    + 2 * EXPECTED_CURRENT_FEE
    + 128 * 1024;

pub(in crate::store) struct ReviewDeclarationInventory {
    expected: ExpectedCurrentInputs,
    base: operational::IndexSnapshot,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    _physical: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl ReviewDeclarationInventory {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        pool: &critical::read_pool::CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        let fee =
            pool.try_reserve_classified(REVIEW_INVENTORY_FEE, || AppError::ChatV2RegistryBusy)?;
        let value = Self {
            expected: ExpectedCurrentInputs::capture(store, &fee, pool)?,
            base: operational::IndexSnapshot::capture_lifecycle_inventory(store)?,
            mount: store.capture_physical_mount_negative(),
            _physical: store._session.retain_for_physical_work(),
            _fee: fee,
        };
        value.require_original(store)?;
        Ok(value)
    }

    pub(in crate::store) fn layout(
        &self,
        store: &ServerStore,
    ) -> Result<critical::archive_current_namespace_codec::CurrentLayoutData, AppError> {
        self.require_original(store)?;
        self.expected.authenticate(store)
    }

    pub(in crate::store) fn catalog(&self) -> (&[u8], SessionId) {
        (self.base.catalog(), self.base.catalog.identity())
    }

    pub(in crate::store) fn index_and_marker(&self) -> (&[u8], &[u8]) {
        (self.base.index(), self.base.marker())
    }

    pub(in crate::store) fn require_original(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_preserved(store)?;
        self.base
            .catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        self.require_preserved(store)
    }

    /// Used after the catalog writer has proved that its actual Stage inode
    /// is Current. The catalog and governance claim have their own roles.
    pub(in crate::store) fn require_preserved(&self, store: &ServerStore) -> Result<(), AppError> {
        self.expected.require(store)?;
        store.require_original_physical_mount(&self.mount)?;
        self.base.require_directories(store)?;
        operational::refuse_upgrade_debt(store)?;
        refuse_upgrade_debt(store)?;
        if self.base.stage.is_some() || store.has_rejoin_evidence()? {
            return Err(denied());
        }
        for name in [
            critical::ARCHIVE_DECLARATION_SEED,
            critical::current_group::DECLARATION_SEED,
            critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
            native::SEED_NAME,
        ] {
            absent(&path(store, false, name)?)?;
        }
        absent(&path(store, true, native::INDEX_STAGE_NAME)?)?;
        self.base
            .index
            .require(&path(store, true, native::INDEX_NAME)?)?;
        self.base
            .marker
            .require(&path(store, true, native::MARKER_NAME)?)?;
        self.base.require_directories(store)?;
        self.expected.require(store)
    }
}
