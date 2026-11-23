//! Warm exact-file writer for the governed reservation. The Host owns the
//! original quiescent member lane and proves the signed invite journal before
//! advancing the SDK outcome or the signing records.
use super::super::outcome::{SealedGovernedAddOutcome, MAX_FRAME};
use super::*;

mod removal;
use crate::store::persistence::v2_text::head::TextPendingData;
use catcoms_storage::kept::session_id;
use catcoms_sync::owner_v2::{V2FounderAddOutcomeData, V2InstalledFounderAddData};
pub(in crate::store) use removal::PreparedGovernedRemoveFence;
use std::fs::OpenOptions;
use std::io::Write;

mod header;

/// File custody is cached immediately after creation and survives errors or
/// interruption. Retrying this owner can append only its own verified prefix.
pub(in crate::store) struct PreparedGovernedAddFence {
    frozen: Option<Zeroizing<Vec<u8>>>,
    entered_create: bool,
    request: PreparedSmallControl,
    fetch_verifier: Option<[u8; 32]>,
    entry: Option<PreparedSmallControl>,
    outcome: Option<PreparedOutcome>,
    header: Option<header::PreparedHeaderSigning>,
    other_paths: [PathBuf; 3],
    // The downstream Files and paths drain before the fence's physical lease.
    held: HeldGovernedAddFence,
    _fee: CatalogReadReservation,
}

struct PreparedSmallControl {
    frozen: Option<Zeroizing<Vec<u8>>>,
    entered_create: bool,
    original: Option<OpenedTextControl>,
    covered: bool,
}

impl PreparedSmallControl {
    fn new(frame: Zeroizing<Vec<u8>>) -> Self {
        Self {
            frozen: Some(frame),
            entered_create: false,
            original: None,
            covered: false,
        }
    }

    fn cover(
        &mut self,
        path: &Path,
        maximum: usize,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
    ) -> Result<(), AppError> {
        let (target, directory, id) = guard.target_directory_data();
        if !self.covered {
            ensure_original(
                path,
                &mut self.frozen,
                &mut self.entered_create,
                &mut self.original,
                maximum,
            )?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            fill_original(path, self.original.as_mut().ok_or_else(refused)?)?;
            crate::store::persistence::sync_directory(target).map_err(io)?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            self.covered = true;
        }
        self.require_covered(path)?;
        guard.validate_source_only()
    }

    fn require_covered(&mut self, path: &Path) -> Result<(), AppError> {
        if !self.covered {
            return Err(refused());
        }
        require_exact_held(path, self.original.as_mut().ok_or_else(refused)?)
    }

    fn covered_frame(&self) -> Result<&[u8], AppError> {
        self.original
            .as_ref()
            .filter(|_| self.covered)
            .map(|original| original.frame.as_slice())
            .ok_or_else(refused)
    }
}

/// Advance one held selector in the original Current directory. The caller
/// has already covered the matching staged file and immutable archive.
fn select_governed_head(
    guard: &V2TextCurrentGroupGuard<'_, '_>,
    stage: &Path,
    head_path: &Path,
    current: &mut OpenedTextControl,
    entered: &mut bool,
) -> Result<(), AppError> {
    let (target, directory, id) = guard.target_directory_data();
    if stage.parent() != Some(target) || head_path != fixed_child(target, "head.bin")?.as_path() {
        return Err(refused());
    }
    crate::store::persistence::v2_genesis_activation::require_directory_name(
        target, directory, id,
    )?;
    require_exact_held(head_path, current)?;
    guard.validate_source_only()?;
    *entered = true;
    fs::rename(stage, head_path).map_err(io)
}

struct PreparedOutcome {
    original: Option<OpenedTextControl>,
    sealed: SealedGovernedAddOutcome,
    entered_create: bool,
    covered: bool,
    header_intent: [u8; 32],
}

const WRITER_BYTES: usize = 3 * MAX_TEXT_PATH
    + 6 * FRAME_BYTES
    + 6 * ENTRY_BYTES
    + 6 * (request::MAX_BYTES + 1)
    + header::BUFFER_BYTES
    + 4 * std::mem::size_of::<PreparedGovernedAddFence>()
    + 1024;

impl PreparedGovernedAddFence {
    pub(in crate::store) fn has_prepared_outcome_data(&self) -> bool {
        self.outcome.is_some()
    }

    pub(in crate::store) fn later_parent_data(&self) -> Option<TextHeadIdData> {
        self.held.later.as_ref().map(|_| self.held.parent.1)
    }

    pub(in crate::store) fn require_parent_current(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        seed: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.held.require_parent(guard, seed)
    }

    /// This must run before entering Current, under the caller's authenticated
    /// original request and selected context. Retain the result across retries.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        Self::prepare_bound(store, source, selected, packet, rng, None)
    }

    pub(in crate::store) fn prepare_later(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        seed: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        (installed, fetch_verifier): ((TextHeadIdData, V2InstalledFounderAddData), [u8; 32]),
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        source.require_original_negative(store)?;
        let original = Box::new(HeldColdFounderAdd::capture(
            store,
            source.source(),
            *seed.head_data().context.spec_data().pin.as_bytes(),
        )?);
        original.require_installed_parent(source.source(), installed.0, installed.1)?;
        Self::prepare_bound(
            store,
            source,
            seed,
            packet,
            rng,
            Some((original, fetch_verifier)),
        )
    }

    fn prepare_bound(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        rng: &mut dyn CryptoRngCore,
        later: Option<(Box<HeldColdFounderAdd>, [u8; 32])>,
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
        let head = later.as_ref().map_or_else(
            || selected.head_id_data(),
            |(prior, _)| prior.writer_parent_data().1,
        );
        let data = GovernedAddFenceData::for_request(
            selected.head_data().context.id_data()?,
            head,
            packet,
        );
        let (later_parent, fetch_verifier) = match later {
            Some((parent, verifier)) => (Some(parent), Some(verifier)),
            None => (None, None),
        };
        let held = HeldGovernedAddFence::prepare_with_parent(
            store,
            source,
            selected,
            Some(data),
            later_parent,
        )?;
        let target = selected.physical_query_data().target;
        let other_paths = [
            fixed_child(target, "text.wal")?,
            fixed_child(target, "head.stage")?,
            fixed_child(target, "text.seed.stage.v1")?,
        ];
        let key = Zeroizing::new(store.keys.db_key()?);
        let frozen = data.seal_data(&key, rng)?;
        let request_frame = if let Some(verifier) = fetch_verifier {
            request::seal_with_fetch_verifier(&key, &frozen, packet, Some(verifier), rng)?
        } else {
            request::seal_data(&key, &frozen, packet, rng)?
        };
        let request = PreparedSmallControl::new(request_frame);
        held.resources
            .require_bound_original_negative(store, source)?;
        Ok(Self {
            frozen: Some(frozen),
            entered_create: false,
            held,
            other_paths,
            request,
            fetch_verifier,
            entry: None,
            outcome: None,
            header: None,
            _fee: fee,
        })
    }

    /// The original runtime alone calls this with its member lane and outbox
    /// held idle. This covers the fixed reservation and its exact request.
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
            require_absent(&self.held.request_path)?;
            require_absent(&self.held.entry_path)?;
            require_absent(&self.held.outcome_path)?;
            self.held.require_header_stages_absent()?;
            ensure_original(
                &self.held.path,
                &mut self.frozen,
                &mut self.entered_create,
                &mut self.held.original,
                FRAME_BYTES,
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
        if !self.request.covered {
            self.require_covered_fence(guard, selected)?;
            require_absent(&self.held.entry_path)?;
            require_absent(&self.held.outcome_path)?;
            self.held.require_header_stages_absent()?;
            self.request
                .cover(&self.held.request_path, request::MAX_BYTES, guard)?;
        }
        self.require_covered(guard, selected)
    }

    pub(in crate::store) fn require_covered(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered_fence(guard, selected)?;
        self.request.require_covered(&self.held.request_path)?;
        guard.validate_source_only()
    }

    fn require_covered_fence(
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
        if !self.held.authenticated || !self.request.covered {
            return Err(refused());
        }
        if self.entry.is_some() {
            return Ok(());
        }
        let raw = &self.held.original.as_ref().ok_or_else(refused)?.frame;
        let data = GovernedAddSdkEntryData::from_fence_data(raw)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let frozen = data.seal_data(&key, rng)?;
        self.entry = Some(PreparedSmallControl::new(frozen));
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
        let entry = self.entry.as_mut().ok_or_else(refused)?;
        if !entry.covered {
            require_absent(&self.held.outcome_path)?;
            self.held.require_header_stages_absent()?;
        }
        entry.cover(&self.held.entry_path, ENTRY_BYTES, guard)?;
        self.require_covered_sdk_entry(guard, selected)
    }

    pub(in crate::store) fn require_covered_sdk_entry(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered(guard, selected)?;
        let entry = self.entry.as_mut().ok_or_else(refused)?;
        entry.require_covered(&self.held.entry_path)?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    /// Bind the actual SDK outcome to this original warm entry outside all
    /// writers. Keep the first prepared frame through any late refusal.
    pub(in crate::store) fn prepare_outcome(
        &mut self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
        packet: &V2JoinRequestPacketData<'_>,
        outcome: &V2FounderAddOutcomeData<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.outcome.is_some() {
            return Err(refused());
        }
        self.held
            .resources
            .require_same_original(selected.resources())?;
        self.held
            .resources
            .require_bound_original_negative(store, source)?;
        let parent = self.held.writer_parent(selected)?;
        let expected = GovernedAddFenceData::for_request(parent.context, parent.head, packet);
        if Some(expected) != self.held.expected
            || outcome.unsigned_data().header_spec_data().parent != parent.header
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
        let sealed = SealedGovernedAddOutcome::prepare(store, raw, outcome, rng)?;
        self.outcome = Some(PreparedOutcome {
            original: None,
            sealed,
            entered_create: false,
            covered: false,
            header_intent: outcome.unsigned_data().intent_commitment_data(),
        });
        self.held
            .resources
            .require_bound_original_negative(store, source)
    }

    /// Cover the outcome using only this writer's actual held Entry and exact
    /// presealed bytes. A retry cannot reseal or adopt another file.
    pub(in crate::store) fn cover_outcome(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered_sdk_entry(guard, selected)?;
        guard.require_text_control_io_quiescent()?;
        let (target, directory, id) = guard.target_directory_data();
        let outcome = self.outcome.as_mut().ok_or_else(refused)?;
        let entry = self
            .entry
            .as_ref()
            .and_then(|entry| entry.original.as_ref())
            .ok_or_else(refused)?;
        outcome.sealed.require_entry(&entry.frame)?;
        if !outcome.covered {
            self.held.require_header_stages_absent()?;
            ensure_original(
                &self.held.outcome_path,
                &mut outcome.sealed.raw,
                &mut outcome.entered_create,
                &mut outcome.original,
                MAX_FRAME,
            )?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            fill_original(
                &self.held.outcome_path,
                outcome.original.as_mut().ok_or_else(refused)?,
            )?;
            crate::store::persistence::sync_directory(target).map_err(io)?;
            crate::store::persistence::v2_genesis_activation::require_directory_name(
                target, directory, id,
            )?;
            outcome.covered = true;
        }
        self.require_covered_outcome(guard, selected)
    }

    pub(in crate::store) fn require_covered_outcome(
        &mut self,
        guard: &V2TextCurrentGroupGuard<'_, '_>,
        selected: &SelectedV2TextHead,
    ) -> Result<(), AppError> {
        self.require_covered_sdk_entry(guard, selected)?;
        let outcome = self.outcome.as_mut().ok_or_else(refused)?;
        if !outcome.covered {
            return Err(refused());
        }
        require_exact_held(
            &self.held.outcome_path,
            outcome.original.as_mut().ok_or_else(refused)?,
        )?;
        let (target, directory, id) = guard.target_directory_data();
        crate::store::persistence::v2_genesis_activation::require_directory_name(
            target, directory, id,
        )?;
        guard.validate_source_only()
    }

    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn prepare_recovery_read_for_test(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        selected: &SelectedV2TextHead,
    ) -> Result<HeldGovernedAddFence, AppError> {
        HeldGovernedAddFence::prepare(store, source, selected, None)
    }
}

fn ensure_original(
    path: &Path,
    frozen: &mut Option<Zeroizing<Vec<u8>>>,
    entered: &mut bool,
    original: &mut Option<OpenedTextControl>,
    maximum: usize,
) -> Result<(), AppError> {
    if original.is_some() {
        return Ok(());
    }
    if *entered
        || frozen
            .as_ref()
            .is_none_or(|raw| raw.is_empty() || raw.len() > maximum)
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
