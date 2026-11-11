//! Fixed namespace5 (member outbox layout) declaration and mount I/O.
//!
//! No caller path, Source/Scope, SDK/key or outbox capability. Every path is derived from the
//! original Store and the closed names in `critical_namespaces::member_outbox_layout`. The
//! write protocol mirrors governance4's reviewed one (legacy stage fence, exact-prefix
//! completion of owned outputs only, same-File re-comparison before selection, seed removal
//! last) with two differences. A zero-byte CLAIM of the fixed seed name precedes the fence, so
//! every pre-commit state holding a namespace5-created object holds that name (see
//! `dispose_first_torn_member_outbox_seed`). And it adds one declared output: the member parent
//! directory, created after the marker and BEFORE promotion, so a promoted catalog always
//! implies an existing directory.
//!
//! Warm fault cuts (cfg(test), `critical_namespace_fault`), in write order: 241 after the claim
//! (claim only, no fence), 240 after the empty stage fence (claim + fence), 242 torn seed
//! content, 243 after the complete seed, 244 torn marker, 245 after the marker, 246 after the
//! parent directory, 247 torn candidate stage, 248 after the candidate stage, 249 after
//! promotion, 250 after seed removal, 255 after the final barrier. 251-254 belong to the
//! group-directory family on the same atomic. Cold first-torn disposal cuts use a separate
//! thread-local seam (the Store, and so its atomic, is created inside the open that runs
//! them): 226 after the fence unlink, 227 after its directory barrier, 228 after the seed
//! unlink.
use super::*;
use crate::store::critical_namespaces::{
    member_outbox_layout::{
        self as member, Declaration, MemberOutboxDirectoryCapture, MemberOutboxMarkerCapture,
        MountedMemberOutboxLayout,
    },
    member_outbox_namespace_codec::MemberOutboxCapabilityData,
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
/// `root`: under `chat-owner-v2`; otherwise directly under the vault directory.
fn fixed(store: &ServerStore, root: bool, name: &str) -> Result<PathBuf, AppError> {
    let mut base = PathBuf::with_capacity(member::MAX_BASE_PATH + 256);
    if store.dir.as_os_str().len() > member::MAX_BASE_PATH {
        return Err(denied());
    }
    base.push(&store.dir);
    if root {
        base.push(critical_namespaces::ARCHIVE_BOOTSTRAP_ROOT);
    }
    // Preserve the final directory component itself; an empty name would add a trailing
    // separator, which can make a final symlink be followed by a symlink_metadata probe.
    if !name.is_empty() {
        base.push(name);
    }
    if base.as_os_str().len() > member::MAX_BASE_PATH + 256
        || base.capacity() > member::MAX_BASE_PATH + 256
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
/// After an unlink, every handle dropped and a directory barrier: the NAME must really be gone.
/// On Windows a name another process holds open (with delete sharing) can stay delete-pending
/// under legacy delete semantics, and a power loss before that handle closes brings the file
/// back. Any result but NotFound (including the delete-pending access error) is therefore a
/// refusal: the caller must not touch the next object until the previous one is provably gone.
fn proven_absent(path: &Path) -> Result<(), AppError> {
    #[cfg(test)]
    if member::take_name_reported_present_for_test() {
        return Err(denied());
    }
    absent(path)
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

/// Claim the fixed seed name BEFORE the legacy fence: a zero-byte reservation, synced, followed
/// by the caller's directory barrier. It carries no content and depends on no catalog, so a
/// binary that predates namespace5 may ignore it harmlessly. What it buys is attribution: a
/// fence is never created without this name, so the first-torn role can tell this
/// declaration's empty fence from another writer's empty stage, which it never touches. The
/// SAME held File later receives the complete seed; no replacement is adopted. A retry with the
/// slot already held only re-proves and re-syncs it.
fn ensure_seed_claim(
    path: &Path,
    slot: &mut Option<critical_namespaces::pending_join::CreatedStage>,
) -> Result<(), AppError> {
    if slot.is_none() {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io)?;
        // Keep the ACTUAL file before the first fallible identity query or sync.
        *slot = Some(critical_namespaces::pending_join::CreatedStage {
            file,
            identity: None,
        });
    }
    let claim = slot.as_mut().ok_or_else(denied)?;
    let id = match claim.identity {
        Some(id) => id,
        None => {
            let id = session_id(&claim.file).map_err(io)?;
            claim.identity = Some(id);
            id
        }
    };
    held(path, &claim.file, id)?;
    claim.file.sync_all().map_err(io)?;
    held(path, &claim.file, id)
}

/// Fence readers that predate namespace5 using a slot they already inspect: an owned EMPTY
/// legacy catalog stage. The SAME File later becomes the exact candidate stage; no
/// replacement is adopted. It always follows the seed claim. Cold, a fence beside an
/// unauthenticated seed is disposable first-torn scratch; any other uncommitted fence stays
/// preserved and refused.
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

/// Capture one fixed real directory (no-reparse), by path, retaining its handle.
fn capture_directory_at(path: &Path) -> Result<MemberOutboxDirectoryCapture, AppError> {
    if !archive_real_directory(&fs::symlink_metadata(path).map_err(io)?) {
        return Err(denied());
    }
    let file = open_archive_directory(path)?;
    let identity = session_id(&file).map_err(io)?;
    let capture = MemberOutboxDirectoryCapture { file, identity };
    validate_directory_at(path, &capture)?;
    Ok(capture)
}
fn validate_directory_at(
    path: &Path,
    capture: &MemberOutboxDirectoryCapture,
) -> Result<(), AppError> {
    if !archive_real_directory(&fs::symlink_metadata(path).map_err(io)?)
        || !archive_real_directory(&capture.file.metadata().map_err(io)?)
        || session_id(&capture.file).map_err(io)? != capture.identity
    {
        return Err(denied());
    }
    let named = open_archive_directory(path)?;
    if session_id(&named).map_err(io)? != capture.identity {
        return Err(denied());
    }
    Ok(())
}
fn root_under_archive(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    capture: &MemberOutboxDirectoryCapture,
) -> Result<(), AppError> {
    validate_directory_at(&fixed(store, true, "")?, capture)?;
    if session_id(&root.directory).map_err(io)? != capture.identity {
        return Err(denied());
    }
    Ok(())
}
/// The member parent directory may be created only by an owned declaration that has already
/// written its marker. `create` is false for an already-promoted (or settled) catalog: a
/// missing directory then stays a preserved refusal, never a re-creation.
fn ensure_parent_directory(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    slot: &mut Option<MemberOutboxDirectoryCapture>,
    create: bool,
) -> Result<(), AppError> {
    let path = fixed(store, true, member::MEMBER_OUTBOX_PARENT)?;
    if slot.is_none() {
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
                fs::create_dir(&path).map_err(io)?;
            }
            Ok(metadata) if archive_real_directory(&metadata) => {}
            _ => return Err(denied()),
        }
        *slot = Some(capture_directory_at(&path)?);
    }
    validate_directory_at(&path, slot.as_ref().ok_or_else(denied)?)?;
    // Only declaration writes call this, and this Store installs the namespace5 mount only
    // after such a write fully succeeds (even a warm retry after the seed was removed has no
    // mount yet). So no outbox namespace can exist under the parent: a non-empty directory
    // (for example one restored from elsewhere) is never adopted.
    require_empty_directory(&path)?;
    sync_parent_of_marker(root)?;
    validate_directory_at(&path, slot.as_ref().ok_or_else(denied)?)
}
fn require_empty_directory(path: &Path) -> Result<(), AppError> {
    let mut entries = fs::read_dir(path).map_err(io)?;
    if entries.next().is_some() {
        return Err(denied());
    }
    Ok(())
}
fn sync_parent_of_marker(
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
) -> Result<(), AppError> {
    root.directory.sync_all().map_err(io)
}

/// Exact-prefix engine for only this declaration's fixed outputs (same rules as governance4).
fn ensure(
    path: &Path,
    expected: &[u8],
    slot: &mut Option<critical_namespaces::pending_join::CreatedStage>,
    mode: ArchiveOutputMode,
    #[cfg(test)] fault: u8,
    #[cfg(test)] short: u8,
) -> Result<(), AppError> {
    if expected.len() < 40 || expected.len() > member::codec::MAX_SEED_FRAME {
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
            return Err(AppError::Io("injected member-outbox prefix write".into()));
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
    capability: MemberOutboxCapabilityData,
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
fn verify_marker(
    store: &ServerStore,
    capability: MemberOutboxCapabilityData,
    path: &Path,
    expected: &[u8],
    original: &critical_namespaces::pending_join::CreatedStage,
) -> Result<(), AppError> {
    let id = original.identity.ok_or_else(denied)?;
    held(path, &original.file, id)?;
    let (file, raw) = read(path, member::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
    if raw.as_slice() != expected || session_id(&file).map_err(io)? != id {
        return Err(denied());
    }
    member::codec::require_marker(&store.keys.db_key()?, capability, &raw)
}

/// Recompare the SAME retained file after all large/source/old-root waits (bounded streaming
/// comparison; no new buffer owner or path adoption).
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
    let Some((file, raw)) = read(path, member::codec::MAX_SEED_FRAME)? else {
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
    let plain = member::codec::open_seed(&key, &raw)?;
    let saved = member::codec::parse_seed(&key, &store.chat_vault_domain()?, &plain)?;
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

/// Both fixed catalog candidates, the seed, the marker, the parent directory (if any) and the
/// held `chat-owner-v2` root, read before any older inspector or cleaner runs.
pub(in crate::store) struct MemberOutboxInventory {
    pub(in crate::store) current: Option<OpenedArchiveObject>,
    pub(in crate::store) stage: Option<OpenedArchiveObject>,
    pub(in crate::store) seed: Option<OpenedArchiveObject>,
    pub(in crate::store) marker: Option<OpenedArchiveObject>,
    pub(in crate::store) parent: Option<MemberOutboxDirectoryCapture>,
    pub(in crate::store) root: MemberOutboxDirectoryCapture,
}

impl ServerStore {
    /// The concrete member-outbox handler configured on THIS opener, if any.
    pub(in crate::store) fn member_outbox_policy(
        &self,
    ) -> Option<&critical_namespaces::V2MemberOutboxPolicy> {
        self.current_layout_policy
            .as_ref()
            .and_then(|policy| policy.member_outbox.as_ref())
    }
    /// The catalog/JOIN pool that pays the transient declaration and mounted-marker fees.
    pub(in crate::store) fn member_outbox_declaration_pool(
        &self,
    ) -> Result<&CriticalCatalogReadPool, AppError> {
        self.member_outbox_policy().ok_or_else(denied)?;
        Ok(&self
            .current_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .archive
            .pending
            .pool)
    }
    /// Another namespace's declaration debt must be settled first; none is repaired or
    /// normalized as an implicit preliminary step of a member-outbox declaration.
    pub(in crate::store) fn require_no_prior_member_outbox_declaration_debts(
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
        absent(&fixed(
            self,
            false,
            critical_namespaces::governance_layout::GOVERNANCE_SEED,
        )?)
    }
    /// The ONE prior debt the first-torn role may sit beside: a governance4 seed (archive and
    /// Current3 seeds still refuse). Only a binary that predates namespace5 can produce it. Its
    /// governance opener does not know the member seed name, so during the zero-byte claim
    /// window (or a disposal intermediate) it can begin, and crash inside, a governance
    /// declaration. Neither head writer can: each refuses while the other's seed exists.
    ///
    /// That governance seed must be COMPLETE and authenticated (so the governance inspector
    /// will finish it). A torn governance first seed is governance's own unrecovered state,
    /// refused by its inspector; the member seed beside it is then preserved and refused
    /// too, rather than disposed in front of that refusal.
    pub(in crate::store) fn require_member_outbox_debts_beside_governance(
        &self,
    ) -> Result<(), AppError> {
        use critical_namespaces::governance_layout::{codec as governance, GOVERNANCE_SEED};
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
        let (_file, raw) = read(
            &fixed(self, false, GOVERNANCE_SEED)?,
            governance::MAX_SEED_FRAME,
        )?
        .ok_or_else(denied)?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = governance::open_seed(&key, &raw)?;
        governance::parse_seed(&key, &self.chat_vault_domain()?, &plain)?;
        Ok(())
    }
    /// Whether a member-outbox declaration seed exists (presence of ANY kind counts). DATA for
    /// gate ordering only: the actual seed is authenticated by the inspector.
    pub(in crate::store) fn member_outbox_seed_present(&self) -> Result<bool, AppError> {
        match fs::symlink_metadata(fixed(self, false, member::MEMBER_OUTBOX_SEED)?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            _ => Ok(true),
        }
    }
    /// Whether a governance4 declaration seed exists (presence of ANY kind counts). DATA for
    /// choosing the member inventory's precondition only; governance owns that seed.
    pub(in crate::store) fn governance_declaration_seed_present(&self) -> Result<bool, AppError> {
        match fs::symlink_metadata(fixed(
            self,
            false,
            critical_namespaces::governance_layout::GOVERNANCE_SEED,
        )?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            _ => Ok(true),
        }
    }
    /// Whether the fixed member parent directory has no entries. DATA for the cold inspector's
    /// refusal BEFORE any recovery write; a read failure is an error, never "empty".
    pub(in crate::store) fn member_outbox_parent_is_empty(&self) -> Result<bool, AppError> {
        let mut entries =
            fs::read_dir(fixed(self, true, member::MEMBER_OUTBOX_PARENT)?).map_err(io)?;
        Ok(entries.next().is_none())
    }
    /// `beside_governance`: the cold inspector's first-torn sub-role only (see
    /// [`Self::require_member_outbox_debts_beside_governance`]); every other reader requires no
    /// prior declaration debt at all.
    pub(in crate::store) fn read_member_outbox_inventory(
        &self,
        fee: &CatalogReadReservation,
        beside_governance: bool,
    ) -> Result<MemberOutboxInventory, AppError> {
        fee.validate(
            self.member_outbox_declaration_pool()?,
            member::codec::DECLARATION_FEE,
        )?;
        if beside_governance {
            self.require_member_outbox_debts_beside_governance()?;
        } else {
            self.require_no_prior_member_outbox_declaration_debts()?;
        }
        // The leaf no-reparse test does not protect an intermediate directory. Prove and
        // retain the actual real root BEFORE the marker or parent path is opened.
        let root = capture_directory_at(&fixed(self, true, "")?)?;
        let parent_path = fixed(self, true, member::MEMBER_OUTBOX_PARENT)?;
        let parent = match fs::symlink_metadata(&parent_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Ok(metadata) if archive_real_directory(&metadata) => {
                Some(capture_directory_at(&parent_path)?)
            }
            _ => return Err(denied()),
        };
        let inventory = MemberOutboxInventory {
            current: read(
                &fixed(self, false, critical_namespaces::Slot::Current.name())?,
                member::codec::MAX_CATALOG_FRAME,
            )?,
            stage: read(
                &fixed(self, false, critical_namespaces::Slot::Stage.name())?,
                member::codec::MAX_CATALOG_FRAME,
            )?,
            seed: read(
                &fixed(self, false, member::MEMBER_OUTBOX_SEED)?,
                member::codec::MAX_SEED_FRAME,
            )?,
            marker: read(
                &fixed(self, true, member::MEMBER_OUTBOX_MARKER)?,
                member::codec::MAX_MARKER_FRAME,
            )?,
            parent,
            root,
        };
        validate_directory_at(&fixed(self, true, "")?, &inventory.root)?;
        Ok(inventory)
    }
    pub(in crate::store) fn require_member_outbox_root_matches(
        &self,
        root_capture: &MemberOutboxDirectoryCapture,
    ) -> Result<(), AppError> {
        let root = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        root_under_archive(self, root.as_ref().ok_or_else(denied)?, root_capture)
    }
    fn member_outbox_fault(&self, point: u8) -> Result<(), AppError> {
        #[cfg(test)]
        if self.critical_namespace_fault.load(Ordering::SeqCst) == point {
            return Err(AppError::Io(
                "injected member-outbox declaration cut".into(),
            ));
        }
        let _ = point;
        Ok(())
    }

    /// Abandon a namespace5 declaration whose FIRST seed never authenticated: unlink the
    /// classified zero-byte fence (if any), then the unauthenticated seed, each followed by a
    /// directory barrier and a NotFound proof of its name. No RNG, no reseal, no other path.
    ///
    /// Callers (the cold member inspector only) must already have proven, from the same paid
    /// inventory: no marker and no parent directory; an authenticated current catalog naming
    /// the archive and Current3 but not ID5; the settled Current3 layout's marker; the seed
    /// `Unauthenticated` (`codec::classify_seed`); and the stage's role. Nothing downstream of
    /// a complete seed exists, so these bytes cannot carry any declaration or authority.
    ///
    /// Order: the fence goes FIRST. Every intermediate (an unauthenticated seed with or without
    /// the fence) is then first-torn again on the next open. Removing the seed first would
    /// leave a fence with no claim, which no reader can attribute. Each name must be provably
    /// gone before the next object is touched (see `proven_absent`).
    pub(in crate::store) fn dispose_first_torn_member_outbox_seed(
        &self,
        stage: member::FirstTornStage,
        seed: OpenedArchiveObject,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        fee.validate(
            self.member_outbox_declaration_pool()?,
            member::codec::DECLARATION_FEE,
        )?;
        let stage_path = fixed(self, false, critical_namespaces::Slot::Stage.name())?;
        let seed_path = fixed(self, false, member::MEMBER_OUTBOX_SEED)?;
        // Re-proven at the edge: nothing a complete seed precedes has appeared.
        absent(&fixed(self, true, member::MEMBER_OUTBOX_MARKER)?)?;
        absent(&fixed(self, true, member::MEMBER_OUTBOX_PARENT)?)?;
        let (seed_file, seed_raw) = seed;
        // Defence in depth: this edge itself refuses an authenticated seed, whatever the
        // caller classified. Only bytes that cannot authenticate are ever discarded.
        let key = Zeroizing::new(self.keys.db_key()?);
        if !matches!(
            member::codec::classify_seed(&key, &seed_raw)?,
            member::codec::SeedEvidence::Unauthenticated
        ) {
            return Err(denied());
        }
        drop(key);
        let seed = critical_namespaces::pending_join::CreatedStage {
            identity: Some(session_id(&seed_file).map_err(io)?),
            file: seed_file,
        };
        exact_retained(&seed_path, &seed, &seed_raw)?;
        if let member::FirstTornStage::Fence((file, raw)) = stage {
            let id = session_id(&file).map_err(io)?;
            held(&stage_path, &file, id)?;
            if !raw.is_empty() || file.metadata().map_err(io)?.len() != 0 {
                return Err(denied());
            }
            fs::remove_file(&stage_path).map_err(io)?;
            // Every handle to the stage drops before the barrier and the name proof.
            drop(file);
            member::member_cold_fault(226)?;
            sync_directory(&self.dir).map_err(io)?;
            member::member_cold_fault(227)?;
            proven_absent(&stage_path)?;
        }
        // The SAME held seed, still exactly the classified bytes.
        exact_retained(&seed_path, &seed, &seed_raw)?;
        fs::remove_file(&seed_path).map_err(io)?;
        drop(seed);
        member::member_cold_fault(228)?;
        sync_directory(&self.dir).map_err(io)?;
        proven_absent(&seed_path)
    }

    /// Finish one exact retained declaration, warm or cold. Every retry re-verifies the complete
    /// retained sources; no Summary reconstructed from a candidate substitutes for original
    /// plaintext, and nothing is resealed.
    pub(in crate::store) fn write_member_outbox_layout_declaration(
        &self,
        declaration: &mut Declaration,
        fee: &CatalogReadReservation,
    ) -> Result<(), AppError> {
        let policy = self.member_outbox_policy().ok_or_else(denied)?;
        policy.require_capacity()?;
        fee.validate(
            self.member_outbox_declaration_pool()?,
            member::codec::DECLARATION_FEE,
        )?;
        self.require_no_prior_member_outbox_declaration_debts()?;
        let key = Zeroizing::new(self.keys.db_key()?);
        let plain = member::codec::open_seed(&key, &declaration.seed)?;
        let saved = member::codec::parse_seed(&key, &self.chat_vault_domain()?, &plain)?;
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
        root_under_archive(self, root, &declaration.root)?;
        let current_path = fixed(self, false, critical_namespaces::Slot::Current.name())?;
        let stage_path = fixed(self, false, critical_namespaces::Slot::Stage.name())?;
        let seed_path = fixed(self, false, member::MEMBER_OUTBOX_SEED)?;
        let marker_path = fixed(self, true, member::MEMBER_OUTBOX_MARKER)?;
        let (current, raw) =
            read(&current_path, member::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
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
            // Claim the seed name first (zero bytes, durable before the fence), so no fence
            // ever exists without it. A held slot (warm retry, cold complete seed) is only
            // re-proven here.
            ensure_seed_claim(&seed_path, &mut declaration.seed_file)?;
            sync_directory(&self.dir).map_err(io)?;
            self.member_outbox_fault(241)?;
            // The durable old-slot refusal MUST precede every CONTENT-BEARING member-only
            // output (seed bytes, marker, parent, candidate). Patching current readers cannot
            // fence an older binary. The zero-byte claim above carries nothing it could misuse.
            ensure_legacy_stage_fence(
                &stage_path,
                &mut declaration.stage_file,
                &declaration.candidate,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.member_outbox_fault(240)?;
            ensure(
                &seed_path,
                &declaration.seed,
                &mut declaration.seed_file,
                ArchiveOutputMode::Original,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                242,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.member_outbox_fault(243)?;
            ensure(
                &marker_path,
                &declaration.marker,
                &mut declaration.marker_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                244,
            )?;
            sync_parent_of_marker(root)?;
            self.member_outbox_fault(245)?;
            ensure_parent_directory(self, root, &mut declaration.parent, true)?;
            self.member_outbox_fault(246)?;
            ensure(
                &stage_path,
                &declaration.candidate,
                &mut declaration.stage_file,
                ArchiveOutputMode::SeedRepair,
                #[cfg(test)]
                self.critical_namespace_fault.load(Ordering::SeqCst),
                #[cfg(test)]
                247,
            )?;
            sync_directory(&self.dir).map_err(io)?;
            self.member_outbox_fault(248)?;
            verify_seed(self, &seed_path, declaration)?.ok_or_else(denied)?;
            let (actual, actual_raw) =
                read(&current_path, member::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
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
            self.require_no_prior_member_outbox_declaration_debts()?;
            require_root(self, root, declaration.capability)?;
            root_under_archive(self, root, &declaration.root)?;
            ensure_parent_directory(self, root, &mut declaration.parent, false)?;
            let stage = declaration.stage_file.as_ref().ok_or_else(denied)?;
            stage.file.sync_all().map_err(io)?;
            // Select only bytes freshly compared from the SAME retained stage after every
            // independent seed/root/marker/directory wait above.
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
            self.member_outbox_fault(249)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
            sync_directory(&self.dir)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        } else {
            // A promoted catalog can only re-sync its already complete marker and already
            // existing parent directory. Even a complete seed cannot authorize creating a
            // missing/partial marker or a missing directory after the commit point.
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
            sync_parent_of_marker(root)?;
            ensure_parent_directory(self, root, &mut declaration.parent, false)?;
            if let Some(stage) = declaration.captured_duplicate_stage.as_ref() {
                exact_retained(&stage_path, stage, &declaration.candidate)?;
                stage.file.sync_all().map_err(io)?;
                exact_retained(&stage_path, stage, &declaration.candidate)?;
                fs::remove_file(&stage_path).map_err(io)?;
                // Cache the completed unlink before any later barrier/refusal.
                declaration.captured_duplicate_stage = None;
            } else {
                // Warm promotion owned only the name moved into current. Byte equality or a
                // hard-link alias confers no cleanup role over a newly appearing stage.
                absent(&stage_path)?;
            }
            current.sync_all().map_err(io)?;
            sync_directory(&self.dir).map_err(io)?;
        }
        // All large seed waits finish BEFORE the last small marker/name proof and unlink. A
        // late marker or directory replacement must preserve the independent seed.
        let seed = verify_seed(self, &seed_path, declaration)?;
        let (selected, selected_raw) =
            read(&current_path, member::codec::MAX_CATALOG_FRAME)?.ok_or_else(denied)?;
        if selected_raw.as_slice() != declaration.candidate.as_slice() {
            return Err(denied());
        }
        let selected_id = session_id(&selected).map_err(io)?;
        selected.sync_all().map_err(io)?;
        held(&current_path, &selected, selected_id)?;
        require_root(self, root, declaration.capability)?;
        root_under_archive(self, root, &declaration.root)?;
        verify_marker(
            self,
            declaration.capability,
            &marker_path,
            &declaration.marker,
            declaration.marker_file.as_ref().ok_or_else(denied)?,
        )?;
        ensure_parent_directory(self, root, &mut declaration.parent, false)?;
        if let Some((seed, id)) = seed {
            held(&seed_path, &seed, id)?;
            fs::remove_file(&seed_path).map_err(io)?;
            self.member_outbox_fault(250)
                .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        }
        sync_directory(&self.dir).map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        self.member_outbox_fault(255)
    }

    /// Capture the SAME marker object the declaration wrote (or the inspector probed) and the
    /// held parent directory, for installation after older gates.
    pub(in crate::store) fn capture_member_outbox_layout_marker(
        &self,
        capability: MemberOutboxCapabilityData,
        expected: &critical_namespaces::pending_join::CreatedStage,
        parent: MemberOutboxDirectoryCapture,
    ) -> Result<MemberOutboxMarkerCapture, AppError> {
        let path = fixed(self, true, member::MEMBER_OUTBOX_MARKER)?;
        let root = capture_directory_at(&fixed(self, true, "")?)?;
        let id = expected.identity.ok_or_else(denied)?;
        held(&path, &expected.file, id)?;
        let (marker, raw) = read(&path, member::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
        if session_id(&marker).map_err(io)? != id {
            return Err(denied());
        }
        self.retain_member_outbox_marker_capture(capability, marker, raw, root, parent)
    }

    pub(in crate::store) fn retain_member_outbox_marker_capture(
        &self,
        capability: MemberOutboxCapabilityData,
        marker: File,
        raw: Zeroizing<Vec<u8>>,
        root: MemberOutboxDirectoryCapture,
        parent: MemberOutboxDirectoryCapture,
    ) -> Result<MemberOutboxMarkerCapture, AppError> {
        let pool = self.member_outbox_declaration_pool()?;
        // A separate small retained debit outlives the large inspector phase.
        let fee = pool.try_reserve(member::MOUNTED_FEE)?;
        fee.validate(pool, member::MOUNTED_FEE)?;
        member::codec::require_marker(&self.keys.db_key()?, capability, &raw)?;
        let marker_id = session_id(&marker).map_err(io)?;
        held(
            &fixed(self, true, member::MEMBER_OUTBOX_MARKER)?,
            &marker,
            marker_id,
        )?;
        validate_directory_at(&fixed(self, true, "")?, &root)?;
        validate_directory_at(&fixed(self, true, member::MEMBER_OUTBOX_PARENT)?, &parent)?;
        Ok(MemberOutboxMarkerCapture {
            capability,
            marker,
            marker_id,
            raw,
            root,
            parent,
            fee,
        })
    }

    pub(in crate::store) fn install_member_outbox_layout_marker(
        &self,
        capture: MemberOutboxMarkerCapture,
    ) -> Result<(), AppError> {
        capture
            .fee
            .validate(self.member_outbox_declaration_pool()?, member::MOUNTED_FEE)?;
        let MemberOutboxMarkerCapture {
            capability,
            marker,
            marker_id,
            raw,
            root: root_capture,
            parent,
            fee,
        } = capture;
        // Declare the possible retired owner BEFORE Root so the final fee/file Drop on every
        // error occurs only after the root/metadata mutexes are released.
        let mut next = Some(MountedMemberOutboxLayout {
            capability,
            marker,
            marker_id,
            marker_digest: *blake3::hash(&raw).as_bytes(),
            root: root_capture,
            parent,
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            _fee: fee,
        });
        drop(raw);
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        require_root(self, root, capability)?;
        let original = next.as_ref().ok_or_else(denied)?;
        root_under_archive(self, root, &original.root)?;
        held(
            &fixed(self, true, member::MEMBER_OUTBOX_MARKER)?,
            &original.marker,
            marker_id,
        )?;
        original.marker.sync_all().map_err(io)?;
        sync_parent_of_marker(root)?;
        {
            let mut installed = self.member_outbox_layout.lock().map_err(|_| denied())?;
            // Acquire the metadata mutex before the final actual marker/name proof.
            validate_mounted(self, root, next.as_ref().ok_or_else(denied)?)?;
            if let Some(existing) = installed.as_ref() {
                validate_mounted(self, root, existing)?;
                if existing.capability != capability
                    || existing.marker_id != marker_id
                    || existing.parent.identity != next.as_ref().ok_or_else(denied)?.parent.identity
                {
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

    /// Fresh proof that THIS Store's namespace5 marker and parent directory are still the
    /// exact installed objects. Required before any member-outbox owner is opened.
    pub(in crate::store) fn validate_mounted_member_outbox_layout(&self) -> Result<(), AppError> {
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        let mounted = self.member_outbox_layout.lock().map_err(|_| denied())?;
        validate_mounted(self, root, mounted.as_ref().ok_or_else(denied)?)
    }

    /// The held member parent directory's fixed path and OS identity, re-proven under the
    /// mount. The only production input of `MemberOutboxParentDirectory::from_mount`.
    pub(in crate::store) fn member_outbox_parent_data(
        &self,
    ) -> Result<(PathBuf, SessionId), AppError> {
        let root_guard = self.archive_bootstrap_root.lock().map_err(|_| denied())?;
        let root = root_guard.as_ref().ok_or_else(denied)?;
        let mounted = self.member_outbox_layout.lock().map_err(|_| denied())?;
        let mounted = mounted.as_ref().ok_or_else(denied)?;
        validate_mounted(self, root, mounted)?;
        Ok((
            fixed(self, true, member::MEMBER_OUTBOX_PARENT)?,
            mounted.parent.identity,
        ))
    }
}

fn validate_mounted(
    store: &ServerStore,
    root: &critical_namespaces::archive_bootstrap::MountedRoot,
    mounted: &MountedMemberOutboxLayout,
) -> Result<(), AppError> {
    mounted
        ._fee
        .validate(store.member_outbox_declaration_pool()?, member::MOUNTED_FEE)?;
    if mounted.negative.is_cancelled() || !Arc::ptr_eq(&mounted.mount, &store.registry_mount()) {
        return Err(denied());
    }
    require_root(store, root, mounted.capability)?;
    root_under_archive(store, root, &mounted.root)?;
    validate_directory_at(
        &fixed(store, true, member::MEMBER_OUTBOX_PARENT)?,
        &mounted.parent,
    )?;
    let path = fixed(store, true, member::MEMBER_OUTBOX_MARKER)?;
    held(&path, &mounted.marker, mounted.marker_id)?;
    let (file, raw) = read(&path, member::codec::MAX_MARKER_FRAME)?.ok_or_else(denied)?;
    if session_id(&file).map_err(io)? != mounted.marker_id
        || *blake3::hash(&raw).as_bytes() != mounted.marker_digest
    {
        return Err(denied());
    }
    member::codec::require_marker(&store.keys.db_key()?, mounted.capability, &raw)?;
    require_root(store, root, mounted.capability)?;
    root_under_archive(store, root, &mounted.root)?;
    // Rehash the SAME retained marker with bounded stack scratch: a same-ID, same-length
    // in-place change is not covered by the metadata/name checks above.
    let mut marker = &mounted.marker;
    marker.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut final_bytes = [0u8; member::codec::MAX_MARKER_FRAME + 1];
    let mut amount = 0;
    loop {
        let read = marker.read(&mut final_bytes[amount..]).map_err(io)?;
        if read == 0 {
            break;
        }
        amount += read;
        if amount > member::codec::MAX_MARKER_FRAME {
            return Err(denied());
        }
    }
    if *blake3::hash(&final_bytes[..amount]).as_bytes() != mounted.marker_digest {
        return Err(denied());
    }
    held(&path, &mounted.marker, mounted.marker_id)?;
    validate_directory_at(
        &fixed(store, true, member::MEMBER_OUTBOX_PARENT)?,
        &mounted.parent,
    )?;
    if mounted.negative.is_cancelled() {
        return Err(denied());
    }
    Ok(())
}
