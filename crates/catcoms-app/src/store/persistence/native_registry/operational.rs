//! Fixed-name index2 IO. The Root owner authenticates selected/candidate DATA,
//! must retain one mount-wide transaction cell and its full prepaid overlap,
//! and supply the actual original ServerStore. That warm producer is unfinished;
//! current production uses the private cold roles. No renderer path reaches IO.
use super::*;

pub(in crate::store) const UPGRADE_SEED_NAME: &str = "chat-critical-namespaces.native-index2.seed";
pub(in crate::store) const UPGRADE_INDEX_NAME: &str = "native-index2.upgrade-index";
pub(in crate::store) const UPGRADE_MARKER_NAME: &str = "native-index2.upgrade-marker";
pub(in crate::store) const MAX_UPGRADE_SEED_FRAME: usize = 2 * native::MAX_CATALOG_FRAME
    + 2 * native::MAX_INDEX_FRAME
    + 2 * native::MAX_MARKER_FRAME
    + 4096;

/// Test-only observation after an owning structure's real files/buffers have
/// dropped. No runtime callback or publication authority is introduced.
#[cfg(test)]
pub(in crate::store) struct RetirementObservation(Option<Box<dyn FnOnce() + Send>>);
#[cfg(test)]
impl RetirementObservation {
    pub(in crate::store) fn new(check: Box<dyn FnOnce() + Send>) -> Self {
        Self(Some(check))
    }
}
#[cfg(test)]
impl Drop for RetirementObservation {
    fn drop(&mut self) {
        if let Some(check) = self.0.take() {
            check();
        }
    }
}

/// Closed deterministic cuts at the actual physical boundaries. Production
/// has no arming API; these calls cannot add a runtime fault/permission switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum Fault {
    UpgradeClaim,
    UpgradeFence,
    UpgradePartialSeed,
    UpgradeSeed,
    UpgradePartialIndex,
    UpgradeIndexTemp,
    UpgradeIndexRename,
    UpgradePartialMarker,
    UpgradeMarkerTemp,
    UpgradeMarkerRename,
    UpgradePartialCatalog,
    UpgradeCatalogRename,
    UpgradeCovered,
    UpgradeSeedUnlink,
    IndexStage,
    IndexRename,
    IndexStageUnlink,
}
#[cfg(test)]
thread_local! {
    static FAULT: std::cell::Cell<Option<Fault>> = const { std::cell::Cell::new(None) };
    static FAULT_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(in crate::store) fn arm_fault(point: Fault) {
    FAULT.with(|fault| {
        assert!(fault.get().is_none());
        fault.set(Some(point));
    });
    FAULT_SEEN.with(|seen| seen.set(false));
}
#[cfg(test)]
pub(in crate::store) fn take_fault_seen() -> bool {
    FAULT.with(|fault| fault.set(None));
    FAULT_SEEN.with(|seen| seen.replace(false))
}
fn fault_requested(point: Fault) -> bool {
    #[cfg(test)]
    return FAULT.with(|fault| {
        if fault.get() == Some(point) {
            fault.set(None);
            FAULT_SEEN.with(|seen| seen.set(true));
            true
        } else {
            false
        }
    });
    #[cfg(not(test))]
    {
        let _ = point;
        false
    }
}
fn interrupted(point: Fault) -> AppError {
    AppError::Io(format!("index storage interrupted at {point:?}"))
}
fn trip(point: Fault) -> Result<(), AppError> {
    if fault_requested(point) {
        Err(interrupted(point))
    } else {
        Ok(())
    }
}
fn complete_with_cut(
    name: &Path,
    slot: &mut Option<critical::pending_join::CreatedStage>,
    expected: &[u8],
    point: Fault,
) -> Result<(), AppError> {
    if fault_requested(point) {
        // Write a REAL partial prefix to the SAME owned file, using the actual
        // file barrier. Retry/cold inspection must handle those exact bytes.
        complete(name, slot, &expected[..expected.len() / 2], 0)?;
        return Err(interrupted(point));
    }
    complete(name, slot, expected, 0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) enum RenameTarget {
    Index,
    UpgradeIndex,
    UpgradeMarker,
    UpgradeCatalog,
}
#[cfg(test)]
type RenameHook = (RenameTarget, Box<dyn FnOnce()>);
#[cfg(test)]
thread_local! {
    static BEFORE_TARGET_PROOF: std::cell::RefCell<Option<RenameHook>> = const { std::cell::RefCell::new(None) };
    static RENAME_HOOK_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(in crate::store) fn before_target_proof(target: RenameTarget, hook: Box<dyn FnOnce()>) {
    BEFORE_TARGET_PROOF.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none());
        *slot = Some((target, hook));
    });
    RENAME_HOOK_SEEN.with(|seen| seen.set(false));
}
#[cfg(test)]
pub(in crate::store) fn take_rename_hook_seen() -> bool {
    BEFORE_TARGET_PROOF.with(|slot| {
        slot.borrow_mut().take();
    });
    RENAME_HOOK_SEEN.with(|seen| seen.replace(false))
}
fn observe_before_target_proof(target: RenameTarget) {
    #[cfg(test)]
    {
        let hook = BEFORE_TARGET_PROOF.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|(expected, _)| *expected == target)
            {
                slot.take().map(|(_, hook)| hook)
            } else {
                None
            }
        });
        // Never invoke a test callback while holding its hook registry guard.
        if let Some(hook) = hook {
            RENAME_HOOK_SEEN.with(|seen| seen.set(true));
            hook();
        }
    }
    #[cfg(not(test))]
    let _ = target;
}

pub(in crate::store) fn refuse_upgrade_debt(store: &ServerStore) -> Result<(), AppError> {
    absent(&path(store, false, UPGRADE_SEED_NAME)?)?;
    absent(&path(store, true, UPGRADE_INDEX_NAME)?)?;
    absent(&path(store, true, UPGRADE_MARKER_NAME)?)
}

fn no_upgrade_debt(store: &ServerStore) -> Result<(), AppError> {
    for (root, name) in [
        (false, native::SEED_NAME),
        (false, UPGRADE_SEED_NAME),
        (true, UPGRADE_INDEX_NAME),
        (true, UPGRADE_MARKER_NAME),
    ] {
        absent(&path(store, root, name)?)?;
    }
    Ok(())
}

/// Continuous physical custody, including the entire selected catalog. Even
/// an equal-byte replacement cannot become this operation's selected parent.
pub(in crate::store) struct IndexSnapshot {
    stage: Option<HeldNativeInput>,
    index: HeldNativeInput,
    marker: HeldNativeInput,
    catalog: HeldNativeInput,
    root: HeldDirectory,
    vault: HeldDirectory,
}
impl IndexSnapshot {
    pub(in crate::store) fn capture(store: &ServerStore) -> Result<Self, AppError> {
        let value = Self::capture_inventory(store)?;
        value.require_selected(store)?;
        Ok(value)
    }
    fn capture_inventory(store: &ServerStore) -> Result<Self, AppError> {
        let value = Self {
            vault: HeldDirectory::capture(&path(store, false, "")?)?,
            root: HeldDirectory::capture(&path(store, true, "")?)?,
            catalog: HeldNativeInput::capture(
                &path(store, false, critical::Slot::Current.name())?,
                native::MAX_CATALOG_FRAME,
            )?
            .ok_or_else(denied)?,
            marker: HeldNativeInput::capture(
                &path(store, true, native::MARKER_NAME)?,
                native::MAX_MARKER_FRAME,
            )?
            .ok_or_else(denied)?,
            index: HeldNativeInput::capture(
                &path(store, true, native::INDEX_NAME)?,
                native::MAX_INDEX_FRAME,
            )?
            .ok_or_else(denied)?,
            stage: HeldNativeInput::capture(
                &path(store, true, native::INDEX_STAGE_NAME)?,
                native::MAX_INDEX_FRAME,
            )?,
        };
        value.require_directories(store)?;
        Ok(value)
    }
    pub(in crate::store) fn catalog(&self) -> &[u8] {
        self.catalog.raw()
    }
    pub(in crate::store) fn marker(&self) -> &[u8] {
        self.marker.raw()
    }
    pub(in crate::store) fn index(&self) -> &[u8] {
        self.index.raw()
    }
    pub(in crate::store) fn stage(&self) -> Option<&[u8]> {
        self.stage.as_ref().map(HeldNativeInput::raw)
    }

    fn require_other(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_directories(store)?;
        no_upgrade_debt(store)?;
        for name in [
            critical::ARCHIVE_DECLARATION_SEED,
            critical::current_group::DECLARATION_SEED,
            critical::governance_layout::GOVERNANCE_SEED,
            critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
        ] {
            absent(&path(store, false, name)?)?;
        }
        // Index writes and JOIN catalog writes share the Store's catalog mutex
        // in the owning consumer. No catalog Stage is admitted concurrently.
        absent(&path(store, false, critical::Slot::Stage.name())?)?;
        self.catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        self.marker
            .require(&path(store, true, native::MARKER_NAME)?)?;
        self.require_directories(store)
    }
    fn require_directories(&self, store: &ServerStore) -> Result<(), AppError> {
        self.vault.require(&path(store, false, "")?)?;
        self.root.require(&path(store, true, "")?)?;
        if store.chat_mount.observer().is_cancelled() {
            return Err(denied());
        }
        Ok(())
    }
    pub(in crate::store) fn require_selected(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_other(store)?;
        self.index
            .require(&path(store, true, native::INDEX_NAME)?)?;
        match &self.stage {
            Some(stage) => stage.require(&path(store, true, native::INDEX_STAGE_NAME)?)?,
            None => absent(&path(store, true, native::INDEX_STAGE_NAME)?)?,
        }
        self.require_other(store)
    }
}

/// Construct before any Stage effect. The future warm caller MUST retain this
/// same object in its uncertain mount cell across caller loss and refusal. Drop
/// loses warm retry custody; a persisted WAL still requires exact cold recovery.
pub(in crate::store) struct IndexCommitIo {
    stage: Option<critical::pending_join::CreatedStage>,
    original: IndexSnapshot,
    promoted: bool,
    candidate: [u8; 32],
}
impl IndexCommitIo {
    pub(in crate::store) fn prepare(
        original: IndexSnapshot,
        expected: &[u8],
    ) -> Result<Self, AppError> {
        if original.stage.is_some() || !(40..=native::MAX_INDEX_FRAME).contains(&expected.len()) {
            return Err(denied());
        }
        Ok(Self {
            original,
            stage: None,
            promoted: false,
            candidate: *blake3::hash(expected).as_bytes(),
        })
    }
    /// A complete authenticated, semantically checked cold Stage may finish the
    /// original replacement, never create a fresh SDK or signing admission.
    pub(in crate::store) fn recover(
        store: &ServerStore,
        original: IndexSnapshot,
        expected: &[u8],
    ) -> Result<Self, AppError> {
        original.require_selected(store)?;
        let old = original.stage.as_ref().ok_or_else(denied)?;
        if old.raw() != expected {
            return Err(denied());
        }
        let stage = capture_writable(
            &path(store, true, native::INDEX_STAGE_NAME)?,
            expected,
            true,
        )?;
        if stage.as_ref().and_then(|stage| stage.identity) != Some(old.identity) {
            return Err(denied());
        }
        original.require_selected(store)?;
        Ok(Self {
            original,
            stage,
            promoted: false,
            candidate: *blake3::hash(expected).as_bytes(),
        })
    }
    pub(in crate::store) fn commit_index(
        &mut self,
        store: &ServerStore,
        expected: &[u8],
    ) -> Result<(), AppError> {
        if !(40..=native::MAX_INDEX_FRAME).contains(&expected.len())
            || *blake3::hash(expected).as_bytes() != self.candidate
        {
            return Err(denied());
        }
        let stage_path = path(store, true, native::INDEX_STAGE_NAME)?;
        let index_path = path(store, true, native::INDEX_NAME)?;
        self.original.require_other(store)?;
        if !self.promoted {
            self.original.index.require(&index_path)?;
            if self.stage.is_none() {
                absent(&stage_path)?;
            }
            complete(&stage_path, &mut self.stage, expected, 0)?;
            trip(Fault::IndexStage)?;
            self.original.root.file.sync_all().map_err(io)?;
            self.original.require_other(store)?;
            self.original.index.require(&index_path)?;
            exact(
                &stage_path,
                self.stage.as_ref().ok_or_else(denied)?,
                expected,
            )?;
            self.original.require_other(store)?;
            // Other tuple/Stage reads may be large. Re-prove the destination
            // AFTER them; an intervening foreign target must not be overwritten.
            observe_before_target_proof(RenameTarget::Index);
            self.original.index.require(&index_path)?;
            held(&stage_path, self.stage.as_ref().ok_or_else(denied)?)?;
            self.original.require_directories(store)?;
            fs::rename(&stage_path, &index_path).map_err(io)?;
            self.promoted = true; // Selection is cached BEFORE the covering barrier.
            trip(Fault::IndexRename)?;
        }
        let selected = self.stage.as_ref().ok_or_else(denied)?;
        exact(&index_path, selected, expected)?;
        selected.file.sync_all().map_err(io)?;
        self.original.root.file.sync_all().map_err(io)?;
        self.original.require_other(store)?;
        exact(&index_path, selected, expected)?;
        absent(&stage_path)?;
        Ok(())
    }
}

/// Negative-only cleanup after Root authenticated the selected index/marker,
/// for either unauthenticated scratch or an EXACT duplicate of the selected
/// frame. Authenticated empty, malformed, foreign or future DATA is preserved.
pub(in crate::store) struct IndexStageCleanup {
    original: IndexSnapshot,
    removed: bool,
}
impl IndexStageCleanup {
    pub(in crate::store) fn prepare_unauthenticated(
        original: IndexSnapshot,
    ) -> Result<Self, AppError> {
        if original.stage.is_none() {
            return Err(denied());
        }
        Ok(Self {
            original,
            removed: false,
        })
    }
    pub(in crate::store) fn prepare_duplicate(original: IndexSnapshot) -> Result<Self, AppError> {
        if original.stage() != Some(original.index()) {
            return Err(denied());
        }
        Ok(Self {
            original,
            removed: false,
        })
    }
    /// The owner retains this SAME phase/interlock on error, including after an
    /// unlink whose directory sync failed. Caller loss cannot publish idle first.
    pub(in crate::store) fn run(&mut self, store: &ServerStore) -> Result<(), AppError> {
        let stage_path = path(store, true, native::INDEX_STAGE_NAME)?;
        if !self.removed {
            self.original.require_selected(store)?;
            self.original.index.file.sync_all().map_err(io)?;
            self.original.root.file.sync_all().map_err(io)?;
            self.original.require_selected(store)?;
            self.original
                .stage
                .as_ref()
                .ok_or_else(denied)?
                .require(&stage_path)?;
            fs::remove_file(&stage_path).map_err(io)?;
            self.removed = true; // Cache unlink BEFORE a fallible covering barrier.
            drop(self.original.stage.take());
            trip(Fault::IndexStageUnlink)?;
        }
        absent(&stage_path)?; // An external Windows handle may defer deletion.
        self.original.root.file.sync_all().map_err(io)?;
        self.original.require_selected(store)?;
        Ok(())
    }
}

/// All six authenticated old/new frames and their complete sealed intent are
/// cached by the original upgrade owner before the first fixed-name effect.
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
            (self.index, native::MAX_INDEX_FRAME),
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
    seed: Option<HeldNativeInput>,
    catalog_stage: Option<HeldNativeInput>,
    index: Option<HeldNativeInput>,
    marker: Option<HeldNativeInput>,
    base: IndexSnapshot,
}
impl UpgradeInventory {
    pub(in crate::store) fn capture(store: &ServerStore) -> Result<Self, AppError> {
        let value = Self {
            base: IndexSnapshot::capture_inventory(store)?,
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
                native::MAX_INDEX_FRAME,
            )?,
            marker: HeldNativeInput::capture(
                &path(store, true, UPGRADE_MARKER_NAME)?,
                native::MAX_MARKER_FRAME,
            )?,
        };
        value.require(store)?;
        Ok(value)
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
    pub(in crate::store) fn into_index_snapshot(
        self,
        store: &ServerStore,
    ) -> Result<IndexSnapshot, AppError> {
        self.require(store)?;
        if self.seed.is_some() || self.catalog_stage.is_some() || self.has_upgrade_outputs() {
            return Err(denied());
        }
        self.base.require_selected(store)?;
        Ok(self.base)
    }
    pub(in crate::store) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
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
        self.base.require_directories(store)
    }
    /// Startup-private negative role. Root authenticated an unchanged v1 G1
    /// tuple and proved this new seed did not authenticate. No v2-selected
    /// catalog or new output can be discarded through this branch.
    pub(in crate::store) fn abandon_first_torn(self, store: &ServerStore) -> Result<(), AppError> {
        self.require(store)?;
        if self.seed.is_none()
            || self.has_upgrade_outputs()
            || self.base.stage.is_some()
            || self
                .catalog_stage
                .as_ref()
                .is_some_and(|stage| !stage.raw().is_empty())
        {
            return Err(denied());
        }
        let stage_path = path(store, false, critical::Slot::Stage.name())?;
        if let Some(stage) = &self.catalog_stage {
            stage.require(&stage_path)?;
            fs::remove_file(&stage_path).map_err(io)?;
        }
        drop(self.catalog_stage);
        absent(&stage_path)?;
        self.base.vault.file.sync_all().map_err(io)?;
        self.base.require_directories(store)?;
        self.base
            .catalog
            .require(&path(store, false, critical::Slot::Current.name())?)?;
        self.base
            .index
            .require(&path(store, true, native::INDEX_NAME)?)?;
        self.base
            .marker
            .require(&path(store, true, native::MARKER_NAME)?)?;
        let seed_path = path(store, false, UPGRADE_SEED_NAME)?;
        self.seed.as_ref().ok_or_else(denied)?.require(&seed_path)?;
        fs::remove_file(&seed_path).map_err(io)?;
        drop(self.seed);
        absent(&seed_path)?;
        self.base.vault.file.sync_all().map_err(io)?;
        self.base.require_selected(store)
    }
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
        self.original.require_directories(store)?;
        require_no_other_declaration(store)?;
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
        self.original.require_directories(store)
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
            // any content-bearing v2 seed or output can become visible.
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
                fs::rename(&marker_temp, &marker).map_err(io)?;
                self.marker_promoted = true;
                trip(Fault::UpgradeMarkerRename)?;
                self.original.root.file.sync_all().map_err(io)?;
            }
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
