//! Explicit governed cold storage, with no signer/Source/native command or
//! facade conversion. Operational cold authoring needs its separate owner.
use super::super::lifecycle::preparation::Policy;
use super::*;
use crate::store::persistence::governance_layout::joined::JoinedPinDeclaration;
use catcoms_rt::CryptoRngCore;

/// Original opaque combined4+5+native3 physical custody. Metadata reads do not
/// reconstruct a signer, reuse fresh Welcome evidence or grant a native caller.
pub struct NativeTextGovernedStorage {
    joined_declaration: Option<JoinedPinDeclaration>,
    index: upgrade_source::owning::Store,
}
impl std::fmt::Debug for NativeTextGovernedStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextGovernedStorage")
            .finish_non_exhaustive()
    }
}
impl NativeTextGovernedStorage {
    /// New explicit configured session only; no fallback to a less capable
    /// opener on error. Complete declaration debt resumes exactly; an aborted
    /// first claim can redeclare only on the independently pristine baseline.
    pub fn open(
        dir: impl AsRef<std::path::Path>,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        let policy = Policy::new(
            CriticalCatalogReadPool::new(super::super::lifecycle::preparation::PREPARATION_FEE)?,
            Policy::required_persistent_capacity(),
        )?;
        Ok(Self {
            joined_declaration: None,
            index: upgrade_source::owning::Store::open_governed(dir, secret, rng, current, policy)?,
        })
    }
    /// Authenticated metadata only; neither zero nor a positive count is live
    /// membership, a complete projection or permission to author.
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.native_row_count_data()
    }

    /// Stage the declaration on this pristine physical owner. Drop this
    /// metadata owner before opening the selected joined-aware storage.
    pub fn stage_joined_pin_layout(
        &mut self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.index
            .stage_joined_pin_layout(&mut self.joined_declaration, rng)
    }
}

/// Explicit custody of the joined-pin catalog and its existing native leaves.
/// Opening this metadata owner cannot install a joined MLS runtime.
pub struct NativeTextJoinedStorage {
    index: upgrade_source::owning::Store,
}

impl NativeTextJoinedStorage {
    pub fn open(
        dir: impl AsRef<std::path::Path>,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        let policy = Policy::new(
            CriticalCatalogReadPool::new(super::super::lifecycle::preparation::PREPARATION_FEE)?,
            Policy::required_persistent_capacity(),
        )?;
        Ok(Self {
            index: upgrade_source::owning::Store::open_joined(dir, secret, rng, current, policy)?,
        })
    }

    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.native_row_count_data()
    }

    /// Historical pin context from the current mounted index.
    pub fn joined_pin_context_data(
        &self,
        anchor: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        self.index.selected_joined_pin_context_data(anchor)
    }
}
