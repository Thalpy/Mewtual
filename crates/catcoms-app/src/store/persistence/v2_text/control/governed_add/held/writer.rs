//! Warm exact-file writer for the governed reservation. The future Host must
//! own the original quiescent member lane and prove the signed invite journal
//! before calling it. No desktop, network or SDK stage invokes it yet.
use super::*;
use crate::store::persistence::v2_text::control::governed_add::outcome::PreparedGovernedAddOutcome;
use crate::store::persistence::v2_text::head::TextPendingData;
use catcoms_storage::kept::session_id;
use catcoms_sync::owner_v2::V2FounderAddOutcomeData;
use std::fs::OpenOptions;
use std::io::Write;

/// File custody is cached immediately after creation and survives errors or
/// interruption. Retrying this owner can append only its own verified prefix.
pub(in crate::store) struct PreparedGovernedAddFence {
    frozen: Option<Zeroizing<Vec<u8>>>,
    entered_create: bool,
    entry: Option<PreparedSdkEntry>,
    other_paths: [PathBuf; 3],
    // The entry's File and path drain before the fence's physical lease.
    held: HeldGovernedAddFence,
    _fee: CatalogReadReservation,
}

struct PreparedSdkEntry {
    frozen: Option<Zeroizing<Vec<u8>>>,
    entered_create: bool,
    original: Option<OpenedTextControl>,
    covered: bool,
}

const WRITER_BYTES: usize = 3 * MAX_TEXT_PATH
    + 6 * FRAME_BYTES
    + 6 * ENTRY_BYTES
    + 4 * std::mem::size_of::<PreparedGovernedAddFence>()
    + 1024;
const _: () = assert!(ENTRY_BYTES < FRAME_BYTES);

impl PreparedGovernedAddFence {
    /// This must run before entering Current, under the caller's authenticated
    /// original request and selected context. Retain the result across retries.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        if selected.head_data().generation != 1
            || selected.head_data().pending != TextPendingData::None
        {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(WRITER_BYTES)?;
        fee.validate(pool, WRITER_BYTES)?;
        let data = GovernedAddFenceData::for_request(
            selected.head_data().context.id_data()?,
            selected.head_id_data(),
            packet,
        );
        let held = HeldGovernedAddFence::prepare(store, source, selected, data)?;
        let target = selected.physical_query_data().target;
        let other_paths = [
            fixed_child(target, "text.wal")?,
            fixed_child(target, "head.stage")?,
            fixed_child(target, "text.seed.stage.v1")?,
        ];
        let key = Zeroizing::new(store.keys.db_key()?);
        let frozen = Some(data.seal_data(&key, rng)?);
        held.resources
            .require_bound_original_negative(store, source)?;
        Ok(Self {
            frozen,
            entered_create: false,
            held,
            other_paths,
            entry: None,
            _fee: fee,
        })
    }

    /// The original runtime alone calls this with its member lane and outbox
    /// held idle. This covers the fixed reservation only, never an SDK entry.
    pub(in crate::store) fn cover(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.held.require_parent(guard, selected)?;
        guard.require_text_control_io_quiescent()?;
        let (target, directory, id) = guard.target_directory_data();
        // An unrelated or ambiguous earlier text transaction always wins.
        require_absences(&self.other_paths)?;

        if !self.held.authenticated {
            require_absent(&self.held.entry_path)?;
            ensure_original(
                &self.held.path,
                &mut self.frozen,
                &mut self.entered_create,
                &mut self.held.original,
            )?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            let original = self.held.original.as_mut().ok_or_else(refused)?;
            fill_original(&self.held.path, original)?;
            crate::store::persistence::sync_directory(target).map_err(io)?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            // After this barrier the original is complete. A failed recheck
            // or a later retry cannot extend or rewrite even a proper prefix.
            self.held.authenticated = true;
        }
        self.require_covered(guard, selected)
    }

    pub(in crate::store) fn require_covered(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.held.require_current(guard, selected)?;
        require_absences(&self.other_paths)?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    /// Cache one entry record outside writers. It commits the actual covered
    /// fence ciphertext and cannot be moved to a resealed reservation.
    pub(in crate::store) fn prepare_sdk_entry(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        if !self.held.authenticated {
            return Err(refused());
        }
        if self.entry.is_some() {
            return Ok(());
        }
        let raw = &self.held.original.as_ref().ok_or_else(refused)?.frame;
        let data = GovernedAddSdkEntryData::from_fence_data(raw)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let frozen = data.seal_data(&key, rng)?;
        self.entry = Some(PreparedSdkEntry {
            frozen: Some(frozen),
            entered_create: false,
            original: None,
            covered: false,
        });
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    /// Record possible SDK entry before the actual SDK can run. A retry may
    /// finish its held prefix only until it has covered the entry once.
    pub(in crate::store) fn cover_sdk_entry(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered(guard, selected)?;
        guard.require_text_control_io_quiescent()?;
        let (target, directory, id) = guard.target_directory_data();
        let entry = self.entry.as_mut().ok_or_else(refused)?;
        if !entry.covered {
            ensure_original(
                &self.held.entry_path,
                &mut entry.frozen,
                &mut entry.entered_create,
                &mut entry.original,
            )?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            fill_original(
                &self.held.entry_path,
                entry.original.as_mut().ok_or_else(refused)?,
            )?;
            crate::store::persistence::sync_directory(target).map_err(io)?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            entry.covered = true;
        }
        self.require_covered_sdk_entry(guard, selected)
    }

    pub(in crate::store) fn require_covered_sdk_entry(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered(guard, selected)?;
        let entry = self.entry.as_mut().ok_or_else(refused)?;
        if !entry.covered {
            return Err(refused());
        }
        require_exact_held(
            &self.held.entry_path,
            entry.original.as_mut().ok_or_else(refused)?,
        )?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    /// Bind the actual SDK outcome to this original warm entry. Root must
    /// retain the returned frame before writing it and independently reprove
    /// the same held controls under Current for the physical save.
    pub(in crate::store::persistence::v2_text) fn prepare_outcome(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<PreparedGovernedAddOutcome, AppError> {
        self.held
            .resources
            .require_same_original(selected.resources())?;
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let expected = GovernedAddFenceData::for_request(
            selected.head_data().context.id_data()?,
            selected.head_id_data(),
            packet,
        );
        if expected != self.held.expected
            || outcome.unsigned_data().header_spec_data().parent
                != selected.archive_data().spec_data().header.id()
        {
            return Err(refused());
        }
        outcome.require_request_data(packet.founder_add_data())?;
        let entry = self
            .entry
            .as_ref()
            .filter(|entry| entry.covered)
            .ok_or_else(refused)?;
        let raw = &entry.original.as_ref().ok_or_else(refused)?.frame;
        let result = PreparedGovernedAddOutcome::prepare(store, raw, outcome, rng)?;
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        Ok(result)
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn prepare_recovery_read_for_test(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
    ) -> Result<HeldGovernedAddFence, AppError> {
        HeldGovernedAddFence::prepare(store, source, selected, self.held.expected)
    }
}

fn ensure_original(
    path: &Path,
    frozen: &mut Option<Zeroizing<Vec<u8>>>,
    entered: &mut bool,
    original: &mut Option<OpenedTextControl>,
) -> Result<(), AppError> {
    if original.is_some() {
        return Ok(());
    }
    if *entered
        || frozen
            .as_ref()
            .is_none_or(|raw| raw.is_empty() || raw.len() > FRAME_BYTES)
    {
        return Err(refused());
    }
    *entered = true;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    let id = session_id(&file).map_err(io)?;
    let frame = frozen
        .take()
        .expect("validated frozen fence before file creation");
    *original = Some(OpenedTextControl { file, id, frame });
    Ok(())
}

fn fill_original(path: &Path, original: &mut OpenedTextControl) -> Result<(), AppError> {
    require_named(path, original)?;
    let prefix = read_matching_prefix(&mut original.file, &original.frame)?;
    require_named(path, original)?;
    if prefix < original.frame.len() {
        original
            .file
            .seek(SeekFrom::Start(prefix as u64))
            .map_err(io)?;
        original
            .file
            .write_all(&original.frame[prefix..])
            .map_err(io)?;
    }
    original.file.sync_all().map_err(io)?;
    require_exact_held(path, original)
}

fn require_absences(paths: &[PathBuf; 3]) -> Result<(), AppError> {
    for path in paths {
        require_absent(path)?;
    }
    Ok(())
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
