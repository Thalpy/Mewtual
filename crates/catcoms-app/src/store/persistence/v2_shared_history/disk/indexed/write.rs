//! One covered exact original, bounded COW outputs, then Head CAS. The WAL
//! remains until selected output proof and obsolete derived-page retirement
//! complete. Its Head commitment also permits restart after a GC cut.
use super::*;
use crate::store::persistence::v2_member_outbox::fs::rename_fixed;

struct ReadyIndexedRetirement {
    row: SharedArchiveIndexValueData,
    head_digest: [u8; 32],
    obsolete: Vec<PrivateChatPageId>,
}

impl SharedHistoryDisk {
    pub(in crate::store::persistence::v2_shared_history::disk) fn prepare_indexed_save(
        &self,
        key: &[u8; 32],
        expected: SelectedHistoryRecord,
        bundle: &[u8],
        admission: OwnerFactId,
        original: &AssembledActionOriginal,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(Zeroizing<Vec<u8>>, Option<PreparedIndexedSave>), AppError> {
        let _gate = self.gate.try_lock().map_err(|_| refused())?;
        self.indexed_clean()?;
        let (mut head_file, head) = self.indexed_head(key)?;
        if let Some((_, saved_admission, saved)) =
            self.indexed_read_selected(key, expected.action)?
        {
            if saved_admission != admission || saved.original_bytes() != original.original_bytes() {
                return Err(refused());
            }
            head_file.require()?;
            // A true saved retry draws nothing and never reconstructs a WAL.
            return Ok((Zeroizing::new(Vec::new()), None));
        }
        if head.count >= self.quota.records as u64 {
            return Err(refused());
        }
        let indexes = index::ActionIndex::new(key, self.indexed_layout()?, self.scope);
        let row = row(expected, admission, original, bundle);
        let mut dependencies = Vec::with_capacity(4 * MAX_PATH);
        let (root, pages, path) = indexes.prepare(
            head.root,
            row,
            |id| self.indexed_page(id, &mut dependencies),
            rng,
        )?;
        let bytes = successor_bytes(
            head,
            bundle.len(),
            &path,
            pages.iter().map(|(_, page)| page.as_bytes().len()),
        )?;
        if bytes > self.quota.bytes as u64 {
            return Err(refused());
        }
        let borrowed = pages
            .iter()
            .map(|(node, page)| (*node, page.as_bytes()))
            .collect::<Vec<_>>();
        let next = codec::Head {
            scope: Some(self.scope.id()),
            generation: head.generation.checked_add(1).ok_or_else(refused)?,
            count: head.count.checked_add(1).ok_or_else(refused)?,
            bytes,
            previous: *blake3::hash(&head_file.bytes).as_bytes(),
            transaction: codec::Wal::commitment(
                &head_file.bytes,
                expected,
                bundle,
                &path,
                &borrowed,
            )?,
            root: Some(root),
        };
        let next_frame = next.seal(key, self.indexed_layout()?, rng)?;
        let wal = codec::Wal::encode(
            &head_file.bytes,
            &next_frame,
            expected,
            bundle,
            &path,
            &pages,
        )?;
        for file in &mut dependencies {
            file.require()?;
        }
        head_file.require()?;
        self.indexed_clean()?;
        Ok((
            wal,
            Some(PreparedIndexedSave {
                head: head_file,
                dependencies,
            }),
        ))
    }

    pub(in crate::store::persistence::v2_shared_history::disk) fn indexed_recover_locked(
        &self,
        key: &[u8; 32],
    ) -> Result<Option<SelectedHistoryRecord>, AppError> {
        if self
            .indexed
            .as_ref()
            .ok_or_else(refused)?
            .retiring
            .try_lock()
            .map_err(|_| refused())?
            .is_some()
        {
            return self.finish_indexed_retirement(key).map(Some);
        }
        if observe(&self.directory.child(WAL))? == Presence::Absent {
            self.indexed_clean()?;
            self.indexed_head(key)?;
            return Ok(None);
        }
        let mut wal = HeldIndexFile::read(self.directory.child(WAL), WAL_CAP)?;
        let ready = self.finish_indexed_wal(key, &mut wal)?;
        self.begin_indexed_retirement(&wal, ready)?;
        drop(wal);
        self.finish_indexed_retirement(key).map(Some)
    }

    pub(in crate::store::persistence::v2_shared_history::disk) fn indexed_pending_action(
        &self,
        key: &[u8; 32],
    ) -> Result<Option<PendingIndexedWrite>, AppError> {
        if self
            .indexed
            .as_ref()
            .ok_or_else(refused)?
            .retiring
            .try_lock()
            .map_err(|_| refused())?
            .is_some()
        {
            return Err(refused());
        }
        if observe(&self.directory.child(WAL))? == Presence::Absent {
            if observe(&self.directory.child(STAGE))? != Presence::Absent {
                return Err(refused());
            }
            return Ok(None);
        }
        let mut held = HeldIndexFile::read(self.directory.child(WAL), WAL_CAP)?;
        let wal = codec::Wal::decode(&held.bytes)?;
        self.authenticate_indexed_wal(key, &wal)?;
        let (mut selected, _) = self.indexed_head(key)?;
        if selected.bytes.as_slice() != wal.old && selected.bytes.as_slice() != wal.next {
            return Err(refused());
        }
        let mut stage = if observe(&self.directory.child(STAGE))? == Presence::Present {
            let stage = HeldIndexFile::read(self.directory.child(STAGE), INDEXED_HEAD_CAP)?;
            if stage.bytes.as_slice() != wal.next {
                return Err(refused());
            }
            Some(stage)
        } else {
            None
        };
        if let Some(stage) = stage.as_mut() {
            stage.require()?;
        }
        selected.require()?;
        let record = wal.record;
        held.require()?;
        Ok(Some(PendingIndexedWrite {
            record,
            wal: held,
            stage,
        }))
    }

    fn authenticate_indexed_wal(
        &self,
        key: &[u8; 32],
        wal: &codec::Wal<'_>,
    ) -> Result<(codec::Head, codec::Head, SharedArchiveIndexValueData), AppError> {
        let layout = self.indexed_layout()?;
        let old = codec::Head::open(key, layout, wal.old)?;
        let next = codec::Head::open(key, layout, wal.next)?;
        old.require_scope(self.scope)?;
        next.require_scope(self.scope)?;
        let (admission, original) = open_shared_history_bundle(
            &self.scope,
            key,
            (wal.record.action, wal.record.manifest),
            wal.bundle,
        )?;
        let row = row(wal.record, admission, &original, wal.bundle);
        if next.generation != old.generation.checked_add(1).ok_or_else(refused)?
            || next.count != old.count.checked_add(1).ok_or_else(refused)?
            || next.scope != Some(self.scope.id())
            || next.previous != *blake3::hash(wal.old).as_bytes()
            || next.transaction
                != codec::Wal::commitment(
                    wal.old,
                    wal.record,
                    wal.bundle,
                    &wal.original,
                    &wal.outputs,
                )?
            || next.bytes
                != successor_bytes(
                    old,
                    wal.bundle.len(),
                    &wal.original,
                    wal.outputs.iter().map(|(_, bytes)| bytes.len()),
                )?
            || next.count > self.quota.records as u64
            || next.bytes > self.quota.bytes as u64
            || wal.original.first().map(|(node, _)| *node) != old.root
        {
            return Err(refused());
        }
        Ok((old, next, row))
    }

    fn finish_indexed_wal(
        &self,
        key: &[u8; 32],
        wal_file: &mut HeldIndexFile,
    ) -> Result<ReadyIndexedRetirement, AppError> {
        let wal = codec::Wal::decode(&wal_file.bytes)?;
        let (old, next, row) = self.authenticate_indexed_wal(key, &wal)?;
        let (mut selected_file, selected) = self.indexed_head(key)?;
        let already_selected = selected_file.bytes.as_slice() == wal.next;
        if !already_selected && selected_file.bytes.as_slice() != wal.old {
            return Err(refused());
        }
        if selected != if already_selected { next } else { old } {
            return Err(refused());
        }
        let indexes = index::ActionIndex::new(key, self.indexed_layout()?, self.scope);
        let mut source_files = Vec::with_capacity(4 * MAX_PATH);
        if !already_selected {
            let original = indexes.verify(
                old.root,
                next.root.ok_or_else(refused)?,
                row,
                &wal.outputs,
                |id| self.indexed_page(id, &mut source_files),
            )?;
            if original.as_slice()
                != wal
                    .original
                    .iter()
                    .map(|(node, _)| *node)
                    .collect::<Vec<_>>()
            {
                return Err(refused());
            }
            for (node, length) in &wal.original {
                let file = source_files
                    .iter()
                    .find(|file| file.name == self.directory.child(&index_name(node.page)))
                    .ok_or_else(refused)?;
                if file.bytes.len() != *length {
                    return Err(refused());
                }
            }
        }
        // The entire transaction is authenticated before rebarriering or
        // creating outputs. A partial canonical WAL is held, never guessed.
        let mut wal_barrier = open_exact_write(&wal_file.name, WriteMode::Existing)?;
        if wal_barrier.1 != wal_file.file.1 {
            return Err(refused());
        }
        finish_exact_write(&wal_file.name, &mut wal_barrier, &wal_file.bytes)?;
        self.directory.sync()?;
        drop(wal_barrier);
        let bundle_path = self.bundle_path(wal.record.manifest);
        let mut bundle = ensure_exact_held(&bundle_path, wal.bundle)?;
        let mut outputs = Vec::with_capacity(wal.outputs.len());
        for (node, bytes) in &wal.outputs {
            let path = self.directory.child(&index_name(node.page));
            let held = ensure_exact_held(&path, bytes)?;
            outputs.push((path, held, *bytes));
        }
        self.directory.sync()?;
        let mut stage = if already_selected {
            None
        } else {
            Some(ensure_exact_held(&self.directory.child(STAGE), wal.next)?)
        };
        self.directory.sync()?;
        for source in &mut source_files {
            source.require()?;
        }
        require_exact_held(&bundle_path, &mut bundle, wal.bundle)?;
        for (path, file, bytes) in &mut outputs {
            require_exact_held(path, file, bytes)?;
        }
        selected_file.require()?;
        require_exact_held(&wal_file.name, &mut wal_file.file, &wal_file.bytes)?;
        if let Some(mut stage) = stage.take() {
            require_exact_held(&self.directory.child(STAGE), &mut stage, wal.next)?;
            let candidate = HeldIndexFile {
                name: self.directory.child(INDEXED_HEAD_NAME),
                file: stage,
                bytes: Zeroizing::new(wal.next.to_vec()),
            };
            let selected_copy = HeldIndexFile {
                name: candidate.name.clone(),
                file: (
                    candidate
                        .file
                        .0
                        .try_clone()
                        .map_err(|error| AppError::Io(error.to_string()))?,
                    candidate.file.1,
                ),
                bytes: candidate.bytes.clone(),
            };
            let owner = self.indexed.as_ref().ok_or_else(refused)?;
            let mut original = owner.head.try_lock().map_err(|_| refused())?;
            original.require()?;
            if original.file.1 != selected_file.file.1 || original.bytes != selected_file.bytes {
                return Err(refused());
            }
            rename_fixed(&self.directory.child(STAGE), &candidate.name)?;
            // The actual candidate File enters the original shared owner
            // before ANY post-rename proof, sync or allocation can fail.
            *original = candidate;
            selected_file = selected_copy;
            drop(original);
            selected_file.require()?;
        } else if observe(&self.directory.child(STAGE))? != Presence::Absent {
            // A duplicate exact stage is attributable to this admitted WAL;
            // malformed/replaced stage evidence is preserved and refuses.
            let mut duplicate = HeldIndexFile::read(self.directory.child(STAGE), INDEXED_HEAD_CAP)?;
            if duplicate.bytes.as_slice() != wal.next {
                return Err(refused());
            }
            duplicate.require()?;
            remove_listed(&duplicate.name)?;
            let name = duplicate.name.clone();
            drop(duplicate);
            self.directory.sync()?;
            require_absent(&name)?;
        }
        selected_file.sync()?;
        self.directory.sync()?;
        #[cfg(test)]
        if take_fault(IndexedHistoryFault::AfterHead) {
            return Err(refused());
        }
        // The exact selected root must actually point to this record before
        // retiring even derived pages. Unchanged siblings remain authenticated.
        let mut selected_paths = Vec::with_capacity(4 * MAX_PATH);
        if indexes.lookup(next.root, wal.record.action, |id| {
            self.indexed_page(id, &mut selected_paths)
        })? != Some(row)
        {
            return Err(refused());
        }
        for path in &mut selected_paths {
            path.require()?;
        }
        // Only derived old paths can retire. Release all of OUR source
        // handles before the Windows true-absence barrier for those names.
        drop(source_files);
        for (depth, (old, size)) in wal.original.iter().copied().enumerate() {
            let mut pivots = Vec::with_capacity(4 * MAX_PATH);
            indexes.require_old_unreachable(next.root.ok_or_else(refused)?, old, |id| {
                self.indexed_page(id, &mut pivots)
            })?;
            let path = self.directory.child(&index_name(old.page));
            if observe(&path)? == Presence::Present {
                let mut obsolete = HeldIndexFile::read(path, MAX_INDEX_FRAME_BYTES)?;
                if obsolete.bytes.len() != size {
                    return Err(refused());
                }
                indexes.require_old_page(old, depth, &obsolete.bytes)?;
                for pivot in &mut pivots {
                    pivot.require()?;
                }
                selected_file.require()?;
                require_exact_held(&wal_file.name, &mut wal_file.file, &wal_file.bytes)?;
                obsolete.require()?;
                remove_listed(&obsolete.name)?;
                let name = obsolete.name.clone();
                drop(obsolete);
                self.directory.sync()?;
                require_absent(&name)?;
                #[cfg(test)]
                if take_fault(IndexedHistoryFault::AfterPageRetirement) {
                    return Err(refused());
                }
            } else {
                require_absent(&path)?;
            }
        }
        for (path, file, bytes) in &mut outputs {
            require_exact_held(path, file, bytes)?;
        }
        require_exact_held(&bundle_path, &mut bundle, wal.bundle)?;
        selected_file.require()?;
        require_exact_held(&wal_file.name, &mut wal_file.file, &wal_file.bytes)?;
        require_absent(&self.directory.child(STAGE))?;
        Ok(ReadyIndexedRetirement {
            row,
            head_digest: *blake3::hash(wal.next).as_bytes(),
            obsolete: wal.original.iter().map(|(node, _)| node.page).collect(),
        })
    }

    fn begin_indexed_retirement(
        &self,
        wal: &HeldIndexFile,
        ready: ReadyIndexedRetirement,
    ) -> Result<(), AppError> {
        let owner = self.indexed.as_ref().ok_or_else(refused)?;
        let mut retiring = owner.retiring.try_lock().map_err(|_| refused())?;
        if retiring.is_some() {
            return Err(refused());
        }
        let mut head = owner.head.try_lock().map_err(|_| refused())?;
        head.require()?;
        if *blake3::hash(&head.bytes).as_bytes() != ready.head_digest {
            return Err(refused());
        }
        let file = wal
            .file
            .0
            .try_clone()
            .map_err(|error| AppError::Io(error.to_string()))?;
        *retiring = Some(IndexedRetirement {
            wal: Some((file, wal.file.1)),
            wal_bytes: wal.bytes.len(),
            wal_digest: *blake3::hash(&wal.bytes).as_bytes(),
            head_digest: ready.head_digest,
            row: ready.row,
            obsolete: ready.obsolete,
        });
        Ok(())
    }

    pub(in crate::store::persistence::v2_shared_history::disk) fn finish_indexed_retirement(
        &self,
        key: &[u8; 32],
    ) -> Result<SelectedHistoryRecord, AppError> {
        let owner = self.indexed.as_ref().ok_or_else(refused)?;
        let mut slot = owner.retiring.try_lock().map_err(|_| refused())?;
        let proof = slot.as_mut().ok_or_else(refused)?;
        let (mut head, _) = self.indexed_head(key)?;
        if *blake3::hash(&head.bytes).as_bytes() != proof.head_digest {
            return Err(refused());
        }
        let expected = proof.row;
        // A cleanup retry must re-open the actual selected index path,
        // not infer selection from the still-readable immutable bundle.
        let selected = self
            .indexed_read_selected_checked(key, expected.action, Some(expected), || {
                for page in &proof.obsolete {
                    require_absent(&self.directory.child(&index_name(*page)))?;
                }
                require_absent(&self.directory.child(STAGE))?;
                let path = self.directory.child(WAL);
                if let Some(original) = proof.wal.as_ref() {
                    let mut wal = HeldIndexFile::read(path.clone(), WAL_CAP)?;
                    if wal.file.1 != original.1
                        || wal.bytes.len() != proof.wal_bytes
                        || *blake3::hash(&wal.bytes).as_bytes() != proof.wal_digest
                    {
                        return Err(refused());
                    }
                    wal.require()?;
                    head.require()?;
                    remove_listed(&path)?;
                    drop(wal);
                    // Close every application WAL handle before true absence.
                    drop(proof.wal.take());
                    #[cfg(test)]
                    if take_fault(IndexedHistoryFault::AfterWalUnlink) {
                        return Err(refused());
                    }
                }
                self.directory.sync()?;
                require_absent(&path)?;
                require_absent(&self.directory.child(STAGE))?;
                for page in &proof.obsolete {
                    require_absent(&self.directory.child(&index_name(*page)))?;
                }
                head.require()?;
                self.directory.require()
            })?
            .ok_or_else(refused)?;
        let record = selected.0;
        *slot = None;
        Ok(record)
    }
}

impl PreparedSharedHistorySave {
    pub(in crate::store::persistence::v2_shared_history::disk) fn commit_indexed(
        &mut self,
        disk: &SharedHistoryDisk,
        key: &[u8; 32],
    ) -> Result<SelectedHistoryRecord, AppError> {
        let mut gate = disk.gate.try_lock().map_err(|_| refused())?;
        advance_revision(&mut gate)?;
        if self.indexed_retiring {
            let retiring = disk
                .indexed
                .as_ref()
                .ok_or_else(refused)?
                .retiring
                .try_lock()
                .map_err(|_| refused())?
                .as_ref()
                .map(|proof| proof.row);
            let selected = if let Some(row) = retiring {
                // Current Store ownership permits only one pending write.
                // Still refuse a direct private-API misuse rather than
                // returning another transaction's successful cleanup.
                if row.action != self.expected.action
                    || row.manifest != self.expected.manifest
                    || row.admission != self.admission
                {
                    return Err(refused());
                }
                let selected = disk.finish_indexed_retirement(key)?;
                if self.prior_selection(disk, key)? != Some(selected) {
                    return Err(refused());
                }
                selected
            } else {
                disk.indexed_clean()?;
                self.prior_selection(disk, key)?.ok_or_else(refused)?
            };
            self.indexed_retiring = false;
            return Ok(selected);
        }
        let path = disk.directory.child(WAL);
        if self.attempt.is_none() {
            disk.indexed_recover_locked(key)?;
            if let Some(selected) = self.prior_selection(disk, key)? {
                return Ok(selected);
            }
            let original = self.indexed.as_mut().ok_or_else(refused)?;
            original.head.require()?;
            for file in &mut original.dependencies {
                file.require()?;
            }
            disk.indexed_clean()?;
            self.attempt = Some(open_exact_write(&path, WriteMode::Create)?);
        } else if observe(&path)? == Presence::Absent {
            let selected = self.prior_selection(disk, key)?.ok_or_else(refused)?;
            self.attempt = None;
            return Ok(selected);
        }
        let held = self.attempt.as_mut().ok_or_else(refused)?;
        #[cfg(test)]
        if SHORT_WAL_WRITE.with(|fault| fault.replace(false)) {
            finish_exact_write(&path, held, &self.wal[..80])?;
            return Err(refused());
        }
        finish_exact_write(&path, held, &self.wal)?;
        disk.directory.sync()?;
        if let Some(original) = self.indexed.as_mut() {
            original.dependencies.clear();
        }
        let file = held
            .0
            .try_clone()
            .map_err(|error| AppError::Io(error.to_string()))?;
        let mut wal = HeldIndexFile {
            name: path,
            file: (file, held.1),
            bytes: self.wal.clone(),
        };
        let ready = disk.finish_indexed_wal(key, &mut wal)?;
        disk.begin_indexed_retirement(&wal, ready)?;
        drop(wal);
        self.indexed_retiring = true;
        drop(self.attempt.take());
        let result = disk.finish_indexed_retirement(key)?;
        self.indexed_retiring = false;
        Ok(result)
    }
}

fn row(
    record: SelectedHistoryRecord,
    admission: OwnerFactId,
    original: &AssembledActionOriginal,
    bundle: &[u8],
) -> SharedArchiveIndexValueData {
    SharedArchiveIndexValueData {
        action: record.action,
        target: projection_target(original.action(), record.action),
        admission,
        manifest: record.manifest,
        bundle_digest: *blake3::hash(bundle).as_bytes(),
        bundle_bytes: bundle.len() as u64,
    }
}
fn successor_bytes(
    old: codec::Head,
    bundle: usize,
    original: &[(NodeRefData, usize)],
    outputs: impl Iterator<Item = usize>,
) -> Result<u64, AppError> {
    let mut bytes = old.bytes.checked_add(bundle as u64).ok_or_else(refused)?;
    for size in outputs {
        bytes = bytes.checked_add(size as u64).ok_or_else(refused)?;
    }
    for (_, size) in original {
        bytes = bytes.checked_sub(*size as u64).ok_or_else(refused)?;
    }
    Ok(bytes)
}

fn require_absent(path: &std::path::Path) -> Result<(), AppError> {
    if observe(path)? != Presence::Absent {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IndexedHistoryFault {
    AfterHead,
    AfterPageRetirement,
    AfterWalUnlink,
}
#[cfg(test)]
thread_local! { pub(super) static FAULT: std::cell::Cell<Option<IndexedHistoryFault>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
fn take_fault(fault: IndexedHistoryFault) -> bool {
    FAULT.with(|slot| {
        if slot.get() == Some(fault) {
            slot.set(None);
            true
        } else {
            false
        }
    })
}
