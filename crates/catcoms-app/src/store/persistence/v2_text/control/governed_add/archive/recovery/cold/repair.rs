//! Completion-only recovery for an already stored later Add. A restarted
//! owner never adopts a partial stage, enters a key, or resumes MLS work.
use super::super::super::held::{add_stage_path, select_named_head, ADD_STAGES, REMOVE_STAGES};
use super::*;
use crate::store::persistence::v2_text::io::governed_add_successor_child;
use std::fs::OpenOptions;

struct CompleteStagedAdd {
    origin: ColdFounderOrigin,
    parent: OpenedTextControl,
    child: (PathBuf, OpenedTextControl),
    archive: (PathBuf, OpenedTextControl),
    stages: Vec<(PathBuf, OpenedTextControl)>,
    chain: RecoveredArchiveChain,
    selected: selected_head::GovernedHeadData,
    parent_id: TextHeadIdData,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl HeldColdFounderAdd {
    /// An authenticated later Add already named by Head can be the result
    /// of an interrupted rename. Cover those same named objects before
    /// startup exposes the recovered row. No stage is recreated.
    pub(in crate::store) fn cover_selected_add_startup(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        if self.files.selected.kind != selected_head::GovernedHeadKind::Add {
            return Ok(());
        }
        self.files.origin.original.require_same_original(source)?;
        self.core.as_ref().ok_or_else(refused)?;
        let parent = (
            self.files.selected.transition.context,
            TextHeadIdData::of_data(&self.files.head.frame),
        );
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let scope = ColdGovernedWriteScope {
            store,
            source,
            root: root.as_ref().ok_or_else(refused)?,
            native: caller,
            parent,
        };
        let target = scope.directory_data().0;
        let require = |files: &mut ColdFounderFiles| {
            scope.require_current(parent)?;
            self.recovered.require_predecessor_contents(source)?;
            files.origin.require_named_files(source)?;
            files.require_named_parent(source)?;
            require_successor_stages_absent(target, parent.1)?;
            let previous_stage = add_stage_path(
                target,
                selected_head::NAME,
                Some(files.selected.transition.before_head),
            )?;
            absent(&previous_stage)?;
            scope.require_live_cached(parent)
        };
        require(&mut self.files)?;
        sync_existing(&self.files.archive_path, &mut self.files.archive)?;
        sync_existing(&fixed_child(target, "head.bin")?, &mut self.files.head)?;
        crate::store::persistence::sync_directory(target).map_err(io)?;
        require(&mut self.files)
    }

    /// Try to cover one complete original successor before normal cold
    /// attachment. The physical vault and original founder remain held.
    /// No receipt or join response is released by the repair.
    pub(in crate::store) fn complete_staged_add(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin: [u8; 32],
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<bool, AppError> {
        let Some(mut repair) = CompleteStagedAdd::capture(store, source, pin, caller)? else {
            return Ok(false);
        };
        repair.cover_selection(store, source, caller)?;
        Ok(true)
    }
}

impl CompleteStagedAdd {
    fn capture(
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        pin: [u8; 32],
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<Option<Self>, AppError> {
        if caller.is_cancelled() {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        const FEE: usize = RecoveredArchiveChain::invoice_bytes()
            + 5 * (MAX_BYTES + 1)
            + 18 * MAX_TEXT_PATH
            + 4 * SEED_FRAME_LIMIT
            + 4 * std::mem::size_of::<CompleteStagedAdd>();
        let fee = pool.try_reserve(FEE)?;
        fee.validate(pool, FEE)?;
        let lease = store._session.retain_for_physical_work();
        let genesis = source.data().genesis_public_data();
        let trusted = GenesisPublicData {
            profile: genesis.profile_data(),
            admission: genesis.admission_data(),
            header: genesis.header_data(),
            image: genesis.image_data(),
        };
        let opened = OpenedColdFounderOrigin::capture(store, source, pin, &trusted)?;
        let (mut origin, key, plain) = (opened.retained, opened.key, opened.plain);
        let promotion = parse_data(&plain)?;
        let target = source.original_target_data().0;
        let head_path = fixed_child(target, "head.bin")?;
        let mut parent = read_control(&head_path, selected_head::FRAME_LEN)?;
        let parent_id = TextHeadIdData::of_data(&parent.frame);
        let stage_path = add_stage_path(target, selected_head::NAME, Some(parent_id))?;
        // Absence only means that this recovery role has nothing to finish.
        // The separate ordinary cold reader still authenticates the Head.
        if matches!(fs::symlink_metadata(&stage_path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            return Ok(None);
        }
        let child = read_control(&stage_path, selected_head::FRAME_LEN)?;
        let selected = selected_head::GovernedHeadData::open_governed_data(&key, &child.frame)?;
        if selected
            != selected_head::GovernedHeadData::after_governed_archive(
                &key,
                &parent.frame,
                selected.transition,
                selected.archive,
            )?
            || selected.transition.context != promotion.context.id_data()?
        {
            return Err(refused());
        }
        let archive_path = archive_child(target, selected.archive.as_bytes())?;
        let archive = read_control_into(
            &archive_path,
            MAX_BYTES,
            &mut Some(Zeroizing::new(Vec::with_capacity(MAX_BYTES + 1))),
        )?;
        let clear = open_later_data(&key, &selected, &archive.frame)?;
        let (_, frames) = decode_later_data(&clear)?;
        if frames.parent_head != parent.frame.as_slice() {
            return Err(refused());
        }
        let raw = [
            frames.fence,
            frames.request,
            frames.entry,
            frames.outcome,
            frames.header_attempt,
            frames.raw_header,
            frames.governing,
            frames.welcome_intent,
            frames.welcome_attempt,
            frames.raw_welcome,
            frames.response,
        ];
        let mut stages = Vec::with_capacity(raw.len());
        for (name, bytes) in ADD_STAGES[..raw.len()].iter().zip(raw) {
            let path = add_stage_path(target, name, Some(parent_id))?;
            let record = read_control_into(
                &path,
                bytes.len(),
                &mut Some(Zeroizing::new(Vec::with_capacity(bytes.len() + 1))),
            )?;
            if record.frame.as_slice() != bytes {
                return Err(refused());
            }
            stages.push((path, record));
        }
        drop(clear);
        let initial_head = TextHeadIdData::of_data(promotion.head_frame);
        let initial_archive = ArchiveHeadId::from_bytes(
            *TextControlKindData::Archive
                .id_data(promotion.archive_frame)
                .as_bytes(),
        );
        let chain = RecoveredArchiveChain::capture(
            &key,
            target,
            &child.frame,
            &archive.frame,
            (selected.transition.context, initial_head, initial_archive),
            &trusted,
            || {
                if caller.is_cancelled() {
                    return Err(refused());
                }
                origin.require_current(store, source)?;
                require_exact_held(&head_path, &mut parent)
            },
        )?;
        // Independently prove that the closed SDK restores to the same signed
        // Header, epoch, state and authenticator as the replayed Core prefix.
        // This uses retained snapshots only, outside the physical writer.
        let proof = V2GoverningJoinPrefixData::decode(&chain.prefix)?;
        let sdk = PreparedV2FounderRecoveredAdd::restore_from_governing_prefix(
            source.data(),
            V2InstalledFounderAddData {
                epoch: selected.transition.epoch,
                authenticator: selected.transition.authenticator,
                header: selected.transition.header,
                state: selected.transition.state,
            },
            chain.journal.selected_admission().ok_or_else(refused)?,
            &proof,
            chain.journal.snapshot_data()?,
            restore_admission(store)?,
        )?;
        drop(sdk);
        Ok(Some(Self {
            origin,
            parent,
            child: (stage_path, child),
            archive: (archive_path, archive),
            stages,
            chain,
            selected,
            parent_id,
            _lease: lease,
            _fee: fee,
        }))
    }

    fn cover_selection(
        &mut self,
        store: &ServerStore,
        source: &RetainedBootstrapSourceData,
        caller: &catcoms_rt::RequestCancellation,
    ) -> Result<(), AppError> {
        self.origin.original.require_same_original(source)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        let parent = (self.selected.transition.context, self.parent_id);
        let scope = ColdGovernedWriteScope {
            store,
            source,
            root: root.as_ref().ok_or_else(refused)?,
            native: caller,
            parent,
        };
        let target = scope.directory_data().0;
        let head = fixed_child(target, "head.bin")?;
        self.require_original(source, &scope, &head, false)?;
        for (path, record) in self
            .stages
            .iter_mut()
            .chain(std::iter::once(&mut self.archive))
        {
            sync_existing(path, record)?;
        }
        sync_existing(&self.child.0, &mut self.child.1)?;
        crate::store::persistence::sync_directory(target).map_err(io)?;
        self.require_original(source, &scope, &head, false)?;

        let (stage, child) = &mut self.child;
        let mut entered = false;
        select_named_head(
            scope.directory_data(),
            stage,
            &head,
            &mut self.parent,
            &mut entered,
            || {
                require_exact_held(stage, child)?;
                scope.require_live_cached(parent)
            },
        )?;
        crate::store::persistence::sync_directory(target).map_err(io)?;
        self.require_original(source, &scope, &head, true)
    }

    fn require_original(
        &mut self,
        source: &RetainedBootstrapSourceData,
        scope: &ColdGovernedWriteScope<'_>,
        head: &Path,
        selected: bool,
    ) -> Result<(), AppError> {
        let parent = (self.selected.transition.context, self.parent_id);
        scope.require_current(parent)?;
        self.origin.require_named_files(source)?;
        let target = scope.directory_data().0;
        self.chain.require_predecessor_contents(target)?;
        let next = TextHeadIdData::of_data(&self.child.1.frame);
        require_successor_stages_absent(target, next)?;
        for name in REMOVE_STAGES {
            absent(&governed_add_successor_child(
                target,
                name,
                self.parent_id.as_bytes(),
            )?)?;
        }
        for (path, record) in &mut self.stages {
            require_exact_held(path, record)?;
        }
        require_exact_held(&self.archive.0, &mut self.archive.1)?;
        if selected {
            absent(&self.child.0)?;
            require_exact_held(head, &mut self.child.1)?;
        } else {
            require_exact_held(head, &mut self.parent)?;
            require_exact_held(&self.child.0, &mut self.child.1)?;
        }
        scope.require_live_cached(parent)
    }
}

fn absent(path: &Path) -> Result<(), AppError> {
    if matches!(fs::symlink_metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound)
    {
        Ok(())
    } else {
        Err(refused())
    }
}

fn io(error: std::io::Error) -> AppError {
    AppError::Io(error.to_string())
}

// Windows file flush requires a writable handle. Reopen only an existing
// object, verify that handle and name, and never write, create or truncate.
fn sync_existing(path: &Path, original: &mut OpenedTextControl) -> Result<(), AppError> {
    require_exact_held(path, original)?;
    let file = OpenOptions::new().write(true).open(path).map_err(io)?;
    if session_id(&file).map_err(io)? != original.id {
        return Err(refused());
    }
    require_exact_held(path, original)?;
    file.sync_all().map_err(io)?;
    require_exact_held(path, original)
}
