//! Fixed Current4 declaration I/O. No caller path, Source/Scope or SDK/key capability.
//! The only writer is the new still-private offline physical constructor.
use super::*;
use crate::store::critical_namespaces::{
    governance_layout::{
        self as governance, Declaration, GovernanceDirectoryCapture, GovernanceMarkerCapture,
        MountedGovernanceLayout,
    },
    governance_namespace_codec::capability::GovernanceCapabilityData,
    read_pool::CatalogReadReservation,
};
use catcoms_storage::kept::{session_at, session_id, SessionAt, SessionId};
use std::io::{Seek, SeekFrom};
use std::sync::Arc;

fn io(error: impl std::fmt::Display) -> AppError {
    AppError::Io(error.to_string())
}
fn denied() -> AppError {
    critical_namespaces::refused()
}
fn fixed(store: &ServerStore, root: bool, name: &str) -> Result<PathBuf, AppError> {
    let mut base = PathBuf::with_capacity(governance::MAX_BASE_PATH + 256);
    if store.dir.as_os_str().len() > governance::MAX_BASE_PATH {
        return Err(denied());
    }
    base.push(&store.dir);
    if root {
        base.push(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
    }
    // Preserve the final directory component itself. Appending an empty name
    // adds a trailing separator on some platforms, which can make a final
    // symlink be followed by an otherwise correct symlink_metadata probe.
    if !name.is_empty() {
        base.push(name);
    }
    if base.as_os_str().len() > governance::MAX_BASE_PATH + 256
        || base.capacity() > governance::MAX_BASE_PATH + 256
    {
        return Err(denied());
    }
    Ok(base)
}
fn real_file(metadata: &fs::Metadata) -> bool {
    let good = metadata.file_type().is_file() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        good && metadata.file_attributes() & 0x400 == 0
    }
    #[cfg(not(windows))]
    {
        good
    }
}
fn read(path: &Path, cap: usize) -> Result<Option<OpenedArchiveObject>, AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta) if real_file(&meta) && meta.len() <= cap as u64 => {}
        _ => return Err(denied()),
    }
    let value = read_archive_object(path, cap)?.ok_or_else(denied)?;
    if !real_file(&value.0.metadata().map_err(io)?) {
        return Err(denied());
    }
    let id = session_id(&value.0).map_err(io)?;
    if !matches!(session_at(path, id), SessionAt::Ours) {
        return Err(denied());
    }
    Ok(Some(value))
}
fn absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(denied()),
    }
}

/// Fence readers that predate namespace4 using a slot they already inspect.
/// Empty stage framing is deliberately invalid to those readers. The SAME File
/// later becomes the exact candidate stage; no replacement is adopted. Cache
/// ownership before metadata/sync failures so a warm retry can finish its own
/// fence. A cold fence without a complete seed stays preserved and closed.
fn ensure_legacy_stage_fence(
    path: &Path,
    original: &mut Option<critical_namespaces::pending_join::CreatedStage>,
    candidate: &[u8],
) -> Result<(), AppError> {
    if original.is_none() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?;
        *original = Some(critical_namespaces::pending_join::CreatedStage {
            file,
            identity: None,
        });
    }
    let original = original.as_mut().ok_or_else(denied)?;
    let id = match original.identity {
        Some(id) => id,
        None => {
            let id = session_id(&original.file).map_err(io)?;
            original.identity = Some(id);
            id
        }
    };
    held(path, &original.file, id)?;
    let (named, raw) = read(path, candidate.len())?.ok_or_else(denied)?;
    if session_id(&named).map_err(io)? != id
        || raw.len() > candidate.len()
        || raw.as_slice() != &candidate[..raw.len()]
    {
        return Err(denied());
    }
    original.file.sync_all().map_err(io)?;
    held(path, &original.file, id)
}
fn held(path: &Path, file: &File, id: SessionId) -> Result<(), AppError> {
    if !real_file(&file.metadata().map_err(io)?)
        || session_id(file).map_err(io)? != id
        || !matches!(session_at(path, id), SessionAt::Ours)
    {
        return Err(denied());
    }
    Ok(())
}

fn capture_directory(store: &ServerStore) -> Result<GovernanceDirectoryCapture, AppError> {
    let path = fixed(store, true, "")?;
    if !archive_real_directory(&fs::symlink_metadata(&path).map_err(io)?) {
        return Err(denied());
    }
    let file = open_archive_directory(&path)?;
    let identity = session_id(&file).map_err(io)?;
    let capture = GovernanceDirectoryCapture { file, identity };
    validate_directory(store, &capture)?;
    Ok(capture)
}
fn validate_directory(
    store: &ServerStore,
    capture: &GovernanceDirectoryCapture,
) -> Result<(), AppError> {
    let path = fixed(store, true, "")?;
    if !archive_real_directory(&fs::symlink_metadata(&path).map_err(io)?)
        || !archive_real_directory(&capture.file.metadata().map_err(io)?)
        || session_id(&capture.file).map_err(io)? != capture.identity
    {
        return Err(denied());
    }
    let named = open_archive_directory(&path)?;
    if session_id(&named).map_err(io)? != capture.identity {
        return Err(denied());
    }
    Ok(())
}
fn directory_under_root(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    capture: &GovernanceDirectoryCapture,
) -> Result<(), AppError> {
    validate_directory(store, capture)?;
    if session_id(&root.directory).map_err(io)? != capture.identity {
        return Err(denied());
    }
    Ok(())
}

pub(in crate::store) struct GovernanceInventory {
    pub(in crate::store) current: Option<OpenedArchiveObject>,
    pub(in crate::store) stage: Option<OpenedArchiveObject>,
    pub(in crate::store) seed: Option<OpenedArchiveObject>,
    pub(in crate::store) marker: Option<OpenedArchiveObject>,
    pub(in crate::store) directory: GovernanceDirectoryCapture,
}
impl ServerStore {
    pub(in crate::store) fn require_no_prior_governance_declaration_debts(
        &self,
    ) -> Result<(), AppError> {
        absent(&fixed(
            self,
            false,
            critical_namespaces::ARCHIVE_DECLARATION_SEED,
        )?)?;
        absent(&fixed(
            self,
            false,
            critical_namespaces::current_group::DECLARATION_SEED,
        )?)?;
        // An unfinished namespace5 declaration is likewise a prior debt: its owner (the
        // member handler, which runs first when configured) must settle it before 4 starts.
        absent(&fixed(
            self,
            false,
            critical_namespaces::member_outbox_layout::MEMBER_OUTBOX_SEED,
        )?)
    }
    pub(in crate::store) fn read_governance_inventory(
        &self,
        fee: &CatalogReadReservation,
    ) -> Result<GovernanceInventory, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        fee.validate(
            &policy.current.archive.pending.pool,
            governance::codec::DECLARATION_FEE,
        )?;
        // Inventory conflicting old debts FIRST. Neither may be repaired/normalized
        // as an implicit preliminary step of this settled-Current3-only migration.
        self.require_no_prior_governance_declaration_debts()?;
        // The leaf no-reparse test does not protect an intermediate directory.
        // Prove and retain the actual real root BEFORE the marker4 path is opened.
        let directory = capture_directory(self)?;
        #[cfg(test)]
        governance::before_marker_open_for_test();
        let inventory = GovernanceInventory {
            current: read(
                &fixed(self, false, critical_namespaces::Slot::Current.name())?,
                governance::codec::MAX_CATALOG_FRAME,
            )?,
            stage: read(
                &fixed(self, false, critical_namespaces::Slot::Stage.name())?,
                governance::codec::MAX_CATALOG_FRAME,
            )?,
            seed: read(
                &fixed(self, false, governance::GOVERNANCE_SEED)?,
                governance::codec::MAX_SEED_FRAME,
            )?,
            marker: read(
                &fixed(self, true, governance::GOVERNANCE_MARKER)?,
                governance::codec::MAX_MARKER_FRAME,
            )?,
            directory,
        };
        validate_directory(self, &inventory.directory)?;
        Ok(inventory)
    }
    pub(in crate::store) fn require_governance_directory_matches_root(
        &self,
        directory: &GovernanceDirectoryCapture,
    ) -> Result<(), AppError> {
        let root = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        directory_under_root(self, root.as_ref().ok_or_else(denied)?, directory)
    }
    fn governance_fault(&self, point: u8) -> Result<(), AppError> {
        #[cfg(test)]
        if self.critical_namespace_fault.load(Ordering::SeqCst) == point {
            return Err(AppError::Io("injected governance declaration cut".into()));
        }
        let _ = point;
        Ok(())
    }
}

/// Exact-prefix engine for only this private declaration's fixed outputs. The old
/// engine retains its old cap and behavior. No scalar/parsed status chooses this mode.
fn ensure(
    path: &Path,
    expected: &[u8],
    slot: &mut Option<critical_namespaces::pending_join::CreatedStage>,
    mode: ArchiveOutputMode,
    #[cfg(test)] fault: u8,
    #[cfg(test)] short: u8,
) -> Result<(), AppError> {
    if expected.len() < 40 || expected.len() > governance::codec::MAX_SEED_FRAME {
        return Err(denied());
    }
    let had = slot.is_some();
    let mut created = false;
    if slot.is_none() {
        let file = match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if matches!(mode, ArchiveOutputMode::SavedExact) {
                    return Err(denied());
                }
                let newly_opened = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(io)?;
                created = true;
                newly_opened
            }
            Ok(meta) if real_file(&meta) && meta.len() <= expected.len() as u64 => {
                if matches!(mode, ArchiveOutputMode::Original) {
                    // Never cache an unowned partial first seed, even on an error.
                    let (file, raw) = read(path, expected.len())?.ok_or_else(denied)?;
                    if raw.as_slice() != expected {
                        return Err(denied());
                    }
                    file
                } else {
                    let existing = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path)
                        .map_err(io)?;
                    if !real_file(&existing.metadata().map_err(io)?) {
                        return Err(denied());
                    }
                    existing
                }
            }
            _ => return Err(denied()),
        };
        *slot = Some(critical_namespaces::pending_join::CreatedStage {
            file,
            identity: None,
        });
        #[cfg(test)]
        if created && matches!(mode, ArchiveOutputMode::Original) && fault == 230 {
            return Err(AppError::Io("injected empty first governance seed".into()));
        }
    }
    let slot = slot.as_mut().ok_or_else(denied)?;
    if slot.identity.is_none() {
        slot.identity = Some(session_id(&slot.file).map_err(io)?);
    }
    let id = slot.identity.ok_or_else(denied)?;
    held(path, &slot.file, id)?;
    slot.file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut previous = Zeroizing::new(Vec::with_capacity(expected.len() + 1));
    (&mut slot.file)
        .take((expected.len() + 1) as u64)
        .read_to_end(&mut previous)
        .map_err(io)?;
    if previous.len() > expected.len() || previous.as_slice() != &expected[..previous.len()] {
        return Err(denied());
    }
    if previous.len() != expected.len() {
        if matches!(mode, ArchiveOutputMode::SavedExact)
            || (matches!(mode, ArchiveOutputMode::Original) && !had && !created)
        {
            return Err(denied());
        }
        held(path, &slot.file, id)?;
        slot.file
            .seek(SeekFrom::Start(previous.len() as u64))
            .map_err(io)?;
        #[cfg(test)]
        if fault == short && fault != 0 {
            let end = previous.len() + (expected.len() - previous.len()) / 2;
            slot.file
                .write_all(&expected[previous.len()..end])
                .map_err(io)?;
            return Err(AppError::Io("injected governance prefix write".into()));
        }
        slot.file
            .write_all(&expected[previous.len()..])
            .map_err(io)?;
    }
    slot.file.sync_all().map_err(io)?;
    held(path, &slot.file, id)
}

fn require_root(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    capability: GovernanceCapabilityData,
) -> Result<(), AppError> {
    let layout = capability.current_layout_data();
    if root.negative.is_cancelled()
        || !Arc::ptr_eq(&root.mount, &store.registry_mount())
        || root.descriptor.vault != layout.vault
        || root.descriptor.root != layout.root
    {
        return Err(denied());
    }
    let root_path = fixed(store, true, "")?;
    validate_archive_root_session(&root_path, root)?;
    store.validate_mounted_current_layout_under_archive_root(root)
}
fn sync_marker_parent(
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
) -> Result<(), AppError> {
    root.directory.sync_all().map_err(io)
}
fn verify_marker(
    store: &ServerStore,
    capability: GovernanceCapabilityData,
    path: &Path,
    expected: &[u8],
    original: &critical_namespaces::pending_join::CreatedStage,
) -> Result<(), AppError> {
    let id = original.identity.ok_or_else(denied)?;
    held(path, &original.file, id)?;
    let (file, raw) = read(path, governance::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
    if raw.as_slice() != expected || session_id(&file).map_err(io)? != id {
        return Err(denied());
    }
    governance::codec::require_marker(&store.keys.db_key()?, capability, &raw)
}

/// Recompare the SAME retained file after all large/source/old-root waits. This
/// is a bounded streaming comparison, not a new buffer owner or path adoption.
/// The private Root/metadata mutex serializes cursor access on these handles.
fn exact_retained(
    path: &Path,
    original: &critical_namespaces::pending_join::CreatedStage,
    expected: &[u8],
) -> Result<(), AppError> {
    let id = original.identity.ok_or_else(denied)?;
    held(path, &original.file, id)?;
    let mut file = &original.file;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut buffer = [0u8; 8192];
    let mut offset = 0;
    while offset < expected.len() {
        let amount = buffer.len().min(expected.len() - offset);
        file.read_exact(&mut buffer[..amount]).map_err(io)?;
        if buffer[..amount] != expected[offset..offset + amount] {
            return Err(denied());
        }
        offset += amount;
    }
    if file.read(&mut buffer[..1]).map_err(io)? != 0 {
        return Err(denied());
    }
    held(path, &original.file, id)
}
fn verify_seed(
    store: &ServerStore,
    path: &Path,
    declaration: &Declaration,
) -> Result<Option<(File, SessionId)>, AppError> {
    let Some((file, raw)) = read(path, governance::codec::MAX_SEED_FRAME)? else {
        return Ok(None);
    };
    if raw.as_slice() != declaration.seed.as_slice() {
        return Err(denied());
    }
    let id = session_id(&file).map_err(io)?;
    let original = declaration.seed_file.as_ref().ok_or_else(denied)?;
    if original.identity != Some(id) {
        return Err(denied());
    }
    held(path, &original.file, id)?;
    let key = Zeroizing::new(store.keys.db_key()?);
    let plain = governance::codec::open_seed(&key, &raw)?;
    let saved = governance::codec::parse_seed(&key, &store.chat_vault_domain()?, &plain)?;
    if saved.capability != declaration.capability
        || saved.original != declaration.original.as_slice()
        || saved.candidate != declaration.candidate.as_slice()
        || saved.marker != declaration.marker.as_slice()
    {
        return Err(denied());
    }
    file.sync_all().map_err(io)?;
    held(path, &file, id)?;
    Ok(Some((file, id)))
}

impl ServerStore {
    pub(in crate::store) fn write_governance_layout_declaration(
        &self,
        declaration: &mut Declaration,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        policy.require_persistent_capacity()?;
        fee.validate(
            &policy.current.archive.pending.pool,
            governance::codec::DECLARATION_FEE,
        )?;
        self.require_no_prior_governance_declaration_debts()?;
        // Every retry verifies complete retained sources outside Root; no Summary
        // reconstructed from a candidate can substitute for original plaintext.
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = governance::codec::open_seed(&key, &declaration.seed)?;
        let saved = governance::codec::parse_seed(&key, &self.chat_vault_domain()?, &plain)?;
        if saved.capability != declaration.capability
            || saved.original != declaration.original.as_slice()
            || saved.candidate != declaration.candidate.as_slice()
            || saved.marker != declaration.marker.as_slice()
            || *blake3::hash(&declaration.original).as_bytes() != declaration.before.framed
            || *blake3::hash(&declaration.candidate).as_bytes() != declaration.next.framed
        {
            return Err(denied());
        }
        drop(plain);
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        require_root(self, root, declaration.capability)?;
        directory_under_root(self, root, &declaration.directory)?;
        let current_path = fixed(self, false, critical_namespaces::Slot::Current.name())?;
        let stage_path = fixed(self, false, critical_namespaces::Slot::Stage.name())?;
        let seed_path = fixed(self, false, governance::GOVERNANCE_SEED)?;
        let marker_path = fixed(self, true, governance::GOVERNANCE_MARKER)?;
        let (current, raw) =
            read(&current_path, governance::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
        let promoted = raw.as_slice() == declaration.candidate.as_slice();
        let named_id = session_id(&current).map_err(io)?;
        let expected_current =
            if declaration.initial_current_frame == *blake3::hash(&raw).as_bytes() {
                &declaration.initial_current
            } else if promoted {
                declaration.stage_file.as_ref().ok_or_else(denied)?
            } else {
                return Err(denied());
            };
        if expected_current.identity != Some(named_id) {
            return Err(denied());
        }
        held(&current_path, &expected_current.file, named_id)?;
        if !promoted {
            if raw.as_slice() != declaration.original.as_slice() {
                return Err(denied());
            }
            let original_id = session_id(&current).map_err(io)?;
            // The durable old-slot refusal MUST precede every governance-only
            // output. Patching current readers cannot fence an older binary.
            ensure_legacy_stage_fence(
                &stage_path,
                &mut declaration.stage_file,
                &declaration.candidate,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.governance_fault(229)?;
            ensure(
                &seed_path,
                &declaration.seed,
                &mut declaration.seed_file,
                ArchiveOutputMode::Original,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                231,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.governance_fault(232)?;
            ensure(
                &marker_path,
                &declaration.marker,
                &mut declaration.marker_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                233,
            )?;
            sync_marker_parent(root)?;
            self.governance_fault(234)?;
            ensure(
                &stage_path,
                &declaration.candidate,
                &mut declaration.stage_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                235,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.governance_fault(236)?;
            verify_seed(self, &seed_path, declaration)?.ok_or_else(denied)?;
            let (actual, actual_raw) =
                read(&current_path, governance::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
            if actual_raw.as_slice() != declaration.original.as_slice()
                || session_id(&actual).map_err(io)? != original_id
            {
                return Err(denied());
            }
            held(&current_path, &current, original_id)?;
            actual.sync_all().map_err(io)?;
            verify_marker(
                self,
                declaration.capability,
                &marker_path,
                &declaration.marker,
                declaration.marker_file.as_ref().ok_or_else(denied)?,
            )?;
            self.require_no_prior_governance_declaration_debts()?;
            require_root(self, root, declaration.capability)?;
            directory_under_root(self, root, &declaration.directory)?;
            let stage = declaration.stage_file.as_ref().ok_or_else(denied)?;
            stage.file.sync_all().map_err(io)?;
            #[cfg(test)]
            governance::before_stage_selection_for_test();
            // The early ensure/sync cannot cover in-place changes during the
            // independent seed/old-root/marker proof. Select only bytes freshly
            // compared from the SAME retained stage after those waits.
            exact_retained(
                &stage_path,
                declaration.stage_file.as_ref().ok_or_else(denied)?,
                &declaration.candidate,
            )?;
            held(&current_path, &current, original_id)?;
            if root.negative.is_cancelled() || self.chat_mount.observer().is_cancelled() {
                return Err(denied());
            }
            fs::rename(&stage_path, &current_path).map_err(io)?;
            self.governance_fault(237)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
            sync_directory(&self.dir)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        } else {
            // A promoted catalog can only re-sync its already complete marker.
            // Even a complete seed cannot authorize missing/partial-marker creation.
            ensure(
                &marker_path,
                &declaration.marker,
                &mut declaration.marker_file,
                ArchiveOutputMode::SavedExact,
                #[cfg(test)]
                0,
                #[cfg(test)]
                0,
            )?;
            sync_marker_parent(root)?;
            if let Some(stage) = declaration.captured_duplicate_stage.as_ref() {
                exact_retained(&stage_path, stage, &declaration.candidate)?;
                stage.file.sync_all().map_err(io)?;
                exact_retained(&stage_path, stage, &declaration.candidate)?;
                fs::remove_file(&stage_path).map_err(io)?;
                // Cache the completed unlink before any later barrier/refusal.
                // A retry owns no duplicate after this exact name was retired.
                declaration.captured_duplicate_stage = None;
            } else {
                // Warm promotion owned only the name moved into current. Even
                // byte equality (or a hard-link alias) cannot confer a second
                // cleanup role over a newly appearing fixed stage.
                absent(&stage_path)?;
            }
            current.sync_all().map_err(io)?;
            sync_directory(&self.dir).map_err(io)?;
        }
        // All large seed waits finish BEFORE the last small marker/name proof and
        // unlink. A late marker replacement must preserve the independent seed.
        let seed = verify_seed(self, &seed_path, declaration)?;
        let (selected, selected_raw) =
            read(&current_path, governance::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
        if selected_raw.as_slice() != declaration.candidate.as_slice() {
            return Err(denied());
        }
        let selected_id = session_id(&selected).map_err(io)?;
        selected.sync_all().map_err(io)?;
        held(&current_path, &selected, selected_id)?;
        require_root(self, root, declaration.capability)?;
        directory_under_root(self, root, &declaration.directory)?;
        verify_marker(
            self,
            declaration.capability,
            &marker_path,
            &declaration.marker,
            declaration.marker_file.as_ref().ok_or_else(denied)?,
        )?;
        if let Some((seed, id)) = seed {
            held(&seed_path, &seed, id)?;
            fs::remove_file(&seed_path).map_err(io)?;
            self.governance_fault(238)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        }
        sync_directory(&self.dir).map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        self.governance_fault(239)
    }

    pub(in crate::store) fn capture_governance_layout_marker(
        &self,
        capability: GovernanceCapabilityData,
        expected: &critical_namespaces::pending_join::CreatedStage,
    ) -> Result<GovernanceMarkerCapture, AppError> {
        let path = fixed(self, true, governance::GOVERNANCE_MARKER)?;
        let directory = capture_directory(self)?;
        let id = expected.identity.ok_or_else(denied)?;
        held(&path, &expected.file, id)?;
        let (marker, raw) = read(&path, governance::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
        if session_id(&marker).map_err(io)? != id {
            return Err(denied());
        }
        self.retain_governance_marker_capture(capability, marker, raw, directory)
    }

    pub(in crate::store) fn retain_governance_marker_capture(
        &self,
        capability: GovernanceCapabilityData,
        marker: File,
        raw: Zeroizing<Vec<u8>>,
        directory: GovernanceDirectoryCapture,
    ) -> Result<GovernanceMarkerCapture, AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        // A separate small retained debit outlives the large inspector phase.
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(governance::MOUNTED_FEE)?;
        fee.validate(
            &policy.current.archive.pending.pool,
            governance::MOUNTED_FEE,
        )?;
        governance::codec::require_marker(&self.keys.db_key()?, capability, &raw)?;
        let marker_id = session_id(&marker).map_err(io)?;
        held(
            &fixed(self, true, governance::GOVERNANCE_MARKER)?,
            &marker,
            marker_id,
        )?;
        validate_directory(self, &directory)?;
        Ok(GovernanceMarkerCapture {
            capability,
            marker,
            marker_id,
            raw,
            directory,
            fee,
        })
    }

    pub(in crate::store) fn install_governance_layout_marker(
        &self,
        capture: GovernanceMarkerCapture,
    ) -> Result<(), AppError> {
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        capture.fee.validate(
            &policy.current.archive.pending.pool,
            governance::MOUNTED_FEE,
        )?;
        let GovernanceMarkerCapture {
            capability,
            marker,
            marker_id,
            raw,
            directory,
            fee,
        } = capture;
        // Declare the possible retired owner BEFORE Root so the final fee/file Drop
        // on every error occurs only after the root/metadata mutexes are released.
        let mut next = Some(MountedGovernanceLayout {
            capability,
            marker,
            marker_id,
            marker_digest: *blake3::hash(&raw).as_bytes(),
            directory,
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            _fee: fee,
        });
        drop(raw);
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        require_root(self, root, capability)?;
        let original = next.as_ref().ok_or_else(denied)?;
        directory_under_root(self, root, &original.directory)?;
        held(
            &fixed(self, true, governance::GOVERNANCE_MARKER)?,
            &original.marker,
            marker_id,
        )?;
        original.marker.sync_all().map_err(io)?;
        sync_marker_parent(root)?;
        {
            let mut installed = self.governance_layout.lock().map_err(|_| denied())?;
            // Acquire the metadata mutex before the final actual marker/name
            // proof. Even this private constructor must not cache across a wait.
            validate_mounted(self, root, next.as_ref().ok_or_else(denied)?)?;
            if let Some(existing) = installed.as_ref() {
                validate_mounted(self, root, existing)?;
                if existing.capability != capability || existing.marker_id != marker_id {
                    return Err(denied());
                }
            } else {
                *installed = next.take();
            }
        }
        drop(root_guard);
        drop(next);
        Ok(())
    }

    pub(in crate::store) fn validate_mounted_governance_layout(&self) -> Result<(), AppError> {
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        let mounted = self.governance_layout.lock().map_err(|_| denied())?;
        validate_mounted(self, root, mounted.as_ref().ok_or_else(denied)?)
    }
}

fn validate_mounted(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    mounted: &MountedGovernanceLayout,
) -> Result<(), AppError> {
    let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
    mounted._fee.validate(
        &policy.current.archive.pending.pool,
        governance::MOUNTED_FEE,
    )?;
    if mounted.negative.is_cancelled() || !Arc::ptr_eq(&mounted.mount, &store.registry_mount()) {
        return Err(denied());
    }
    require_root(store, root, mounted.capability)?;
    directory_under_root(store, root, &mounted.directory)?;
    let path = fixed(store, true, governance::GOVERNANCE_MARKER)?;
    held(&path, &mounted.marker, mounted.marker_id)?;
    let (file, raw) = read(&path, governance::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
    if session_id(&file).map_err(io)? != mounted.marker_id
        || *blake3::hash(&raw).as_bytes() != mounted.marker_digest
    {
        return Err(denied());
    }
    governance::codec::require_marker(&store.keys.db_key()?, mounted.capability, &raw)?;
    require_root(store, root, mounted.capability)?;
    directory_under_root(store, root, &mounted.directory)?;
    #[cfg(test)]
    governance::after_marker_verification_for_test();
    // The old-root proof above can wait/read after the first4 comparison. Rehash
    // the SAME retained4 file now, using only bounded stack scratch. A same-ID,
    // same-length in-place change is not covered by a final metadata/name check.
    let mut marker = &mounted.marker;
    marker.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut final_bytes = [0u8; governance::codec::MAX_MARKER_FRAME + 1];
    let mut amount = 0;
    loop {
        let read = marker.read(&mut final_bytes[amount..]).map_err(io)?;
        if read == 0 {
            break;
        }
        amount += read;
        if amount > governance::codec::MAX_MARKER_FRAME {
            return Err(denied());
        }
    }
    if *blake3::hash(&final_bytes[..amount]).as_bytes() != mounted.marker_digest {
        return Err(denied());
    }
    // No account wait or old-marker IO follows this last full4 comparison.
    held(&path, &mounted.marker, mounted.marker_id)?;
    if mounted.negative.is_cancelled() {
        return Err(denied());
    }
    Ok(())
}
