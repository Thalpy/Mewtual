//! Closed normal3 Stage classification. Successful authentication is distinct
//! from syntax support; a future/unsupported authenticated operation is kept.
use super::{
    capability::Capability,
    data::{Backend, BirthKind, Change, Index},
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
    FounderBirth,
    FounderActivate,
    Rename,
    RefreshLegacyInvite,
    Duplicate,
    DiscardShort,
    DiscardBadMac,
}

impl Decision {
    /// The only implemented content-bearing normal3 WAL operations. Cleanup
    /// decisions and future Change variants must never enter this IO role.
    pub(in crate::store) fn is_commit(self) -> bool {
        matches!(
            self,
            Self::FounderBirth | Self::FounderActivate | Self::Rename | Self::RefreshLegacyInvite
        )
    }
}
// `successor_data` describes the whole future grammar. It is deliberately
// wider than the actual writing/recovery producers. Inspect BOTH authenticated
// rows so supporting a founder cannot accidentally activate a joined/legacy
// source whose namespace/journal/own-Add authority is not implemented here.
fn supported_decision(
    selected: &Index,
    candidate: &Index,
    parent: [u8; 32],
) -> Result<Decision, AppError> {
    match selected.successor_data(candidate, parent)? {
        Change::Birth
            if matches!(candidate.rows.last().map(|r| &r.backend),
            Some(Backend::Pending { birth, .. }) if birth.kind.is_founder() && birth.join_context.is_none()) =>
        {
            Ok(Decision::FounderBirth)
        }
        Change::Activate
            if selected.rows.iter().zip(&candidate.rows).any(|(old, new)| {
                old != new
                    && matches!((&old.backend, &new.backend),
                (Backend::Pending { birth, .. }, Backend::Founder { .. })
                    if birth.kind.is_founder() && birth.join_context.is_none())
            }) =>
        {
            Ok(Decision::FounderActivate)
        }
        Change::Rename => Ok(Decision::Rename),
        Change::RefreshLegacyInvite => Ok(Decision::RefreshLegacyInvite),
        _ => Err(refused()),
    }
}

/// Affine semantic proof for THIS held inventory/selected frame/Stage. Exposing
/// its inert decision cannot construct another proof or native write authority.
pub(in crate::store) struct VerifiedWal {
    original: FirstTornInventoryWitness,
    selected: [u8; 32],
    observed_stage: Option<[u8; 32]>,
    candidate: [u8; 32],
    decision: Decision,
    network: Option<
        std::sync::Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
    >,
}
impl VerifiedWal {
    pub(in crate::store) fn take_network(
        &mut self,
    ) -> Option<
        std::sync::Arc<crate::store::persistence::native_registry::network::NetworkDependency>,
    > {
        self.network.take()
    }
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
        network: None,
    }
}

/// Cold native Stage and JOIN Stage may not coexist. Initially the concrete
/// normal producer supports founder Birth/Activate, Rename and Legacy invite refresh; other authenticated Change kinds are
/// preserved/refused until their actual owning producers are implemented.
pub(super) fn classify(
    store: &ServerStore,
    inventory: &UpgradeInventory,
) -> Result<VerifiedWal, AppError> {
    require_normal_inventory(store, inventory)?;
    let (selected, _) = preparation::decode_selected_data(store, inventory)?;
    let stage = inventory.index_stage().ok_or_else(refused)?;
    let mut network = None;
    let decision = if stage == inventory.index() {
        Decision::Duplicate
    } else if stage.len() < 40 {
        Decision::DiscardShort
    } else {
        let key = Zeroizing::new(store.keys.db_key()?);
        match open_index(&key, stage, selected.capability) {
            Ok(next) => {
                let decision = supported_decision(
                    &selected,
                    &next,
                    *blake3::hash(inventory.index()).as_bytes(),
                )?;
                if selected.rows.iter().chain(&next.rows).any(|row| {
                    row.backend
                        .birth()
                        .is_some_and(|birth| birth.kind == BirthKind::NativeFounder)
                }) {
                    let handler = native_handler(store)?;
                    // Pending Birth may lack a file; validate its authenticated
                    // reservation DATA without granting cold execution.
                    network = super::network::capture_activation(store, &selected, &next, handler)?;
                    super::network::require_birth_candidate(store, &selected, &next)?;
                }
                decision
            }
            Err(ReadError::Unauthenticated(_)) => Decision::DiscardBadMac,
            Err(error) => return Err(error.into_error()),
        }
    };
    inventory.require(store)?;
    let mut proof = proof(inventory, stage, decision);
    proof.network = network;
    Ok(proof)
}

fn native_handler(store: &ServerStore) -> Result<&super::admission::Handler, AppError> {
    match store
        .current_layout_policy
        .as_ref()
        .and_then(|policy| policy.native_registry.as_ref())
    {
        Some(super::super::preparation::NativeRegistryHandler::Lifecycle(handler)) => Ok(handler),
        _ => Err(refused()),
    }
}

/// Issue warm IO proof only for the actual selected3 parent and the SAME
/// sealed canonical candidate. Caller keeps all preparation inside its paid
/// cell; neither the candidate nor a remembered hash authorizes physical IO.
pub(super) fn verify_warm_metadata(
    store: &ServerStore,
    inventory: &UpgradeInventory,
    candidate: &Index,
    bytes: &[u8],
    expected: Change,
) -> Result<VerifiedWal, AppError> {
    require_normal_inventory(store, inventory)?;
    if inventory.index_stage().is_some() {
        return Err(refused());
    }
    let (selected, _) = preparation::decode_selected_data(store, inventory)?;
    let key = Zeroizing::new(store.keys.db_key()?);
    let sealed = open_index(&key, bytes, selected.capability).map_err(ReadError::into_error)?;
    let actual = selected.successor_data(&sealed, *blake3::hash(inventory.index()).as_bytes())?;
    if sealed != *candidate || actual != expected {
        return Err(refused());
    }
    inventory.require(store)?;
    Ok(proof(
        inventory,
        bytes,
        supported_decision(
            &selected,
            &sealed,
            *blake3::hash(inventory.index()).as_bytes(),
        )?,
    ))
}

#[cfg(test)]
pub(in crate::store::critical_namespaces::native_registry) mod tests;
