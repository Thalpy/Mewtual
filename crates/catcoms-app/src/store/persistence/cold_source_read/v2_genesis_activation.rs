//! Genesis activation worker paired with the physical namespace and distinct
//! vacant/initializing admission consumers. This does not enable operational Chat V2.
//! Parsed metadata never constructs the saved receipts below. The actual original held
//! objects and file/parent barriers do; equal bytes in a replacement object do not suffice.
use super::*;
use crate::store::critical_namespaces::{
    archive_current_codec as codec,
    current_layout::{
        InitializingV2GenesisGroupGuard, OriginalV2GenesisGroupObservation,
        PreparedV2GenesisGroupNamespace, SavedV2GroupDirectory, VacantV2GenesisGroupGuard,
    },
};
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::persistence::{ensure_new_founder_control, verify_archive_held_name};
use codec::{SavedSourceExpectationData, V2GroupPinId};

// Paired with the complete cold genesis Current2 caller. The cold receipt remains
// private and independently authenticated; a warm Saved receipt is never reconstructed.
mod cold_capture;
pub(in crate::store) mod cold_writer;
mod genesis_inventory;
#[cfg(test)]
pub(in crate::store) use cold_capture::{ColdV2GenesisActivationSeed, COLD_CAPTURE_METADATA_FEE};
pub(in crate::store) use cold_capture::{
    ColdV2GenesisCurrentHead, ColdV2GenesisReadResources, PreparedColdV2GenesisCapture,
};

const SEED: &str = "activation.seed";
const PIN: &str = "pin.bin";
const HEAD: &str = "head.bin";
const STAGE: &str = "head.stage";

/// This invoice is distinct from SDK/provider/read resources and the large temporary scope
/// serialization allowance. All immutable candidates and BOTH saved receipts share ONE
/// actual metadata debit. No key, Store, task sender, SDK graph or source owner lives here.
struct ActivationCandidates {
    original_read: OriginalBootstrapReadObservation,
    expected: SavedSourceExpectationData,
    pin: Zeroizing<Vec<u8>>,
    head: Zeroizing<Vec<u8>>,
    seed: Zeroizing<Vec<u8>>,
    // LAST: exact candidates, contexts and observer control-block ownership drain first.
    _fee: critical_namespaces::read_pool::CatalogReadReservation,
}

/// Owning warm work, retained by the concrete Current initializer across every refusal. A
/// newly created File is stored BEFORE its first fallible identity/write operation. A retry
/// never reseals, adopts an unowned partial file or reruns the source SDK/read producer.
pub(in crate::store) struct PreparedV2GenesisActivation {
    seed_file: Option<critical_namespaces::pending_join::CreatedStage>,
    pin_file: Option<critical_namespaces::pending_join::CreatedStage>,
    head_file: Option<critical_namespaces::pending_join::CreatedStage>,
    saved_seed: Option<Arc<SavedV2GenesisActivationSeed>>,
    saved_head: Option<Arc<SavedV2GenesisHead>>,
    // LAST: the prepared owner can be the final metadata payer while unfinished Files drain.
    candidates: Arc<ActivationCandidates>,
}

/// Actual original activation seed after exact file+group-parent barriers. No decoder, public
/// field constructor, matching scalar or Saved boolean can issue this receipt.
pub(in crate::store) struct SavedV2GenesisActivationSeed {
    group: OriginalV2GenesisGroupObservation,
    directory: Arc<SavedV2GroupDirectory>,
    file: critical_namespaces::pending_join::CreatedStage,
    candidates: Arc<ActivationCandidates>,
}

/// Actual immutable pin and original local head, both synchronized through their respective
/// parent barriers. The scope finisher must retain THIS receipt before Current2 mutation.
pub(in crate::store) struct SavedV2GenesisHead {
    seed: Arc<SavedV2GenesisActivationSeed>,
    pin: critical_namespaces::pending_join::CreatedStage,
    head: critical_namespaces::pending_join::CreatedStage,
    writer: V2WriterObservation,
    candidates: Arc<ActivationCandidates>,
}

macro_rules! sanitized_debug {
    ($($ty:ident),* $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )*};
}
sanitized_debug!(
    PreparedV2GenesisActivation,
    SavedV2GenesisActivationSeed,
    SavedV2GenesisHead
);

impl PreparedV2GenesisActivation {
    /// Compare the retained callback result, never decoded data or a logical head ID. This
    /// is only a cached original association; the caller must still prove physical Current.
    pub(in crate::store) fn require_same_saved_head(
        &self,
        saved: &Arc<SavedV2GenesisHead>,
    ) -> Result<(), AppError> {
        if !self
            .saved_head
            .as_ref()
            .is_some_and(|ours| Arc::ptr_eq(ours, saved))
        {
            return Err(critical_namespaces::refused());
        }
        Ok(())
    }
    /// Only the real absent-row guard may create the independent first activation seed.
    /// It is saved BEFORE any scope insertion. The later initializing guard cannot be
    /// substituted here, and this method never constructs its own admission or directory.
    pub(in crate::store) fn save_seed(
        &mut self,
        guard: &VacantV2GenesisGroupGuard<'_, '_>,
    ) -> Result<Arc<SavedV2GenesisActivationSeed>, AppError> {
        self.candidates
            .retains_original(guard.store(), guard.source())?;
        guard.validate_source_only()?;
        if let Some(saved) = &self.saved_seed {
            saved.validate_under_archive_root(
                guard.store(),
                guard.source(),
                guard.task(),
                guard.archive_root(),
            )?;
            return Ok(saved.clone());
        }
        let (base, _, _) = guard.directory().directory_data();
        let path = child(base, SEED)?;
        // Repeat originals after every guard/path/resource wait and immediately before the
        // synchronous effect. The runtime/pins/canonical/Root guards serialize app writes.
        guard.validate_source_only()?;
        guard.directory().validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.task(),
            guard.archive_root(),
        )?;
        ensure_new_founder_control(
            guard.store(),
            &path,
            &self.candidates.seed,
            &mut self.seed_file,
            180,
            181,
        )?;
        crate::store::persistence::sync_directory(base).map_err(io)?;
        exact_held_frame(
            &path,
            self.seed_file
                .as_ref()
                .ok_or_else(critical_namespaces::refused)?,
            &self.candidates.seed,
        )?;
        guard.directory().validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.task(),
            guard.archive_root(),
        )?;
        // Cache before returning: losing this result cannot make retry create anew. The
        // original held File moves into the saved proof, never a newly opened equal object.
        let saved = Arc::new(SavedV2GenesisActivationSeed {
            group: guard.task().observe_original(),
            directory: guard.directory().clone(),
            file: self
                .seed_file
                .take()
                .ok_or_else(critical_namespaces::refused)?,
            candidates: self.candidates.clone(),
        });
        self.saved_seed = Some(saved.clone());
        saved.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.task(),
            guard.archive_root(),
        )?;
        Ok(saved)
    }

    /// Cover ONLY the exact Initializing1 owner selected by the real scope finisher. The
    /// distinct guard cannot be obtained for a vacant row or a Current2 resync retry.
    pub(in crate::store) fn save_head(
        &mut self,
        guard: &InitializingV2GenesisGroupGuard<'_, '_>,
    ) -> Result<Arc<SavedV2GenesisHead>, AppError> {
        use critical_namespaces::refused;
        let seed = self.saved_seed.as_ref().ok_or_else(refused)?;
        if !Arc::ptr_eq(seed, guard.activation()) {
            return Err(refused());
        }
        guard.validate_source_only()?;
        if let Some(saved) = &self.saved_head {
            saved.validate_under_archive_root(
                guard.store(),
                guard.source(),
                guard.task(),
                guard.archive_root(),
            )?;
            return Ok(saved.clone());
        }
        seed.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.task(),
            guard.archive_root(),
        )?;
        let (group_path, _, _) = guard.directory().directory_data();
        let target = &guard.source().source().capture.target;
        let pin_path = child(group_path, PIN)?;
        let stage_path = child(target, STAGE)?;
        let head_path = child(target, HEAD)?;
        guard.validate_source_only()?;
        ensure_new_founder_control(
            guard.store(),
            &pin_path,
            &self.candidates.pin,
            &mut self.pin_file,
            182,
            183,
        )?;
        crate::store::persistence::sync_directory(group_path).map_err(io)?;
        let promoted = self.head_file.as_ref().is_some_and(|held| {
            held.identity.is_some_and(|id| {
                matches!(
                    catcoms_storage::kept::session_at(&head_path, id),
                    catcoms_storage::kept::SessionAt::Ours
                )
            })
        });
        if promoted {
            // Only this original held object can prove an ambiguous rename completed. A
            // byte-identical foreign current, successor head or leftover stage is refused.
            super::require_absent(&stage_path)?;
            exact_held_frame(
                &head_path,
                self.head_file.as_ref().ok_or_else(refused)?,
                &self.candidates.head,
            )?;
        } else {
            super::require_absent(&head_path)?;
            guard.validate_source_only()?;
            ensure_new_founder_control(
                guard.store(),
                &stage_path,
                &self.candidates.head,
                &mut self.head_file,
                184,
                185,
            )?;
            seed.validate_under_archive_root(
                guard.store(),
                guard.source(),
                guard.task(),
                guard.archive_root(),
            )?;
            exact_held_frame(
                &stage_path,
                self.head_file.as_ref().ok_or_else(refused)?,
                &self.candidates.head,
            )?;
            super::require_absent(&head_path)?;
            guard.validate_source_only()?;
            // This is the only promotion. The actual held stage/File identity is retained
            // across a failed rename or parent barrier; retries classify that SAME object.
            fs::rename(&stage_path, &head_path).map_err(io)?;
        }
        crate::store::persistence::sync_directory(target).map_err(io)?;
        exact_held_frame(
            &pin_path,
            self.pin_file.as_ref().ok_or_else(refused)?,
            &self.candidates.pin,
        )?;
        exact_held_frame(
            &head_path,
            self.head_file.as_ref().ok_or_else(refused)?,
            &self.candidates.head,
        )?;
        guard.validate_source_only()?;
        // All fallible work completed. Both ensure_new calls guarantee their actual owners.
        // Keep the full receipt in this task BEFORE returning it to the scope finisher.
        let saved = Arc::new(SavedV2GenesisHead {
            seed: seed.clone(),
            pin: self.pin_file.take().expect("completed original V2 pin"),
            head: self.head_file.take().expect("completed original V2 head"),
            writer: guard.writer_registration().observe_original(),
            candidates: self.candidates.clone(),
        });
        self.saved_head = Some(saved.clone());
        saved.validate_under_archive_root(
            guard.store(),
            guard.source(),
            guard.task(),
            guard.archive_root(),
        )?;
        Ok(saved)
    }
}

impl ActivationCandidates {
    /// Identity/negative checks only: safe for the scope serializer BEFORE it takes pins or
    /// Root. This MUST NOT call validate_original, quota validation, codec/SDK, or filesystem.
    fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        // Match the actual opaque SDK-seed instance to its original resource bundle as
        // well as the paid read below. Equal source bytes cannot replace either owner.
        // This is the allocation-free negative loan check, safe under existing writers.
        source.require_original_negative(store)?;
        self.original_read.require_same_original(source.source())?;
        let capture = &source.source().capture;
        capture.require_open()?;
        if !Arc::ptr_eq(&capture.mount, &store.registry_mount())
            || self.expected.context != capture.context
            || self.expected.source_id != capture.source_id
            || self.expected.invocation != capture.invocation
        {
            return Err(critical_namespaces::refused());
        }
        source
            .seed()
            .check_opened_source_data(source.source().data())?;
        Ok(())
    }
}

fn exact_held_frame(
    path: &Path,
    held: &critical_namespaces::pending_join::CreatedStage,
    expected: &[u8],
) -> Result<(), AppError> {
    use critical_namespaces::refused;
    verify_archive_held_name(path, held)?;
    let (file, actual) = crate::store::persistence::read_archive_object(path, expected.len())?
        .ok_or_else(refused)?;
    if catcoms_storage::kept::session_id(&file).map_err(io)? != held.identity.ok_or_else(refused)?
        || actual.as_slice() != expected
    {
        return Err(refused());
    }
    verify_archive_held_name(path, held)
}

impl SavedV2GenesisActivationSeed {
    /// Opaque actual same-task/source proof only, not a phase/Current permit. No lock, I/O,
    /// quota, callback or positive-owner creation is permitted in this pre-writer predicate.
    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.group.require_same_original(task)?;
        self.candidates.retains_original(store, source)?;
        self.directory.retains_original(store, source, task)
    }

    /// Full original physical closure after the real writer has acquired admission -> pins
    /// -> canonical -> Root. This variant never reacquires any outer guard.
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.retains_original(store, source, task)?;
        source.source().validate_under_archive_root(store, root)?;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(critical_namespaces::refused)?;
        self.candidates
            ._fee
            .validate(&policy.pending.pool, codec::ACTIVATION_METADATA_FEE)?;
        self.directory
            .validate_under_archive_root(store, source, task, root)?;
        let (base, held, id) = self.directory.directory_data();
        let path = child(base, SEED)?;
        exact_held_frame(&path, &self.file, &self.candidates.seed)?;
        // Candidate comparisons/quota/directory waits may have outlived the earlier proof.
        // Repeat Current-layout and actual SOURCE controls after all those full comparers;
        // then only held-object identity checks and cheap original negatives may follow.
        self.directory
            .validate_under_archive_root(store, source, task, root)?;
        source.source().validate_under_archive_root(store, root)?;
        let capture = &source.source().capture;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(
            &capture.target,
            &capture.target_directory,
            capture.target_id,
        )?;
        require_directory_name(base, held, id)?;
        verify_archive_held_name(&path, &self.file)?;
        self.retains_original(store, source, task)
    }

    pub(in crate::store) fn expectation_data(&self) -> &SavedSourceExpectationData {
        &self.candidates.expected
    }
}

impl SavedV2GenesisHead {
    /// Exact original activation association at the callback return edge. This cannot
    /// construct a receipt, select another source or issue storage/publication authority.
    pub(in crate::store) fn activation(&self) -> &Arc<SavedV2GenesisActivationSeed> {
        &self.seed
    }

    /// The scope finisher retains this complete independently paid original observation LAST.
    /// Neither this borrow nor a clone keeps the canonical writer's Sender alive.
    pub(in crate::store) fn writer_observation(&self) -> &V2WriterObservation {
        &self.writer
    }

    pub(in crate::store) fn retains_original(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.candidates, &self.seed.candidates) {
            return Err(critical_namespaces::refused());
        }
        if self.writer.is_cancelled() {
            return Err(critical_namespaces::refused());
        }
        self.seed.retains_original(store, source, task)
    }

    /// This pre-handoff receipt still requires its independent activation seed. The later
    /// owned Current/node handoff must use a distinct seed-settled role, not weaken this proof
    /// with a parsed phase, missing-file fallback or a boolean mode.
    pub(in crate::store) fn validate_under_archive_root(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        root: &critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<(), AppError> {
        self.retains_original(store, source, task)?;
        self.seed
            .validate_under_archive_root(store, source, task, root)?;
        let capture = &source.source().capture;
        let (base, directory, id) = self.seed.directory.directory_data();
        let pin = child(base, PIN)?;
        let head = child(&capture.target, HEAD)?;
        let stage = child(&capture.target, STAGE)?;
        super::require_absent(&stage)?;
        exact_held_frame(&pin, &self.pin, &self.candidates.pin)?;
        exact_held_frame(&head, &self.head, &self.candidates.head)?;
        let seed_path = child(base, SEED)?;
        exact_held_frame(&seed_path, &self.seed.file, &self.candidates.seed)?;
        self.seed
            .directory
            .validate_under_archive_root(store, source, task, root)?;
        source.source().validate_under_archive_root(store, root)?;
        require_directory_name(&capture.root, &capture.root_directory, capture.root_id)?;
        require_directory_name(
            &capture.target,
            &capture.target_directory,
            capture.target_id,
        )?;
        require_directory_name(base, directory, id)?;
        verify_archive_held_name(&pin, &self.pin)?;
        verify_archive_held_name(&head, &self.head)?;
        verify_archive_held_name(&seed_path, &self.seed.file)?;
        self.retains_original(store, source, task)
    }
}

fn require_directory_name(
    path: &Path,
    held: &File,
    expected: catcoms_storage::kept::SessionId,
) -> Result<(), AppError> {
    let named = crate::store::persistence::open_archive_directory(path)?;
    if catcoms_storage::kept::session_id(held).map_err(io)? != expected
        || catcoms_storage::kept::session_id(&named).map_err(io)? != expected
    {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

/// Exact complete path capacity BEFORE pushing closed components. A cloned PathBuf followed
/// by join can double its allocation and is not covered by the reviewed activation invoice.
fn child(base: &Path, leaf: &str) -> Result<PathBuf, AppError> {
    let len = base
        .as_os_str()
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_add(leaf.len()))
        .filter(|n| *n <= MAX_PATH + 256)
        .ok_or_else(critical_namespaces::refused)?;
    let mut path = PathBuf::with_capacity(len);
    path.push(base);
    path.push(leaf);
    if path.capacity() > MAX_PATH + 256 {
        return Err(critical_namespaces::refused());
    }
    Ok(path)
}

fn validate_invoice() -> Result<(), AppError> {
    let bytes = 8 * (MAX_PATH + 256)
        + 12 * (codec::MAX_ACTIVATION_FRAME + codec::MAX_HEAD_FRAME + codec::MAX_PIN_FRAME)
        + std::mem::size_of::<ActivationCandidates>()
        + std::mem::size_of::<PreparedV2GenesisActivation>()
        + std::mem::size_of::<SavedV2GenesisActivationSeed>()
        + std::mem::size_of::<SavedV2GenesisHead>()
        + 2 * crate::store::persistence::ORIGINAL_BOOTSTRAP_READ_OBSERVATION_BYTES
        + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 64
        + crate::store::chat_admission::v2_current::V2_CURRENT_SCOPE_METADATA_BYTES
        + 16 * 1024;
    if bytes > codec::ACTIVATION_METADATA_FEE {
        return Err(critical_namespaces::refused());
    }
    Ok(())
}

impl PreparedV2GenesisActivation {
    /// Prepare exact encrypted candidates outside all host writers. The actual source capture
    /// supplies invocation/control identities; the separately authenticated immutable debit
    /// supplies its original App display name. No caller label supplies either authority.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<Self, AppError> {
        use critical_namespaces::refused;
        source.validate_original(store)?;
        validate_invoice()?;
        let policy = store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?;
        let fee = policy
            .pending
            .pool
            .try_reserve(codec::ACTIVATION_METADATA_FEE)?;
        source.validate_original(store)?;
        source
            .seed()
            .check_opened_source_data(source.source().data())?;
        let capture = &source.source().capture;
        let key = Zeroizing::new(store.keys.db_key()?);
        let debit = founder::open_debit_data(&key, capture.invocation, &capture.debit.frame)?;
        let expected = SavedSourceExpectationData {
            invocation: capture.invocation,
            debit_id: founder::DebitId::of(&capture.debit.frame),
            terminal_id: founder::IntentId::of(&capture.terminal.frame),
            source_id: capture.source_id,
            marker_id: founder::TargetMarkerId::of(&capture.origin.frame),
            context: capture.context.clone(),
            debit_display_name: debit.name,
        };
        let public = source.data();
        let original = codec::GenesisPublicData {
            profile: public.profile,
            admission: public.admission,
            header: public.header,
            image: public.state,
        };
        let pin = codec::GroupPinData::from_source_data(&expected, original.profile)?;
        let pin_frame = codec::seal_pin_data(&key, &pin, rng)?;
        let head = codec::GenesisHeadData::from_source_data(
            &expected,
            V2GroupPinId::of(&pin_frame),
            &pin,
            &original,
        )?;
        let head_frame = codec::seal_head_data(&key, &head, &pin, rng)?;
        let seed_plain =
            codec::encode_activation_seed_data(&key, &expected, &pin_frame, &head_frame)?;
        let seed_frame = codec::seal_activation_seed_data(&key, &seed_plain, rng)?;
        // The large decode already happened under its separate original fee. Repeat only
        // the original bounded controls/objects after these small encodes and RNG callbacks.
        source.validate_original(store)?;
        Ok(Self {
            candidates: Arc::new(ActivationCandidates {
                original_read: source.source().observe_original(),
                expected,
                pin: pin_frame,
                head: head_frame,
                seed: seed_frame,
                _fee: fee,
            }),
            seed_file: None,
            pin_file: None,
            head_file: None,
            saved_seed: None,
            saved_head: None,
        })
    }
}

#[cfg(test)]
mod tests;
