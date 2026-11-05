//! Current-layout old-key association WIP. Actual installation derives keys only from the
//! already loaded authenticated non-evicting row table. No parsed limit/empty Runtime/caller
//! flag can create an index or prove group vacancy. The actual private constructor installs
//! it; the separate future group consumer still needs original source/pin/head/scope barriers.
use super::*;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use std::cell::Cell;

pub(in crate::store) const KEY_CAPACITY: usize = MAX_CHAT_ADMISSION_IDENTITIES;
pub(in crate::store) const KEY_BYTES: usize = KEY_CAPACITY * std::mem::size_of::<[u8; 32]>();
pub(in crate::store) const INDEX_FEE: usize = KEY_BYTES
    + std::mem::size_of::<LineageIndex>()
    + std::mem::size_of::<blake3::Hasher>()
    + 2 * std::mem::size_of::<usize>() // actual empty identity Arc header
    + 2*(32*1024+256)+2*std::mem::size_of::<std::fs::Metadata>()
    +2*std::mem::size_of::<std::fs::File>(); // held and freshly named directory handles

/// Constructor-only temporary allocation. Keep outside the admission guard so error cleanup
/// returns the actual strong pool fee only after partial vectors/guards have drained.
pub(in crate::store) struct PreparedLineageIndex {
    keys: Vec<[u8; 32]>,
    identity: Arc<()>,
    mount: Arc<()>,
    negative: RequestCancellation,
    fee: CatalogReadReservation,
}

/// Original physical row-key index. Membership grows before first head effects and is NEVER
/// evicted when a row becomes Retired. Invalidating table reloads preserve bytes/fee until
/// normal out-of-writer destruction; they cannot install a fresh empty/clean index in place.
pub(in crate::store) struct LineageIndex {
    keys: Vec<[u8; 32]>,
    identity: Arc<()>,
    mount: Arc<()>,
    negative: RequestCancellation,
    invalid: Cell<bool>,
    generation: Cell<u64>,
    // The actual read-only filename walker alone constructs this opaque complete/unknown
    // observation. Known legal file effects retain its invariant; no cold budget default does.
    association: Option<crate::store::persistence::CurrentCanonicalFilenameAssociation>,
    // LAST: requested vector/metadata and any directory observation drain before refund.
    _fee: CatalogReadReservation,
}
impl LineageIndex {
    pub(in crate::store) fn contains_key_data(&self, key: &[u8; 32]) -> bool {
        !self.invalid.get() && self.keys.binary_search(key).is_ok()
    }
    pub(in crate::store) fn identity_data(&self) -> &Arc<()> {
        &self.identity
    }
    pub(in crate::store) fn generation_data(&self) -> u64 {
        self.generation.get()
    }
    pub(in crate::store) fn same_original(&self, store: &ServerStore) -> bool {
        !self.invalid.get()
            && !self.negative.is_cancelled()
            && Arc::ptr_eq(&self.mount, &store.registry_mount())
    }
    fn invalidate(&self) {
        self.invalid.set(true);
    }
    fn advance(&self) {
        let old = self.generation.get();
        if let Some(next) = old.checked_add(1) {
            if let Some(association) = &self.association {
                association.advance_known(&self.identity, old, next);
            }
            self.generation.set(next);
        } else {
            self.invalidate();
        }
    }

    /// Known-key file effects retain the complete-walk invariant, never repair an unknown one.
    /// Call only from an actual original storage permit before its fixed head I/O, while the
    /// same admission writer is held. Failure latches future V2 vacancy closed but does not
    /// turn an unrelated legitimate V1 save into a new global availability dependency.
    pub(super) fn observe_known_effect(
        &self,
        store: &ServerStore,
        key: &ChatAdmissionKey,
        directory: &Path,
    ) {
        let identifier = crate::store::owner_evidence::canonical::key(key.group(), key.signer());
        if !self.same_original(store) || !self.contains_key_data(&identifier) {
            self.invalidate();
            return;
        }
        if let Some(association) = &self.association {
            association.observe_known_directory(self, store, directory);
        }
        self.advance();
    }

    fn install_association(
        &mut self,
        store: &ServerStore,
        association: crate::store::persistence::CurrentCanonicalFilenameAssociation,
    ) -> Result<(), AppError> {
        if self.association.is_some()
            || !self.same_original(store)
            || !association.belongs_to_index(self, store)
        {
            return Err(refused());
        }
        self.association = Some(association);
        Ok(())
    }

    #[cfg(test)] // positive group constructor must land with its own concrete consumer
    fn require_complete(&self, store: &ServerStore) -> Result<(), AppError> {
        if !self.same_original(store)
            || !self
                .association
                .as_ref()
                .is_some_and(|proof| proof.matches_index(self, store))
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Negative preservation hook for an actual NEW row key. Call at the row's in-memory
    /// insertion point, before any corresponding head/canonical I/O. Existing records never
    /// change membership. A hash collision/inconsistent capacity closes only future V2 vacancy;
    /// it does not revoke already-proven unrelated V1 communications.
    fn observe_new_key(&mut self, key: &ChatAdmissionKey) {
        if self.invalid.get() {
            return;
        }
        let identifier = crate::store::owner_evidence::canonical::key(key.group(), key.signer());
        match self.keys.binary_search(&identifier) {
            Ok(_) => self.invalidate(), // a supposedly NEW full key cannot already be indexed
            Err(offset)
                if self.keys.len() < KEY_CAPACITY && self.keys.capacity() >= KEY_CAPACITY =>
            {
                self.keys.insert(offset, identifier); // fixed preallocated capacity: no growth
                self.advance();
            }
            Err(_) => self.invalidate(),
        }
    }
}

impl Runtime {
    /// Table replacement is not an index refresh. Retain old resource/keys and latch only new
    /// V2 vacancy closed; a newly loaded empty table must never erase old complete-walk debt.
    pub(super) fn invalidate_current_lineage(&self) {
        if let Some(index) = &self.current_lineage {
            index.invalidate();
        }
    }
    pub(super) fn observe_new_current_lineage_key(&mut self, key: &ChatAdmissionKey) {
        if let Some(index) = self.current_lineage.as_mut() {
            index.observe_new_key(key);
        }
    }
}

impl ServerStore {
    /// Private constructor-only setup. Allocate both index and scan scratch outside guards;
    /// retries retain the SAME installed index, never replace a failed/stale proof by empty.
    pub(in crate::store) fn initialize_current_lineage_index(&self) -> Result<(), AppError> {
        if self.current_layout_policy.is_none() {
            return Err(refused());
        }
        let missing = {
            let runtime = self.chat_mount.locked()?;
            runtime.current_lineage.is_none()
        };
        if missing {
            self.install_current_lineage_index(self.prepare_current_lineage_index()?)?;
        }
        let policy = self.current_layout_policy.as_ref().ok_or_else(refused)?;
        let scratch = policy
            .archive
            .pending
            .pool
            .try_reserve(crate::store::persistence::CURRENT_FILENAME_SCRATCH_FEE)?;
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed || runtime.dirty {
            return Err(refused());
        }
        let index = runtime.current_lineage.as_mut().ok_or_else(refused)?;
        if index.association.is_none() {
            let association =
                self.inspect_current_canonical_filenames(index, policy.canonical, &scratch)?;
            index.install_association(self, association)?;
        }
        Ok(())
    }
    /// Real shared-pool reservation and exact requested Vec capacity BEFORE admission. This
    /// is preparation DATA only; row-derived installation and a paid filename walk are separate.
    pub(in crate::store) fn prepare_current_lineage_index(
        &self,
    ) -> Result<PreparedLineageIndex, AppError> {
        let policy = self.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = policy.archive.pending.pool.try_reserve(INDEX_FEE)?;
        let keys = Vec::<[u8; 32]>::with_capacity(KEY_CAPACITY);
        if keys.capacity() != KEY_CAPACITY {
            return Err(refused());
        }
        Ok(PreparedLineageIndex {
            keys,
            identity: Arc::new(()),
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            fee,
        })
    }

    /// Install exactly once while this physical Store is still private. Read the real table
    /// under its writer, not a caller-provided iterator or serialized key list. In-place sort
    /// avoids stable-sort allocation; duplicate canonical identifiers refuse rather than hide
    /// a collision. No canonical directory or HeadCell is allocated by this operation.
    pub(in crate::store) fn install_current_lineage_index(
        &self,
        mut prepared: PreparedLineageIndex,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&prepared.mount, &self.registry_mount())
            || prepared.negative.is_cancelled()
            || self.current_layout_policy.is_none()
        {
            return Err(refused());
        }
        let mut runtime = self.chat_mount.locked()?;
        if !runtime.managed
            || runtime.dirty
            || runtime.format != Some(catcoms_storage::VaultFormat::CriticalNamespaces)
            || runtime.current_lineage.is_some()
        {
            return Err(refused());
        }
        let table = runtime.table.as_ref().ok_or_else(refused)?;
        if table.len() > KEY_CAPACITY || prepared.keys.capacity() < KEY_CAPACITY {
            return Err(refused());
        }
        for key in table.keys() {
            prepared
                .keys
                .push(crate::store::owner_evidence::canonical::key(
                    key.group(),
                    key.signer(),
                ));
        }
        prepared.keys.sort_unstable();
        if prepared.keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(refused());
        }
        // No fallible work or callback after custody is transferred into the actual Runtime.
        runtime.current_lineage = Some(LineageIndex {
            keys: prepared.keys,
            identity: prepared.identity,
            mount: prepared.mount,
            negative: prepared.negative,
            invalid: Cell::new(false),
            generation: Cell::new(0),
            association: None,
            _fee: prepared.fee,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
