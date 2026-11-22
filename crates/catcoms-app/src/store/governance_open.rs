//! Closed constructor configuration shared by old and new private physical openers.
//! These fields are trusted Rust policy, not serialized support flags or authority.
use super::*;

pub(super) struct CatalogPolicies {
    pub(super) catalog_read_pool: Option<CriticalCatalogReadPool>,
    pub(super) pending_join_policy: Option<PendingJoinCatalogPolicy>,
    pub(super) archive_bootstrap_policy: Option<ArchiveBootstrapCatalogPolicy>,
    pub(super) current_layout_policy:
        Option<critical_namespaces::current_layout::V2CurrentLayoutPolicy>,
    pub(super) governance_layout_policy:
        Option<critical_namespaces::governance_layout::V2GovernanceLayoutPolicy>,
}

impl CatalogPolicies {
    pub(super) fn requires_existing_joined_vault(&self) -> bool {
        self.current_layout_policy
            .as_ref()
            .and_then(|policy| policy.native_registry.as_ref())
            .is_some_and(|handler| handler.uses_joined_pin_layout())
    }
}

impl ServerStore {
    /// Only the new opaque private physical constructor calls this; no exposed
    /// Store is upgraded and no Source/Scope or generic SDK operation is issued.
    pub(in crate::store) fn open_with_governance_catalog_policy(
        dir: impl AsRef<Path>,
        passphrase: &[u8],
        rng: &mut impl CryptoRngCore,
        policy: critical_namespaces::governance_layout::V2GovernanceLayoutPolicy,
    ) -> Result<Self, AppError> {
        policy.require_persistent_capacity()?;
        let current = policy.current.clone();
        Self::open_with_catalog_policies(
            dir,
            passphrase,
            rng,
            CatalogPolicies {
                catalog_read_pool: Some(current.archive.pending.pool.clone()),
                pending_join_policy: Some(current.archive.pending.clone()),
                archive_bootstrap_policy: Some(current.archive.clone()),
                current_layout_policy: Some(current),
                governance_layout_policy: Some(policy),
            },
        )
    }
}
