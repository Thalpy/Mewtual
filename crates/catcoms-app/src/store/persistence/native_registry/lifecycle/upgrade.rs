//! Dedicated version2 -> version3 fixed writing roles. Root's owning consumer
//! and cold handler are still under construction; no public opener selects3.
//! Root must authenticate the complete relation against the independently held
//! Current expectation and retain the original gate claim before invoking IO.
//! No generic byte ceiling, pathname, semantic validator or SDK authority is
//! supplied by this module. Old version2 readers retain their original bounds.
#[cfg(test)]
use super::super::operational::RetirementObservation;
use super::super::operational::{
    complete_with_cut, observe_before_target_proof, trip, Fault, IndexSnapshot, RenameTarget,
};
use super::*;
mod first_torn;
pub(in crate::store) use first_torn::FirstTornIo;
#[cfg(test)]
pub(in crate::store) use first_torn::{
    before_seed_unlink, fence_covers, reset_fence_covers, take_seed_hook_seen,
};

#[cfg(test)]
thread_local! {
    static OUTPUT_ROOT_COVERS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(in crate::store) fn reset_output_root_covers() {
    OUTPUT_ROOT_COVERS.with(|count| count.set(0));
}
#[cfg(test)]
pub(in crate::store) fn output_root_covers() -> u64 {
    OUTPUT_ROOT_COVERS.with(std::cell::Cell::get)
}

fn require_no_old_upgrade(store: &ServerStore) -> Result<(), AppError> {
    for (root, name) in [
        (false, native::SEED_NAME),
        (false, super::super::operational::UPGRADE_SEED_NAME),
        (true, super::super::operational::UPGRADE_INDEX_NAME),
        (true, super::super::operational::UPGRADE_MARKER_NAME),
    ] {
        absent(&path(store, root, name)?)?;
    }
    Ok(())
}

pub(in crate::store) struct UpgradeBytes<'a> {
    pub(in crate::store) original: &'a [u8],
    pub(in crate::store) catalog: &'a [u8],
    pub(in crate::store) old_index: &'a [u8],
    pub(in crate::store) index: &'a [u8],
    pub(in crate::store) old_marker: &'a [u8],
    pub(in crate::store) marker: &'a [u8],
    pub(in crate::store) seed: &'a [u8],
}
impl UpgradeBytes<'_> {
    fn identities(&self) -> Result<[[u8; 32]; 7], AppError> {
        let mut hashes = [[0; 32]; 7];
        for ((bytes, limit), hash) in [
            (self.original, native::MAX_CATALOG_FRAME),
            (self.catalog, native::MAX_CATALOG_FRAME),
            (self.old_index, native::MAX_INDEX_FRAME),
            (self.index, MAX_INDEX_FRAME),
            (self.old_marker, native::MAX_MARKER_FRAME),
            (self.marker, native::MAX_MARKER_FRAME),
            (self.seed, MAX_UPGRADE_SEED_FRAME),
        ]
        .into_iter()
        .zip(&mut hashes)
        {
            if !(40..=limit).contains(&bytes.len()) {
                return Err(denied());
            }
            *hash = *blake3::hash(bytes).as_bytes();
        }
        Ok(hashes)
    }
}

/// Continuous original classification. Root must authenticate/validate the
/// complete seed relation before this inventory can issue any writing role.
pub(in crate::store) struct UpgradeInventory {
    // Binds a semantic cleanup proof to THIS held inventory, not merely equal
    // ciphertext observed through another mount/capture. Paid control metadata.
    cleanup_identity: std::sync::Arc<()>,
    expected_current: ExpectedCurrentInputs,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    seed: Option<HeldNativeInput>,
    catalog_stage: Option<HeldNativeInput>,
    index: Option<HeldNativeInput>,
    marker: Option<HeldNativeInput>,
    base: IndexSnapshot,
}
impl UpgradeInventory {
    pub(in crate::store) fn capture(
        store: &ServerStore,
        debit: &CatalogReadReservation,
        pool: &critical::read_pool::CriticalCatalogReadPool,
    ) -> Result<Self, AppError> {
        // The small fixed-marker invoice alone cannot fund this whole native
        // index/intent/output inventory. Refuse before even the first capture.
        debit.validate(pool, critical::native_registry::NATIVE_INDEX3_FULL_FEE)?;
        let value = Self {
            expected_current: ExpectedCurrentInputs::capture(store, debit, pool)?,
            cleanup_identity: std::sync::Arc::new(()),
            mount: store.capture_physical_mount_negative(),
            base: IndexSnapshot::capture_lifecycle_inventory(store)?,
            seed: HeldNativeInput::capture(
                &path(store, false, UPGRADE_SEED_NAME)?,
                MAX_UPGRADE_SEED_FRAME,
            )?,
            catalog_stage: HeldNativeInput::capture(
                &path(store, false, critical::Slot::Stage.name())?,
                native::MAX_CATALOG_FRAME,
            )?,
            index: HeldNativeInput::capture(
                &path(store, true, UPGRADE_INDEX_NAME)?,
                MAX_INDEX_FRAME,
            )?,
            marker: HeldNativeInput::capture(
                &path(store, true, UPGRADE_MARKER_NAME)?,
                native::MAX_MARKER_FRAME,
            )?,
        };
        value.require(store)?;
        Ok(value)
    }
    pub(in crate::store) fn first_torn_witness(&self) -> FirstTornInventoryWitness {
        FirstTornInventoryWitness {
            original: self.cleanup_identity.clone(),
        }
    }
    pub(in crate::store) fn matches_first_torn_witness(
        &self,
        witness: &FirstTornInventoryWitness,
    ) -> bool {
        std::sync::Arc::ptr_eq(&self.cleanup_identity, &witness.original)
    }
    /// DATA derived from the independently held real marker pair, never from
    /// the intent's catalogs. The same pair moves into the eventual IO owner.
    pub(in crate::store) fn authenticate_expected(
        &self,
        store: &ServerStore,
    ) -> Result<critical::archive_current_namespace_codec::CurrentLayoutData, AppError> {
        self.require(store)?;
        let layout = self.expected_current.authenticate(store)?;
        self.require(store)?;
        Ok(layout)
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
    pub(in crate::store) fn seed(&self) -> Option<&[u8]> {
        self.seed.as_ref().map(HeldNativeInput::raw)
    }
    pub(in crate::store) fn catalog_stage(&self) -> Option<&[u8]> {
        self.catalog_stage.as_ref().map(HeldNativeInput::raw)
    }
    pub(in crate::store) fn index_stage(&self) -> Option<&[u8]> {
        self.base.stage()
    }
    pub(in crate::store) fn has_upgrade_outputs(&self) -> bool {
        self.index.is_some() || self.marker.is_some()
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        self.expected_current.require(store)?;
        store.require_original_physical_mount(&self.mount)?;
        require_no_old_upgrade(store)?;
        self.base.require_directories(store)?;
        for name in [
            critical::ARCHIVE_DECLARATION_SEED,
            critical::current_group::DECLARATION_SEED,
            critical::governance_layout::GOVERNANCE_SEED,
            critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
        ] {
            absent(&path(store, false, name)?)?;
        }
        absent(&path(store, false, native::SEED_NAME)?)?;
        for (root, name, observed) in [
            (false, UPGRADE_SEED_NAME, self.seed.as_ref()),
            (
                false,
                critical::Slot::Stage.name(),
                self.catalog_stage.as_ref(),
            ),
            (true, UPGRADE_INDEX_NAME, self.index.as_ref()),
            (true, UPGRADE_MARKER_NAME, self.marker.as_ref()),
            (
                false,
                critical::Slot::Current.name(),
                Some(&self.base.catalog),
            ),
            (true, native::INDEX_NAME, Some(&self.base.index)),
            (true, native::MARKER_NAME, Some(&self.base.marker)),
            (true, native::INDEX_STAGE_NAME, self.base.stage.as_ref()),
        ] {
            let name = path(store, root, name)?;
            match observed {
                Some(file) => file.require(&name)?,
                None => absent(&name)?,
            }
        }
        self.base.require_directories(store)?;
        store.require_original_physical_mount(&self.mount)?;
        self.expected_current.require(store)
    }
}

/// Inert, non-clone original-inventory identity. Only the semantic verifier's
/// closed proof can turn it into cleanup; it grants no writer or Store access.
pub(in crate::store) struct FirstTornInventoryWitness {
    original: std::sync::Arc<()>,
}

/// Mutable roles always retain the very same objects classified above. A cold
/// second pathname open of equal bytes cannot adopt a substituted file.
fn upgrade_writable(
    store: &ServerStore,
    root: bool,
    name: &str,
    observed: Option<&HeldNativeInput>,
    expected: &[u8],
    exact_only: bool,
) -> Result<Option<critical::pending_join::CreatedStage>, AppError> {
    let slot = capture_writable(&path(store, root, name)?, expected, exact_only)?;
    if slot.as_ref().and_then(|file| file.identity) != observed.map(HeldNativeInput::identity) {
        return Err(denied());
    }
    Ok(slot)
}

fn upgrade_stage_prefix(
    name: &Path,
    slot: &critical::pending_join::CreatedStage,
    expected: &[u8],
    empty_only: bool,
) -> Result<(), AppError> {
    held(name, slot)?;
    let observed = HeldNativeInput::capture(name, expected.len())?.ok_or_else(denied)?;
    if Some(observed.identity) != slot.identity
        || (empty_only && !observed.raw().is_empty())
        || observed.raw() != &expected[..observed.raw().len()]
    {
        return Err(denied());
    }
    held(name, slot)
}

pub(in crate::store) struct UpgradeIo {
    expected_current: ExpectedCurrentInputs,
    mount: crate::store::chat_admission::PhysicalMountNegative,
    seed: Option<critical::pending_join::CreatedStage>,
    catalog: Option<critical::pending_join::CreatedStage>,
    index: Option<critical::pending_join::CreatedStage>,
    marker: Option<critical::pending_join::CreatedStage>,
    original: IndexSnapshot,
    expected: [[u8; 32]; 7],
    index_promoted: bool,
    marker_promoted: bool,
    catalog_promoted: bool,
    seed_removed: bool,
    // MUST remain last: observes retirement after every actual File/input buffer.
    #[cfg(test)]
    retirement: Option<RetirementObservation>,
}
impl UpgradeIo {
    #[cfg(test)]
    pub(in crate::store) fn observe_retirement(&mut self, check: Box<dyn FnOnce() + Send>) {
        self.retirement = Some(RetirementObservation::new(check));
    }
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        inventory: UpgradeInventory,
        bytes: &UpgradeBytes<'_>,
    ) -> Result<Self, AppError> {
        inventory.require(store)?;
        if inventory.seed.is_some()
            || inventory.catalog_stage.is_some()
            || inventory.index.is_some()
            || inventory.marker.is_some()
            || inventory.base.stage.is_some()
            || inventory.catalog() != bytes.original
            || inventory.index() != bytes.old_index
            || inventory.marker() != bytes.old_marker
        {
            return Err(denied());
        }
        Ok(Self {
            expected_current: inventory.expected_current,
            mount: inventory.mount,
            original: inventory.base,
            seed: None,
            catalog: None,
            index: None,
            marker: None,
            expected: bytes.identities()?,
            index_promoted: false,
            marker_promoted: false,
            catalog_promoted: false,
            seed_removed: false,
            #[cfg(test)]
            retirement: None,
        })
    }

    pub(in crate::store) fn recover(
        store: &ServerStore,
        inventory: UpgradeInventory,
        bytes: &UpgradeBytes<'_>,
    ) -> Result<Self, AppError> {
        inventory.require(store)?;
        let catalog_promoted = inventory.catalog() == bytes.catalog;
        let index_promoted = inventory.index() == bytes.index;
        let marker_promoted = inventory.marker() == bytes.marker;
        if (!catalog_promoted && inventory.catalog() != bytes.original)
            || (!index_promoted && inventory.index() != bytes.old_index)
            || (!marker_promoted && inventory.marker() != bytes.old_marker)
            || inventory.base.stage.is_some()
            || inventory.seed() != Some(bytes.seed)
            || (catalog_promoted
                && (!index_promoted || !marker_promoted || inventory.catalog_stage.is_some()))
            || (!(index_promoted && marker_promoted)
                && inventory
                    .catalog_stage
                    .as_ref()
                    .is_some_and(|stage| !stage.raw().is_empty()))
            || (index_promoted && inventory.index.is_some())
            || (marker_promoted && inventory.marker.is_some())
        {
            return Err(denied());
        }
        // Classification above finishes BEFORE acquiring any writable roles.
        let seed = upgrade_writable(
            store,
            false,
            UPGRADE_SEED_NAME,
            inventory.seed.as_ref(),
            bytes.seed,
            true,
        )?;
        let catalog = if catalog_promoted {
            upgrade_writable(
                store,
                false,
                critical::Slot::Current.name(),
                Some(&inventory.base.catalog),
                bytes.catalog,
                true,
            )?
        } else {
            upgrade_writable(
                store,
                false,
                critical::Slot::Stage.name(),
                inventory.catalog_stage.as_ref(),
                bytes.catalog,
                false,
            )?
        };
        let index = if index_promoted {
            upgrade_writable(
                store,
                true,
                native::INDEX_NAME,
                Some(&inventory.base.index),
                bytes.index,
                true,
            )?
        } else {
            upgrade_writable(
                store,
                true,
                UPGRADE_INDEX_NAME,
                inventory.index.as_ref(),
                bytes.index,
                false,
            )?
        };
        let marker = if marker_promoted {
            upgrade_writable(
                store,
                true,
                native::MARKER_NAME,
                Some(&inventory.base.marker),
                bytes.marker,
                true,
            )?
        } else {
            upgrade_writable(
                store,
                true,
                UPGRADE_MARKER_NAME,
                inventory.marker.as_ref(),
                bytes.marker,
                false,
            )?
        };
        inventory.require(store)?;
        Ok(Self {
            expected_current: inventory.expected_current,
            mount: inventory.mount,
            original: inventory.base,
            seed,
            catalog,
            index,
            marker,
            expected: bytes.identities()?,
            index_promoted,
            marker_promoted,
            catalog_promoted,
            seed_removed: false,
            #[cfg(test)]
            retirement: None,
        })
    }

    fn require_selected_parts(
        &self,
        store: &ServerStore,
        bytes: &UpgradeBytes<'_>,
    ) -> Result<(), AppError> {
        self.expected_current.require(store)?;
        store.require_original_physical_mount(&self.mount)?;
        self.original.require_directories(store)?;
        require_no_other_declaration(store)?;
        require_no_old_upgrade(store)?;
        absent(&path(store, false, native::SEED_NAME)?)?;
        let stage = path(store, false, critical::Slot::Stage.name())?;
        if self.catalog_promoted {
            absent(&stage)?;
        } else if let Some(slot) = &self.catalog {
            upgrade_stage_prefix(
                &stage,
                slot,
                bytes.catalog,
                !(self.index_promoted && self.marker_promoted),
            )?;
        } else {
            absent(&stage)?;
        }
        for (promoted, root, name, old, slot, expected) in [
            (
                self.catalog_promoted,
                false,
                critical::Slot::Current.name(),
                &self.original.catalog,
                &self.catalog,
                bytes.catalog,
            ),
            (
                self.index_promoted,
                true,
                native::INDEX_NAME,
                &self.original.index,
                &self.index,
                bytes.index,
            ),
            (
                self.marker_promoted,
                true,
                native::MARKER_NAME,
                &self.original.marker,
                &self.marker,
                bytes.marker,
            ),
        ] {
            let name = path(store, root, name)?;
            if promoted {
                exact(&name, slot.as_ref().ok_or_else(denied)?, expected)?;
            } else {
                old.require(&name)?;
            }
        }
        self.original.require_directories(store)?;
        store.require_original_physical_mount(&self.mount)?;
        self.expected_current.require(store)
    }

    /// Re-cover both already-renamed output objects on EVERY pre-selection
    /// retry. Promotion flags record effects, not completed directory barriers;
    /// an earlier marker-rename error may have skipped its original root sync.
    fn cover_selected_outputs_before_catalog(
        &self,
        store: &ServerStore,
        bytes: &UpgradeBytes<'_>,
    ) -> Result<(), AppError> {
        self.require_selected_parts(store, bytes)?;
        for (name, slot, expected) in [
            (native::INDEX_NAME, &self.index, bytes.index),
            (native::MARKER_NAME, &self.marker, bytes.marker),
        ] {
            let output = slot.as_ref().ok_or_else(denied)?;
            exact(&path(store, true, name)?, output, expected)?;
            output.file.sync_all().map_err(io)?;
        }
        self.original.root.file.sync_all().map_err(io)?;
        exact(
            &path(store, false, UPGRADE_SEED_NAME)?,
            self.seed.as_ref().ok_or_else(denied)?,
            bytes.seed,
        )?;
        self.require_selected_parts(store, bytes)?;
        #[cfg(test)]
        OUTPUT_ROOT_COVERS.with(|count| count.set(count.get() + 1));
        Ok(())
    }

    pub(in crate::store) fn advance_upgrade(
        &mut self,
        store: &ServerStore,
        bytes: &UpgradeBytes<'_>,
    ) -> Result<(), AppError> {
        if bytes.identities()? != self.expected {
            return Err(denied());
        }
        self.require_selected_parts(store, bytes)?;
        let seed = path(store, false, UPGRADE_SEED_NAME)?;
        let stage = path(store, false, critical::Slot::Stage.name())?;
        let current = path(store, false, critical::Slot::Current.name())?;
        let index_temp = path(store, true, UPGRADE_INDEX_NAME)?;
        let marker_temp = path(store, true, UPGRADE_MARKER_NAME)?;
        let index = path(store, true, native::INDEX_NAME)?;
        let marker = path(store, true, native::MARKER_NAME)?;
        if !self.catalog_promoted {
            claim(&seed, &mut self.seed)?;
            self.original.vault.file.sync_all().map_err(io)?;
            trip(Fault::UpgradeClaim)?;
            // The predecessor must encounter an EMPTY old-catalog fence before
            // any content-bearing v3 seed or output can become visible.
            claim(&stage, &mut self.catalog)?;
            self.original.vault.file.sync_all().map_err(io)?;
            self.require_selected_parts(store, bytes)?;
            trip(Fault::UpgradeFence)?;
            complete_with_cut(&seed, &mut self.seed, bytes.seed, Fault::UpgradePartialSeed)?;
            self.original.vault.file.sync_all().map_err(io)?;
            trip(Fault::UpgradeSeed)?;
            if !self.index_promoted {
                complete_with_cut(
                    &index_temp,
                    &mut self.index,
                    bytes.index,
                    Fault::UpgradePartialIndex,
                )?;
                self.original.root.file.sync_all().map_err(io)?;
                trip(Fault::UpgradeIndexTemp)?;
                self.require_selected_parts(store, bytes)?;
                exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
                exact(
                    &index_temp,
                    self.index.as_ref().ok_or_else(denied)?,
                    bytes.index,
                )?;
                observe_before_target_proof(RenameTarget::UpgradeIndex);
                self.original.index.require(&index)?;
                held(&index_temp, self.index.as_ref().ok_or_else(denied)?)?;
                self.original.require_directories(store)?;
                self.expected_current.require(store)?;
                fs::rename(&index_temp, &index).map_err(io)?;
                self.index_promoted = true;
                trip(Fault::UpgradeIndexRename)?;
                self.original.root.file.sync_all().map_err(io)?;
            }
            if !self.marker_promoted {
                complete_with_cut(
                    &marker_temp,
                    &mut self.marker,
                    bytes.marker,
                    Fault::UpgradePartialMarker,
                )?;
                self.original.root.file.sync_all().map_err(io)?;
                trip(Fault::UpgradeMarkerTemp)?;
                self.require_selected_parts(store, bytes)?;
                exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
                exact(
                    &marker_temp,
                    self.marker.as_ref().ok_or_else(denied)?,
                    bytes.marker,
                )?;
                observe_before_target_proof(RenameTarget::UpgradeMarker);
                self.original.marker.require(&marker)?;
                held(&marker_temp, self.marker.as_ref().ok_or_else(denied)?)?;
                self.original.require_directories(store)?;
                self.expected_current.require(store)?;
                fs::rename(&marker_temp, &marker).map_err(io)?;
                self.marker_promoted = true;
                trip(Fault::UpgradeMarkerRename)?;
                self.original.root.file.sync_all().map_err(io)?;
            }
            self.cover_selected_outputs_before_catalog(store, bytes)?;
            complete_with_cut(
                &stage,
                &mut self.catalog,
                bytes.catalog,
                Fault::UpgradePartialCatalog,
            )?;
            self.original.vault.file.sync_all().map_err(io)?;
            self.require_selected_parts(store, bytes)?;
            exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
            exact(
                &stage,
                self.catalog.as_ref().ok_or_else(denied)?,
                bytes.catalog,
            )?;
            self.original.require_directories(store)?;
            observe_before_target_proof(RenameTarget::UpgradeCatalog);
            self.original.catalog.require(&current)?;
            held(&stage, self.catalog.as_ref().ok_or_else(denied)?)?;
            self.original.require_directories(store)?;
            self.expected_current.require(store)?;
            fs::rename(&stage, &current).map_err(io)?;
            self.catalog_promoted = true; // Cache selection before all fallible barriers.
            trip(Fault::UpgradeCatalogRename)?;
        }
        // No output is recreated after catalog promotion. The same held File
        // handles cover every selected frame before intent can be removed.
        for (name, slot, expected) in [
            (&current, &self.catalog, bytes.catalog),
            (&index, &self.index, bytes.index),
            (&marker, &self.marker, bytes.marker),
        ] {
            let selected = slot.as_ref().ok_or_else(denied)?;
            exact(name, selected, expected)?;
            selected.file.sync_all().map_err(io)?;
        }
        self.original.root.file.sync_all().map_err(io)?;
        self.original.vault.file.sync_all().map_err(io)?;
        absent(&stage)?;
        absent(&index_temp)?;
        absent(&marker_temp)?;
        self.require_selected_parts(store, bytes)?;
        trip(Fault::UpgradeCovered)?;
        if !self.seed_removed {
            exact(&seed, self.seed.as_ref().ok_or_else(denied)?, bytes.seed)?;
            // Re-prove the selected tuple after the large seed read, immediately
            // before cleanup. A seed read is not a replacement for that late proof.
            self.require_selected_parts(store, bytes)?;
            fs::remove_file(&seed).map_err(io)?;
            self.seed_removed = true;
            drop(self.seed.take());
            trip(Fault::UpgradeSeedUnlink)?;
        }
        absent(&seed)?;
        self.original.vault.file.sync_all().map_err(io)?;
        self.require_selected_parts(store, bytes)?;
        Ok(())
    }
}
