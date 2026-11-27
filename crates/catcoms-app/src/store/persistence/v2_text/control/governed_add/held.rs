//! Hold and revalidate the fixed governed reservation under actual Current.
//! The read path authenticates an existing file. It cannot adopt an owner,
//! complete a partial write, enter MLS, or return a covering save receipt.
use super::archive::recovery::HeldColdFounderAdd;
use super::governing_record;
use super::header_signing::{
    ATTEMPT_BYTES, ATTEMPT_NAME, RAW_BYTES, RAW_NAME, WELCOME_ATTEMPT_BYTES, WELCOME_ATTEMPT_NAME,
    WELCOME_RAW_BYTES, WELCOME_RAW_NAME,
};
use super::outcome::OUTCOME_NAME;
use super::request;
use super::response;
use super::sdk_entry::{GovernedAddSdkEntryData, ENTRY_BYTES, SDK_ENTRY_NAME};
use super::selected_head;
use super::welcome_intent;
use super::*;
use crate::store::critical_namespaces::current_layout::V2TextCurrentGroupGuard;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::persistence::v2_text::capture::SelectedV2TextHead;
use crate::store::persistence::v2_text::io::{
    fixed_child, governed_add_successor_child, read_control, require_named, OpenedTextControl,
    MAX_TEXT_PATH,
};
use crate::store::persistence::v2_text::V2TextReadResources;
use crate::store::persistence::GenesisSourceDataRef;
use crate::store::ServerStore;
use catcoms_replication::chat::{cold_manifest::ArchiveHeadId, owner_header::OwnerHeaderId};
use catcoms_storage::VaultPhysicalWorkLease;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(super) const STAGE: &str = "text.wal.stage.v1";
const FILE_COMPARE_BYTES: usize = 8192;
pub(super) const ADD_STAGES: [&str; 12] = [
    STAGE,
    request::NAME,
    SDK_ENTRY_NAME,
    OUTCOME_NAME,
    ATTEMPT_NAME,
    RAW_NAME,
    governing_record::NAME,
    welcome_intent::NAME,
    WELCOME_ATTEMPT_NAME,
    WELCOME_RAW_NAME,
    response::NAME,
    selected_head::NAME,
];
pub(super) const REMOVE_STAGES: [&str; 4] = [
    "text.remove.sdk-entry.v1",
    "text.remove.outcome.v1",
    "text.remove.header-attempt.v1",
    "text.remove.head-stage.v1",
];

pub(super) fn add_stage_path(
    target: &Path,
    name: &'static str,
    later: Option<TextHeadIdData>,
) -> Result<PathBuf, AppError> {
    if let Some(parent) = later {
        if !ADD_STAGES.contains(&name) {
            return Err(refused());
        }
        governed_add_successor_child(target, name, parent.as_bytes())
    } else {
        fixed_child(target, name)
    }
}

pub(super) fn require_successor_stages_absent(
    target: &Path,
    selected: TextHeadIdData,
) -> Result<(), AppError> {
    for name in ADD_STAGES {
        require_absent(&add_stage_path(target, name, Some(selected))?)?;
    }
    for name in REMOVE_STAGES {
        require_absent(&governed_add_successor_child(
            target,
            name,
            selected.as_bytes(),
        )?)?;
    }
    Ok(())
}

// Read-only inspectors exercise interrupted stages without turning a cold
// observation into authority to resume MLS or signing.
#[cfg(all(test, any(windows, target_os = "linux")))]
mod header_read;
#[cfg(all(test, any(windows, target_os = "linux")))]
mod outcome_read;
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(in crate::store) use header_read::{
    GovernedAddHeaderRecoveryState, GovernedAddWelcomeRecoveryState,
};
mod writer;
pub(super) use writer::select_named_head;
pub(in crate::store) use writer::PreparedGovernedRemoveFence;
pub(in crate::store) use writer::{PreparedColdGovernedAdd, PreparedGovernedAddFence};

/// An actual bounded, read-only File capture, tied to the exact selected
/// parent through its read-resource association. All file and path custody
/// leaves before the physical lease and the dedicated buffer reservation.
pub(in crate::store) struct HeldGovernedAddFence {
    path: PathBuf,
    request_path: PathBuf,
    entry_path: PathBuf,
    outcome_path: PathBuf,
    header_attempt_path: PathBuf,
    raw_header_path: PathBuf,
    governing_path: PathBuf,
    welcome_paths: WelcomeControlPaths,
    original: Option<OpenedTextControl>,
    #[cfg(all(test, any(windows, target_os = "linux")))]
    request: Option<ObservedRequest>,
    #[cfg(all(test, any(windows, target_os = "linux")))]
    entry: Option<ObservedSdkEntry>,
    #[cfg(all(test, any(windows, target_os = "linux")))]
    outcome: Option<outcome_read::ObservedOutcome>,
    #[cfg(all(test, any(windows, target_os = "linux")))]
    header: Option<header_read::ObservedHeader>,
    parent: (TextContextIdData, TextHeadIdData),
    expected: Option<GovernedAddFenceData>,
    authenticated: bool,
    later: Option<Box<HeldColdFounderAdd>>,
    resources: V2TextReadResources,
    // The independent buffer payment includes the bounded path and file read,
    // copied AEAD input/plaintext and exact same-handle EOF comparison.
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

#[cfg(all(test, any(windows, target_os = "linux")))]
enum ObservedSdkEntry {
    Absent,
    Present(OpenedTextControl),
}

#[cfg(all(test, any(windows, target_os = "linux")))]
struct ObservedRequest {
    original: Option<OpenedTextControl>,
    packet: Option<Zeroizing<Vec<u8>>>,
}

struct WelcomeControlPaths {
    intent: PathBuf,
    attempt: PathBuf,
    raw: PathBuf,
    response: PathBuf,
    head_stage: PathBuf,
}

struct WriterParent<'a> {
    context: TextContextIdData,
    head: TextHeadIdData,
    archive: ArchiveHeadId,
    header: OwnerHeaderId,
    head_frame: &'a [u8],
    archive_frame: &'a [u8],
}

impl WelcomeControlPaths {
    fn require_absent(&self) -> Result<(), AppError> {
        require_absent(&self.intent)?;
        require_absent(&self.attempt)?;
        require_absent(&self.raw)?;
        self.require_after_raw_absent()
    }

    fn require_after_raw_absent(&self) -> Result<(), AppError> {
        require_absent(&self.response)?;
        require_absent(&self.head_stage)
    }
}

/// Historical disk status for the exact reservation. Even ReservationOnly
/// requires separate proofs of all downstream absences before disposal.
#[cfg(all(test, any(windows, target_os = "linux")))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum GovernedAddRecoveryState {
    ReservationOnly,
    PossiblyEntered,
}

const FENCE_READ_BYTES: usize = 12 * MAX_TEXT_PATH
    + 6 * FRAME_BYTES
    + 6 * ENTRY_BYTES
    + 6 * (ATTEMPT_BYTES + RAW_BYTES + WELCOME_ATTEMPT_BYTES + WELCOME_RAW_BYTES)
    + 10 * governing_record::MAX_BYTES
    + 8 * welcome_intent::MAX_BYTES
    + 6 * (request::MAX_BYTES + 1)
    + FILE_COMPARE_BYTES
    + 4 * std::mem::size_of::<HeldGovernedAddFence>()
    + 1024;

impl HeldGovernedAddFence {
    fn writer_parent<'a>(
        &'a self,
        seed: &'a SelectedV2TextHead,
    ) -> Result<WriterParent<'a>, AppError> {
        if let Some(later) = self.later.as_deref() {
            let (context, head, archive, header, head_frame) = later.writer_parent_data();
            Ok(WriterParent {
                context,
                head,
                archive,
                header,
                head_frame,
                archive_frame: &[],
            })
        } else {
            let physical = seed.physical_query_data();
            Ok(WriterParent {
                context: seed.head_data().context.id_data()?,
                head: seed.head_id_data(),
                archive: seed.head_data().archive,
                header: seed.archive_data().spec_data().header.id(),
                head_frame: physical.head_frame,
                archive_frame: physical.archive_frame,
            })
        }
    }

    /// Prepare outside Current. Keep this whole receiver outside the writers
    /// through capture and any late refusal, so its file and lease drain there.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store::persistence::v2_text) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        expected: Option<GovernedAddFenceData>,
    ) -> Result<Self, AppError> {
        Self::prepare_with_parent(store, source, selected, expected, None)
    }

    fn prepare_with_parent(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        expected: Option<GovernedAddFenceData>,
        later: Option<Box<HeldColdFounderAdd>>,
    ) -> Result<Self, AppError> {
        let parent = (
            selected.head_data().context.id_data()?,
            selected.head_id_data(),
        );
        let parent = if let Some(later) = later.as_deref() {
            let (context, head, _, _, _) = later.writer_parent_data();
            if context != parent.0 {
                return Err(refused());
            }
            (context, head)
        } else {
            parent
        };
        if let Some(expected) = expected {
            expected.require_parent(parent.0, parent.1)?;
        }
        let resources = selected.resources().clone();
        resources.require_bound_original_negative(store, source)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(FENCE_READ_BYTES)?;
        fee.validate(pool, FENCE_READ_BYTES)?;
        let lease = store._session.retain_for_physical_work();
        let target = selected.physical_query_data().target;
        let later_head = later.as_ref().map(|_| parent.1);
        let path = |name| add_stage_path(target, name, later_head);
        let welcome_paths = WelcomeControlPaths {
            intent: path(welcome_intent::NAME)?,
            attempt: path(WELCOME_ATTEMPT_NAME)?,
            raw: path(WELCOME_RAW_NAME)?,
            response: path(response::NAME)?,
            head_stage: path(selected_head::NAME)?,
        };
        resources.require_bound_original_negative(store, source)?;
        Ok(Self {
            path: path(STAGE)?,
            request_path: path(request::NAME)?,
            entry_path: path(SDK_ENTRY_NAME)?,
            outcome_path: path(OUTCOME_NAME)?,
            header_attempt_path: path(ATTEMPT_NAME)?,
            raw_header_path: path(RAW_NAME)?,
            governing_path: path(governing_record::NAME)?,
            welcome_paths,
            original: None,
            #[cfg(all(test, any(windows, target_os = "linux")))]
            request: None,
            #[cfg(all(test, any(windows, target_os = "linux")))]
            entry: None,
            #[cfg(all(test, any(windows, target_os = "linux")))]
            outcome: None,
            #[cfg(all(test, any(windows, target_os = "linux")))]
            header: None,
            parent,
            expected,
            authenticated: false,
            later,
            resources,
            _lease: lease,
            _fee: fee,
        })
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn capture(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if self.original.is_some() {
            return Err(refused());
        }
        self.require_parent(guard, selected)?;
        self.original = Some(read_control(&self.path, FRAME_BYTES)?);
        self.request = Some(ObservedRequest {
            original: header_read::read_stage(&self.request_path, request::MAX_BYTES)?,
            packet: None,
        });
        self.entry = Some(match fs::symlink_metadata(&self.entry_path) {
            Ok(_) => ObservedSdkEntry::Present(read_control(&self.entry_path, ENTRY_BYTES)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                require_absent(&self.outcome_path)?;
                self.require_header_stages_absent()?;
                ObservedSdkEntry::Absent
            }
            Err(error) => return Err(io(error)),
        });
        guard.validate_source_only()
    }

    /// Outside Current. MAC success becomes usable only after a fresh check
    /// of the same original, named bytes and selected parent.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn authenticate(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        self.resources
            .require_bound_original_negative(store, source)?;
        let original = self.original.as_ref().ok_or_else(refused)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let data = GovernedAddFenceData::open_data(&key, &original.frame)?;
        data.require_parent(self.parent.0, self.parent.1)?;
        if self.expected.is_some_and(|expected| data != expected) {
            return Err(refused());
        }
        let request = self.request.as_mut().ok_or_else(refused)?;
        if let Some(capture) = &request.original {
            request.packet = Some(request::open_data(&key, &original.frame, &capture.frame)?);
        }
        if let ObservedSdkEntry::Present(entry) = self.entry.as_ref().ok_or_else(refused)? {
            GovernedAddSdkEntryData::from_fence_data(&original.frame)?
                .require_fence_data(&key, &entry.frame)?;
        }
        self.resources
            .require_bound_original_negative(store, source)?;
        self.authenticated = true;
        self.expected = Some(data);
        Ok(())
    }

    pub(in crate::store::persistence::v2_text) fn require_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        if !self.authenticated {
            return Err(refused());
        }
        self.require_parent(guard, selected)?;
        require_exact_held(&self.path, self.original.as_mut().ok_or_else(refused)?)?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn require_recovery_state(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<GovernedAddRecoveryState, AppError> {
        self.require_request_current(guard, selected)?;
        let state = match self.entry.as_mut().ok_or_else(refused)? {
            ObservedSdkEntry::Absent => {
                require_absent(&self.entry_path)?;
                require_absent(&self.outcome_path)?;
                self.require_header_stages_absent()?;
                GovernedAddRecoveryState::ReservationOnly
            }
            ObservedSdkEntry::Present(entry) => {
                require_exact_held(&self.entry_path, entry)?;
                GovernedAddRecoveryState::PossiblyEntered
            }
        };
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(state)
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn require_request_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<Option<&[u8]>, AppError> {
        self.require_current(guard, selected)?;
        let request = self.request.as_mut().ok_or_else(refused)?;
        header_read::require_stage(&self.request_path, request.original.as_mut())?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()?;
        Ok(request.packet.as_ref().map(|bytes| bytes.as_slice()))
    }

    fn require_parent(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        let (target, _, _) = guard.target_directory_data();
        if self.path.parent() != Some(target) {
            return Err(refused());
        }
        self.resources.require_same_original(selected.resources())?;
        self.resources.require_same_original(guard.resources())?;
        if let Some(later) = self.later.as_mut() {
            let (context, head, _, _, _) = later.writer_parent_data();
            if (context, head) != self.parent
                || context != selected.head_data().context.id_data()?
            {
                return Err(refused());
            }
            return later.require_writer_parent_current(guard);
        }
        if self.parent
            != (
                selected.head_data().context.id_data()?,
                selected.head_id_data(),
            )
        {
            return Err(refused());
        }
        selected.validate_under_current_guard(guard)?;
        guard.require_governed_native_invite_parent()
    }

    fn require_header_stages_absent(&self) -> Result<(), AppError> {
        require_absent(&self.header_attempt_path)?;
        require_absent(&self.raw_header_path)?;
        require_absent(&self.governing_path)?;
        self.welcome_paths.require_absent()
    }
}

pub(super) fn require_exact_held(
    path: &Path,
    original: &mut OpenedTextControl,
) -> Result<(), AppError> {
    require_named(path, original)?;
    if read_matching_prefix(&mut original.file, &original.frame)? != original.frame.len() {
        return Err(refused());
    }
    require_named(path, original)
}

fn read_matching_prefix(file: &mut std::fs::File, expected: &[u8]) -> Result<usize, AppError> {
    if expected.is_empty() {
        return Err(refused());
    }
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.len() > expected.len() as u64 {
        return Err(refused());
    }
    let length = metadata.len() as usize;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    // A larger outcome uses the same EOF and held-file proof, with constant
    // stack storage and no allocation under the Current writer.
    let mut frame = [0; FILE_COMPARE_BYTES];
    for expected_chunk in expected[..length].chunks(frame.len()) {
        let chunk = &mut frame[..expected_chunk.len()];
        file.read_exact(chunk).map_err(io)?;
        if chunk != expected_chunk {
            return Err(refused());
        }
    }
    let mut trailer = [0];
    loop {
        match file.read(&mut trailer) {
            Ok(0) => break,
            Ok(_) => return Err(refused()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io(error)),
        }
    }
    Ok(length)
}

fn io(error: std::io::Error) -> AppError {
    AppError::Io(error.to_string())
}

fn require_absent(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err(refused()),
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
