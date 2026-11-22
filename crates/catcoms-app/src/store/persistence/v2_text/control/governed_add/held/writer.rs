//! Warm exact-file writer for the governed reservation. The future Host must
//! own the original quiescent member lane and prove the signed invite journal
//! before calling it. No desktop, network or SDK stage invokes it yet.
use super::*;
use crate::store::persistence::v2_text::head::TextPendingData;
use catcoms_storage::kept::session_id;
use std::fs::{self, OpenOptions};
use std::io::Write;

/// File custody is cached immediately after creation and survives errors or
/// interruption. Retrying this owner can append only its own verified prefix.
pub(in crate::store) struct PreparedGovernedAddFence {
    frozen: Option<Zeroizing<Vec<u8>>>,
    entered_create: bool,
    held: HeldGovernedAddFence,
    other_paths: [PathBuf; 3],
    _fee: CatalogReadReservation,
}

const WRITER_BYTES: usize = 3 * MAX_TEXT_PATH
    + 6 * FRAME_BYTES
    + 4 * std::mem::size_of::<PreparedGovernedAddFence>()
    + 1024;

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
    if *entered || frozen.as_ref().is_none_or(|raw| raw.len() != FRAME_BYTES) {
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
    if prefix < FRAME_BYTES {
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
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refused()),
        }
    }
    Ok(())
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests;
