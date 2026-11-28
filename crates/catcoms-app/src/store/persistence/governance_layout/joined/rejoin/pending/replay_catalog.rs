//! Separate a selected catalog that may still admit a rejoin from the
//! historical binding of an already installed A2. The installed replay
//! scope is issued only inside the fixed Index4 recovery path.
use super::*;
use rejoin_namespace::{inspect_installed_catalog_binding, InstalledCatalogBindingData};
use std::io::{Read, Seek, SeekFrom};

const REVIEW_PREDECESSOR: &str = "chat-critical-namespaces.rejoin-review-parent.bin";
const MAX_CATALOG: usize = critical_namespaces::pending_join_codec::MAX_CATALOG_FRAMED;
const PREDECESSOR_CHECK_FEE: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(super) enum RejoinReadCatalog<'a> {
    Current {
        inventory: &'a UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
    },
    Installed(&'a InstalledReplayCatalog<'a>),
}

impl<'a> RejoinReadCatalog<'a> {
    pub(super) fn parts(
        self,
    ) -> (
        &'a UpgradeInventory,
        (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
    ) {
        match self {
            Self::Current {
                inventory,
                original,
            } => (inventory, original),
            Self::Installed(original) => (original.inventory, original.original),
        }
    }

    pub(super) fn existing_only(self) -> bool {
        matches!(self, Self::Installed(_))
    }

    pub(super) fn digest(self, store: &ServerStore) -> Result<[u8; 32], AppError> {
        match self {
            Self::Current {
                inventory,
                original: (registration, removal),
            } => store.cover_selected_rejoin_catalog(registration, removal, inventory),
            Self::Installed(replay) => {
                replay.require_current(store)?;
                Ok(replay.birth_digest)
            }
        }
    }

    /// An existing replay must consume the exact files taken into custody
    /// before the original binding was issued. Equal bytes from a replacement
    /// file cannot provide a fresh Pending, graph or native registry.
    pub(super) fn require_leaf(
        self,
        name: &'static str,
        observed: (&File, &[u8]),
    ) -> Result<(), AppError> {
        let Self::Installed(replay) = self else {
            return Ok(());
        };
        let original = replay
            .leaves
            .iter()
            .find(|(leaf, _)| *leaf == name)
            .map(|(_, value)| value)
            .ok_or_else(denied)?;
        require_original_leaf(original, observed)
    }
}

fn require_original_leaf(
    original: &OpenedArchiveObject,
    observed: (&File, &[u8]),
) -> Result<(), AppError> {
    if session_id(observed.0).map_err(io)? != session_id(&original.0).map_err(io)?
        || observed.1 != original.1.as_slice()
    {
        return Err(denied());
    }
    Ok(())
}

pub(super) struct InstalledReplayCatalog<'a> {
    inventory: &'a UpgradeInventory,
    original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
    leaves: Vec<(&'static str, OpenedArchiveObject)>,
    birth_digest: [u8; 32],
    witness: InstalledCatalogWitness,
    _physical: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

/// Move this proof into the actual installed owner after fixed replay. The
/// pending binding digest stays private to replay and the original Current
/// remains independently owned by the same native inventory.
pub(super) struct InstalledCatalogWitness(Option<ReplayPredecessor>);

struct ReplayPredecessor {
    current_id: SessionId,
    current_digest: [u8; 32],
    original: RetainedPredecessorFile,
    _physical: catcoms_storage::VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

struct RetainedPredecessorFile {
    file: File,
    id: SessionId,
    len: usize,
    digest: [u8; 32],
}

impl RetainedPredecessorFile {
    /// This digest is verified against the original encrypted bytes by the
    /// installed replay reader. The same held file has to match before use.
    fn new(
        path: &Path,
        (file, bytes): OpenedArchiveObject,
        digest: [u8; 32],
    ) -> Result<Self, AppError> {
        if !(41..=MAX_CATALOG).contains(&bytes.len()) {
            return Err(denied());
        }
        let value = Self {
            id: session_id(&file).map_err(io)?,
            file,
            len: bytes.len(),
            digest,
        };
        value.require(path)?;
        Ok(value)
    }

    fn require(&self, path: &Path) -> Result<(), AppError> {
        held(path, &self.file, self.id)?;
        let mut file = &self.file;
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let mut bytes = [0; 8192];
        let mut hash = blake3::Hasher::new();
        let mut left = self.len;
        while left != 0 {
            let count = left.min(bytes.len());
            file.read_exact(&mut bytes[..count]).map_err(io)?;
            hash.update(&bytes[..count]);
            left -= count;
        }
        if file.read(&mut bytes[..1]).map_err(io)? != 0
            || hash.finalize().as_bytes() != &self.digest
        {
            return Err(denied());
        }
        held(path, &self.file, self.id)
    }
}

impl InstalledCatalogWitness {
    pub(super) fn require(
        &self,
        store: &ServerStore,
        inventory: &UpgradeInventory,
    ) -> Result<(), AppError> {
        let path = fixed(store, false, REVIEW_PREDECESSOR)?;
        let Some(parent) = self.0.as_ref() else {
            return absent(&path);
        };
        inventory.require(store)?;
        if inventory.catalog_identity() != parent.current_id
            || blake3::hash(inventory.catalog()).as_bytes() != &parent.current_digest
        {
            return Err(denied());
        }
        parent.original.require(&path)?;
        inventory.require(store)
    }

    /// A2 MemberAdds needs an additional admission check only when it was
    /// restored from a checked linked catalog. Direct ID9 still uses the
    /// existing path for each ordinary publication.
    pub(super) fn require_linked(
        &self,
        store: &ServerStore,
        inventory: &UpgradeInventory,
    ) -> Result<(), AppError> {
        if self.0.is_some() {
            self.require(store, inventory)?;
        }
        Ok(())
    }
}

impl<'a> InstalledReplayCatalog<'a> {
    pub(super) fn capture(
        store: &ServerStore,
        inventory: &'a UpgradeInventory,
        original: (NativeTextJoinedRegistrationData, SelectedRejoinRemoval),
        names: [(&'static str, usize); 7],
    ) -> Result<Self, AppError> {
        // Keep the original fixed bytes and pay for one concurrent exact
        // comparison. The native inventory already owns paid Current.
        let bytes = names
            .iter()
            .try_fold(32usize * 1024, |total, (_, bound)| {
                total.checked_add(bound.checked_mul(2)?)
            })
            .ok_or_else(denied)?;
        let pool = &store
            .governance_layout_policy
            .as_ref()
            .ok_or_else(denied)?
            .current
            .archive
            .pending
            .pool;
        let fee = pool.try_reserve(bytes)?;
        let physical = store._session.retain_for_physical_work();
        inventory.require(store)?;
        let mut leaves = Vec::with_capacity(names.len());
        for (name, bound) in names {
            let frame = read(&fixed(store, false, name)?, bound)?.ok_or_else(denied)?;
            leaves.push((name, frame));
        }
        let (birth_digest, predecessor) = store.with_selected_rejoin_catalog(
            original.0,
            original.1,
            inventory,
            |key, current, domain| {
                let path = fixed(store, false, REVIEW_PREDECESSOR)?;
                match inspect_installed_catalog_binding(key, current, domain)? {
                    InstalledCatalogBindingData::Direct(digest) => {
                        absent(&path)?;
                        Ok((digest, None))
                    }
                    InstalledCatalogBindingData::ReviewSuccessor(parent) => {
                        let fee = pool.try_reserve(PREDECESSOR_CHECK_FEE)?;
                        let _scratch = pool.try_reserve(4 * MAX_CATALOG + 16 * 1024)?;
                        let file = read(&path, MAX_CATALOG)?.ok_or_else(denied)?;
                        let digest =
                            parent.verify_installed_parent(key, &file.1, current, domain)?;
                        Ok((
                            digest,
                            Some(ReplayPredecessor {
                                current_id: inventory.catalog_identity(),
                                current_digest: *blake3::hash(current).as_bytes(),
                                original: RetainedPredecessorFile::new(&path, file, digest)?,
                                _physical: store._session.retain_for_physical_work(),
                                _fee: fee,
                            }),
                        ))
                    }
                }
            },
        )?;
        let replay = Self {
            inventory,
            original,
            leaves,
            birth_digest,
            witness: InstalledCatalogWitness(predecessor),
            _physical: physical,
            _fee: fee,
        };
        replay.require(store)?;
        Ok(replay)
    }

    fn require_current(&self, store: &ServerStore) -> Result<(), AppError> {
        store.require_recovered_joined_removal(self.original.0, self.original.1)?;
        absent(&fixed(store, false, REJOIN_STAGE)?)?;
        self.witness.require(store, self.inventory)?;
        self.inventory.require(store)
    }

    pub(super) fn require(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require_current(store)?;
        for (name, (file, bytes)) in &self.leaves {
            exact_held(
                &fixed(store, false, name)?,
                file,
                session_id(file).map_err(io)?,
                bytes,
            )?;
        }
        self.require_current(store)
    }

    pub(super) fn into_witness(self) -> InstalledCatalogWitness {
        self.witness
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_rejoin_catalog_leaf_rejects_an_equal_byte_file_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("rejoin-original.bin");
        let parked = directory.path().join("rejoin-original.old");
        let bytes = [0xc0; 64];
        std::fs::write(&live, bytes).unwrap();
        let original = read(&live, bytes.len()).unwrap().unwrap();
        let unchanged = read(&live, bytes.len()).unwrap().unwrap();
        require_original_leaf(&original, (&unchanged.0, &unchanged.1)).unwrap();
        assert!(require_original_leaf(&original, (&unchanged.0, &bytes[..63])).is_err());
        std::fs::rename(&live, &parked).unwrap();
        std::fs::copy(&parked, &live).unwrap();
        let replacement = read(&live, bytes.len()).unwrap().unwrap();
        assert!(require_original_leaf(&original, (&replacement.0, &replacement.1)).is_err());
    }

    #[test]
    fn installed_rejoin_retained_predecessor_checks_exact_bytes_and_file() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join(REVIEW_PREDECESSOR);
        let parked = directory.path().join("rejoin-review-parent.old");
        // Exercise more than one streaming scratch window.
        let bytes = vec![0xbb; 8193];
        std::fs::write(&live, &bytes).unwrap();
        let original = read(&live, bytes.len()).unwrap().unwrap();
        let digest = *blake3::hash(&bytes).as_bytes();
        let retained = RetainedPredecessorFile::new(&live, original, digest).unwrap();
        retained.require(&live).unwrap();

        let mut changed = bytes.clone();
        *changed.last_mut().unwrap() ^= 1;
        std::fs::write(&live, &changed).unwrap();
        assert!(
            retained.require(&live).is_err(),
            "a same-length in-place change"
        );
        std::fs::write(&live, &bytes[..8192]).unwrap();
        assert!(retained.require(&live).is_err(), "a truncated frame");
        changed = bytes.clone();
        changed.push(0);
        std::fs::write(&live, &changed).unwrap();
        assert!(retained.require(&live).is_err(), "a trailing byte");

        std::fs::write(&live, &bytes).unwrap();
        retained.require(&live).unwrap();
        std::fs::rename(&live, &parked).unwrap();
        assert!(retained.require(&live).is_err(), "a missing original name");
        std::fs::copy(&parked, &live).unwrap();
        assert!(
            retained.require(&live).is_err(),
            "an equal-byte replacement"
        );
    }
}
