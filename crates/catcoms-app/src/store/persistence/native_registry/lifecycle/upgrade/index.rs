//! Dedicated normal3 WAL roles. This child can consume the SAME continuously
//! held inventory without a raw extractor, fresh capture or version2 writer.
use super::*;
use critical::native_registry::{NativeIndex3WalDecision as Decision, VerifiedNativeIndex3Wal};

#[cfg(test)]
thread_local! {
    static CLEANUP_COVERS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
pub(in crate::store) fn take_cleanup_covers() -> u64 {
    CLEANUP_COVERS.with(|count| count.replace(0))
}

/// Static roles shared by normal3 commit and cleanup. Only the native selected
/// index/Stage may change, under their private role mapping below. The genuine
/// catalog, marker, Current pair, root and original mount never change here.
fn require_static(original: &UpgradeInventory, store: &ServerStore) -> Result<(), AppError> {
    original.expected_current.require(store)?;
    store.require_original_physical_mount(&original.mount)?;
    original.base.require_directories(store)?;
    require_no_old_upgrade(store)?;
    // The older declaration helper also forbids the native index Stage. That
    // is correct for an upgrade/declaration, but it is THIS transaction's WAL.
    // Keep all foreign declaration debts absent here; the concrete commit or
    // cleanup phase below separately proves its own Stage identity/absence.
    // Do not weaken the old helper (or allow arbitrary native Stage contents).
    for name in [
        critical::ARCHIVE_DECLARATION_SEED,
        critical::current_group::DECLARATION_SEED,
        critical::governance_layout::GOVERNANCE_SEED,
        critical::member_outbox_layout::MEMBER_OUTBOX_SEED,
    ] {
        absent(&path(store, false, name)?)?;
    }
    if original.seed.is_some()
        || original.catalog_stage.is_some()
        || original.index.is_some()
        || original.marker.is_some()
    {
        return Err(denied());
    }
    for (root, name) in [
        (false, native::SEED_NAME),
        (false, UPGRADE_SEED_NAME),
        (false, critical::Slot::Stage.name()),
        (true, UPGRADE_INDEX_NAME),
        (true, UPGRADE_MARKER_NAME),
    ] {
        absent(&path(store, root, name)?)?;
    }
    original
        .base
        .catalog
        .require(&path(store, false, critical::Slot::Current.name())?)?;
    original
        .base
        .marker
        .require(&path(store, true, native::MARKER_NAME)?)?;
    original.base.require_directories(store)?;
    original.expected_current.require(store)
}

pub(in crate::store) struct IndexCommitIo {
    stage: Option<critical::pending_join::CreatedStage>,
    original: UpgradeInventory,
    promoted: bool,
    complete: bool,
    candidate: [u8; 32],
    #[cfg(test)]
    retirement: Option<RetirementObservation>,
}
impl IndexCommitIo {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        original: UpgradeInventory,
        proof: VerifiedNativeIndex3Wal,
        candidate: &[u8],
    ) -> Result<Self, AppError> {
        original.require(store)?;
        proof.require_inventory(&original)?;
        if !proof.decision().is_commit()
            || proof.has_stage()
            || original.index_stage().is_some()
            || !(40..=MAX_INDEX_FRAME).contains(&candidate.len())
            || proof.candidate() != *blake3::hash(candidate).as_bytes()
        {
            return Err(denied());
        }
        require_static(&original, store)?;
        Ok(Self {
            stage: None,
            original,
            promoted: false,
            complete: false,
            candidate: proof.candidate(),
            #[cfg(test)]
            retirement: None,
        })
    }
    pub(in crate::store) fn recover(
        store: &ServerStore,
        original: UpgradeInventory,
        proof: VerifiedNativeIndex3Wal,
        candidate: &[u8],
    ) -> Result<Self, AppError> {
        original.require(store)?;
        proof.require_inventory(&original)?;
        if !proof.decision().is_commit()
            || !proof.has_stage()
            || original.index_stage() != Some(candidate)
            || !(40..=MAX_INDEX_FRAME).contains(&candidate.len())
            || proof.candidate() != *blake3::hash(candidate).as_bytes()
        {
            return Err(denied());
        }
        require_static(&original, store)?;
        let stage = capture_writable(
            &path(store, true, native::INDEX_STAGE_NAME)?,
            candidate,
            true,
        )?;
        if stage.as_ref().and_then(|held| held.identity)
            != original.base.stage.as_ref().map(HeldNativeInput::identity)
        {
            return Err(denied());
        }
        original.require(store)?;
        Ok(Self {
            stage,
            original,
            promoted: false,
            complete: false,
            candidate: proof.candidate(),
            #[cfg(test)]
            retirement: None,
        })
    }
    pub(in crate::store) fn require_unstarted_parent(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        if self.promoted || self.complete || self.stage.is_some() {
            return Err(denied());
        }
        self.original.require(store)?;
        require_static(&self.original, store)
    }
    pub(in crate::store) fn commit(
        &mut self,
        store: &ServerStore,
        candidate: &[u8],
    ) -> Result<(), AppError> {
        self.complete = false;
        if !(40..=MAX_INDEX_FRAME).contains(&candidate.len())
            || *blake3::hash(candidate).as_bytes() != self.candidate
        {
            return Err(denied());
        }
        require_static(&self.original, store)?;
        let stage = path(store, true, native::INDEX_STAGE_NAME)?;
        let selected = path(store, true, native::INDEX_NAME)?;
        if !self.promoted {
            self.original.base.index.require(&selected)?;
            if self.stage.is_none() {
                absent(&stage)?;
            }
            complete_with_cut(
                &stage,
                &mut self.stage,
                candidate,
                Fault::Index3PartialStage,
            )?;
            trip(Fault::IndexStage)?;
            self.original.base.root.file.sync_all().map_err(io)?;
            require_static(&self.original, store)?;
            exact(&stage, self.stage.as_ref().ok_or_else(denied)?, candidate)?;
            observe_before_target_proof(RenameTarget::Index);
            // The native marker/catalog are part of the original tuple too.
            // Equal bytes in replacement objects must refuse BEFORE selection,
            // not merely be noticed by the post-rename covering proof.
            require_static(&self.original, store)?;
            self.original.base.index.require(&selected)?;
            held(&stage, self.stage.as_ref().ok_or_else(denied)?)?;
            self.original.base.require_directories(store)?;
            self.original.expected_current.require(store)?;
            fs::rename(&stage, &selected).map_err(io)?;
            self.promoted = true; // Visible progress precedes fallible barriers.
            trip(Fault::IndexRename)?;
        }
        let output = self.stage.as_ref().ok_or_else(denied)?;
        exact(&selected, output, candidate)?;
        output.file.sync_all().map_err(io)?;
        self.original.base.root.file.sync_all().map_err(io)?;
        require_static(&self.original, store)?;
        exact(&selected, output, candidate)?;
        absent(&stage)?;
        trip(Fault::Index3Covered)?;
        self.complete = true;
        Ok(())
    }
    pub(in crate::store) fn require_complete(
        &self,
        store: &ServerStore,
        candidate: &[u8],
    ) -> Result<(), AppError> {
        if !self.complete || !self.promoted || *blake3::hash(candidate).as_bytes() != self.candidate
        {
            return Err(denied());
        }
        require_static(&self.original, store)?;
        exact(
            &path(store, true, native::INDEX_NAME)?,
            self.stage.as_ref().ok_or_else(denied)?,
            candidate,
        )?;
        absent(&path(store, true, native::INDEX_STAGE_NAME)?)?;
        self.original.expected_current.require(store)
    }
    #[cfg(test)]
    pub(in crate::store) fn observe_retirement(&mut self, hook: Box<dyn FnOnce() + Send>) {
        self.retirement = Some(RetirementObservation::new(hook));
    }
}

/// Consume only the closed semantic proof against THIS inventory. Neither a
/// remembered error nor an asserted duplicate grants any unlink authority.
pub(in crate::store) struct IndexStageCleanup {
    original: UpgradeInventory,
    removed: bool,
    complete: bool,
}
impl IndexStageCleanup {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        original: UpgradeInventory,
        proof: VerifiedNativeIndex3Wal,
    ) -> Result<Self, AppError> {
        original.require(store)?;
        proof.require_inventory(&original)?;
        if !proof.has_stage()
            || !matches!(
                proof.decision(),
                Decision::Duplicate | Decision::DiscardShort | Decision::DiscardBadMac
            )
        {
            return Err(denied());
        }
        require_static(&original, store)?;
        Ok(Self {
            original,
            removed: false,
            complete: false,
        })
    }
    fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        require_static(&self.original, store)?;
        self.original
            .base
            .index
            .require(&path(store, true, native::INDEX_NAME)?)?;
        let stage = path(store, true, native::INDEX_STAGE_NAME)?;
        if self.removed {
            absent(&stage)?;
        } else {
            self.original
                .base
                .stage
                .as_ref()
                .ok_or_else(denied)?
                .require(&stage)?;
        }
        self.original.expected_current.require(store)
    }
    pub(in crate::store) fn run(&mut self, store: &ServerStore) -> Result<(), AppError> {
        self.complete = false;
        self.require(store)?;
        let stage = path(store, true, native::INDEX_STAGE_NAME)?;
        if !self.removed {
            self.original.base.index.file.sync_all().map_err(io)?;
            self.original.base.root.file.sync_all().map_err(io)?;
            self.require(store)?;
            self.original
                .base
                .stage
                .as_ref()
                .ok_or_else(denied)?
                .require(&stage)?;
            fs::remove_file(&stage).map_err(io)?;
            self.removed = true;
            drop(self.original.base.stage.take());
            trip(Fault::IndexStageUnlink)?;
        }
        self.cover_removed_stage(store)?;
        trip(Fault::Index3CleanupCovered)?;
        self.complete = true;
        Ok(())
    }
    /// Always repeated after an earlier visible unlink. The test observation
    /// belongs to this actual barrier+late-proof path, not a removed flag.
    fn cover_removed_stage(&self, store: &ServerStore) -> Result<(), AppError> {
        absent(&path(store, true, native::INDEX_STAGE_NAME)?)?;
        self.original.base.root.file.sync_all().map_err(io)?;
        self.require(store)?;
        #[cfg(test)]
        CLEANUP_COVERS.with(|count| count.set(count.get() + 1));
        Ok(())
    }
    pub(in crate::store) fn require_complete(&self, store: &ServerStore) -> Result<(), AppError> {
        if !self.complete || !self.removed {
            return Err(denied());
        }
        self.require(store)
    }
}
