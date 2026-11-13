//! Closed normal3 Stage classification. Successful authentication is distinct
//! from syntax support; a future/unsupported authenticated operation is kept.
use super::{
    capability::Capability,
    data::{Change, Index},
    preparation, *,
};
use crate::store::persistence::native_registry::lifecycle::upgrade::{
    FirstTornInventoryWitness, UpgradeInventory,
};
use crate::store::{unframe, ServerStore};
use catcoms_crypto::unseal;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::store) enum Decision {
    Rename,
    Duplicate,
    DiscardShort,
    DiscardBadMac,
}

/// Affine semantic proof for THIS held inventory/selected frame/Stage. Exposing
/// its inert decision cannot construct another proof or native write authority.
pub(in crate::store) struct VerifiedWal {
    original: FirstTornInventoryWitness,
    selected: [u8; 32],
    observed_stage: Option<[u8; 32]>,
    candidate: [u8; 32],
    decision: Decision,
}
impl VerifiedWal {
    pub(in crate::store) fn decision(&self) -> Decision {
        self.decision
    }
    pub(in crate::store) fn candidate(&self) -> [u8; 32] {
        self.candidate
    }
    pub(in crate::store) fn has_stage(&self) -> bool {
        self.observed_stage.is_some()
    }
    pub(in crate::store) fn require_inventory(
        &self,
        inventory: &UpgradeInventory,
    ) -> Result<(), AppError> {
        if !inventory.matches_first_torn_witness(&self.original)
            || self.selected != *blake3::hash(inventory.index()).as_bytes()
            || self.observed_stage
                != inventory
                    .index_stage()
                    .map(|raw| *blake3::hash(raw).as_bytes())
        {
            return Err(refused());
        }
        Ok(())
    }
}

enum ReadError {
    Indeterminate(AppError),
    Unauthenticated(AppError),
    Authenticated(AppError),
}
impl ReadError {
    fn into_error(self) -> AppError {
        match self {
            Self::Indeterminate(e) | Self::Unauthenticated(e) | Self::Authenticated(e) => e,
        }
    }
}
fn open_index(key: &[u8; 32], bytes: &[u8], cap: Capability) -> Result<Index, ReadError> {
    if !(40..=super::codec::MAX_FRAME).contains(&bytes.len()) {
        return Err(ReadError::Indeterminate(refused()));
    }
    let framed = unframe(bytes).map_err(ReadError::Indeterminate)?;
    let derived = Zeroizing::new(blake3::derive_key(preparation::INDEX_KEY, key));
    let plain = Zeroizing::new(
        unseal(&derived, &framed).map_err(|error| ReadError::Unauthenticated(error.into()))?,
    );
    Index::decode_data(&plain, cap).map_err(ReadError::Authenticated)
}
fn require_normal_inventory(
    store: &ServerStore,
    inventory: &UpgradeInventory,
) -> Result<(), AppError> {
    inventory.require(store)?;
    if inventory.seed().is_some()
        || inventory.has_upgrade_outputs()
        || inventory.catalog_stage().is_some()
    {
        return Err(refused());
    }
    Ok(())
}
fn proof(inventory: &UpgradeInventory, candidate: &[u8], decision: Decision) -> VerifiedWal {
    VerifiedWal {
        original: inventory.first_torn_witness(),
        selected: *blake3::hash(inventory.index()).as_bytes(),
        observed_stage: inventory
            .index_stage()
            .map(|raw| *blake3::hash(raw).as_bytes()),
        candidate: *blake3::hash(candidate).as_bytes(),
        decision,
    }
}

/// Cold native Stage and JOIN Stage may not coexist. Initially the concrete
/// normal producer supports Rename only; other authenticated Change kinds are
/// preserved/refused until their actual owning producers are implemented.
pub(super) fn classify(
    store: &ServerStore,
    inventory: &UpgradeInventory,
) -> Result<VerifiedWal, AppError> {
    require_normal_inventory(store, inventory)?;
    let (selected, _) = preparation::decode_selected_data(store, inventory)?;
    let stage = inventory.index_stage().ok_or_else(refused)?;
    let decision = if stage == inventory.index() {
        Decision::Duplicate
    } else if stage.len() < 40 {
        Decision::DiscardShort
    } else {
        let key = Zeroizing::new(store.keys.db_key()?);
        match open_index(&key, stage, selected.capability) {
            Ok(next) => {
                if selected.successor_data(&next, *blake3::hash(inventory.index()).as_bytes())?
                    != Change::Rename
                {
                    return Err(refused());
                }
                Decision::Rename
            }
            Err(ReadError::Unauthenticated(_)) => Decision::DiscardBadMac,
            Err(error) => return Err(error.into_error()),
        }
    };
    inventory.require(store)?;
    Ok(proof(inventory, stage, decision))
}

/// Issue warm IO proof only for the actual selected3 parent and the SAME
/// sealed canonical candidate. Caller keeps all preparation inside its paid
/// cell; neither the candidate nor a remembered hash authorizes physical IO.
pub(in crate::store::critical_namespaces::native_registry) fn verify_warm_rename(
    store: &ServerStore,
    inventory: &UpgradeInventory,
    candidate: &Index,
    bytes: &[u8],
) -> Result<VerifiedWal, AppError> {
    require_normal_inventory(store, inventory)?;
    if inventory.index_stage().is_some() {
        return Err(refused());
    }
    let (selected, _) = preparation::decode_selected_data(store, inventory)?;
    let key = Zeroizing::new(store.keys.db_key()?);
    let sealed = open_index(&key, bytes, selected.capability).map_err(ReadError::into_error)?;
    if sealed != *candidate
        || selected.successor_data(candidate, *blake3::hash(inventory.index()).as_bytes())?
            != Change::Rename
    {
        return Err(refused());
    }
    inventory.require(store)?;
    Ok(proof(inventory, bytes, Decision::Rename))
}

#[cfg(test)]
pub(super) mod tests;
