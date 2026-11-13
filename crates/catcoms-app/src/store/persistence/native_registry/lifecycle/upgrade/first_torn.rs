//! An attributable empty catalog fence is retired before its first seed.
//! Both unlink flags are cached before fallible barriers, and every retry
//! repeats strict name absence plus the directory barrier. No role is reopened.
use super::*;

#[cfg(test)]
thread_local! {
    static FENCE_COVERS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static BEFORE_SEED_UNLINK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static SEED_HOOK_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
pub(in crate::store) fn reset_fence_covers() {
    FENCE_COVERS.with(|count| count.set(0));
}
#[cfg(test)]
pub(in crate::store) fn fence_covers() -> u64 {
    FENCE_COVERS.with(std::cell::Cell::get)
}
#[cfg(test)]
pub(in crate::store) fn before_seed_unlink(hook: Box<dyn FnOnce()>) {
    BEFORE_SEED_UNLINK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(hook);
    });
    SEED_HOOK_SEEN.with(|seen| seen.set(false));
}
#[cfg(test)]
pub(in crate::store) fn take_seed_hook_seen() -> bool {
    BEFORE_SEED_UNLINK.with(|slot| drop(slot.borrow_mut().take()));
    SEED_HOOK_SEEN.with(|seen| seen.replace(false))
}

pub(in crate::store) struct FirstTornIo {
    original: UpgradeInventory,
    stage_unlinked: bool,
    seed_unlinked: bool,
    preserve_join: bool,
}
impl FirstTornIo {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        inventory: UpgradeInventory,
        proof: critical::native_registry::VerifiedNativeIndex3FirstTorn,
    ) -> Result<Self, AppError> {
        inventory.require(store)?;
        if inventory.has_upgrade_outputs()
            || inventory.base.stage.is_some()
            || !proof.matches(&inventory)
        {
            return Err(denied());
        }
        let value = Self {
            original: inventory,
            stage_unlinked: false,
            seed_unlinked: false,
            preserve_join: proof.preserves_join_stage(),
        };
        value.require(store)?;
        Ok(value)
    }

    /// Once an original role is unlinked its ordinary inventory check cannot
    /// run. Check exactly the same tuple/expectation and only substitute strict
    /// absence for this owner's cached unlink, never for another unknown file.
    fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        let original = &self.original;
        original.expected_current.require(store)?;
        store.require_original_physical_mount(&original.mount)?;
        original.base.require_directories(store)?;
        require_no_old_upgrade(store)?;
        require_no_other_declaration(store)?;
        absent(&path(store, false, native::SEED_NAME)?)?;
        absent(&path(store, true, native::INDEX_STAGE_NAME)?)?;
        absent(&path(store, true, UPGRADE_INDEX_NAME)?)?;
        absent(&path(store, true, UPGRADE_MARKER_NAME)?)?;
        for (root, name, held) in [
            (
                false,
                critical::Slot::Current.name(),
                &original.base.catalog,
            ),
            (true, native::INDEX_NAME, &original.base.index),
            (true, native::MARKER_NAME, &original.base.marker),
        ] {
            held.require(&path(store, root, name)?)?;
        }
        for (name, removed, observed) in [
            (
                critical::Slot::Stage.name(),
                self.stage_unlinked,
                original.catalog_stage.as_ref(),
            ),
            (
                UPGRADE_SEED_NAME,
                self.seed_unlinked,
                original.seed.as_ref(),
            ),
        ] {
            let name = path(store, false, name)?;
            match (removed, observed) {
                (false, Some(held)) => held.require(&name)?,
                (true, None) | (false, None) => absent(&name)?,
                // The flag must advance together with destruction of the
                // original held role, never by reclassifying a new pathname.
                (true, Some(_)) => return Err(denied()),
            }
        }
        original.base.require_directories(store)?;
        original.expected_current.require(store)
    }

    /// Cached unlink progress does not prove its directory barrier succeeded.
    /// Repeat the actual held-directory flush and late tuple proof on EVERY
    /// retry before the seed can be touched, even if the fence is already gone.
    fn cover_catalog_fence_before_seed(&self, store: &ServerStore) -> Result<(), AppError> {
        let stage = path(store, false, critical::Slot::Stage.name())?;
        if self.preserve_join {
            self.original
                .catalog_stage
                .as_ref()
                .ok_or_else(denied)?
                .require(&stage)?;
        } else {
            absent(&stage)?;
        }
        self.original.base.vault.file.sync_all().map_err(io)?;
        self.require(store)?;
        #[cfg(test)]
        FENCE_COVERS.with(|count| count.set(count.get() + 1));
        Ok(())
    }

    pub(in crate::store) fn run(&mut self, store: &ServerStore) -> Result<(), AppError> {
        self.require(store)?;
        let stage = path(store, false, critical::Slot::Stage.name())?;
        let seed = path(store, false, UPGRADE_SEED_NAME)?;
        if !self.preserve_join && !self.stage_unlinked {
            if let Some(original) = &self.original.catalog_stage {
                original.require(&stage)?;
                self.require(store)?;
                fs::remove_file(&stage).map_err(io)?;
            }
            self.stage_unlinked = true;
            drop(self.original.catalog_stage.take());
            trip(Fault::UpgradeFenceUnlink)?;
        }
        self.cover_catalog_fence_before_seed(store)?;
        trip(Fault::UpgradeFenceUnlinkCovered)?;
        // The seed cannot disappear before the fence's covering barrier. This
        // ordering also lets cold recovery attribute an already-absent fence.
        if !self.seed_unlinked {
            self.original
                .seed
                .as_ref()
                .ok_or_else(denied)?
                .require(&seed)?;
            self.require(store)?;
            #[cfg(test)]
            if let Some(hook) = BEFORE_SEED_UNLINK.with(|slot| slot.borrow_mut().take()) {
                SEED_HOOK_SEEN.with(|seen| seen.set(true));
                hook();
                self.require(store)?;
            }
            fs::remove_file(&seed).map_err(io)?;
            self.seed_unlinked = true;
            drop(self.original.seed.take());
            trip(Fault::UpgradeSeedUnlink)?;
        }
        absent(&seed)?;
        self.original.base.vault.file.sync_all().map_err(io)?;
        self.require(store)
    }
}
