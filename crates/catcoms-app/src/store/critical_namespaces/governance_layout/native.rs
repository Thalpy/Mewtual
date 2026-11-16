//! Constructor-private same-owner native3 -> governed native3 preparation.
//! There is no raw Store getter, Source/SDK issuer or arbitrary facade adopter.
use super::super::native_registry::{lifecycle, preparation::NativeRegistryHandler};
use super::*;

#[derive(Default)]
pub(in crate::store::critical_namespaces) struct Startup {
    declaration: Option<Box<Declaration>>,
    initial: Option<lifecycle::VerifiedGovernanceNative>,
    fee: Option<CatalogReadReservation>,
    policy: Option<V2GovernanceLayoutPolicy>,
    complete: bool,
}
impl Startup {
    pub(in crate::store::critical_namespaces) fn is_complete(&self) -> bool {
        self.complete
    }
    /// Last fallible operation before consuming the whole startup owner.
    /// Keep the original declaration witnesses through every earlier metadata
    /// read; no fresh capture can replace them at the final handoff.
    pub(in crate::store::critical_namespaces) fn require_complete(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        if !self.complete {
            return Err(refused());
        }
        store.validate_mounted_governance_layout()?;
        store.require_native_governance_selection(self.declaration.as_deref().ok_or_else(refused)?)
    }

    /// The only caller lends the unique original Selected3 Current from its
    /// unexposed whole startup owner. All physical work happens outside native
    /// guards. Every refusal retains this exact plan and its original files.
    pub(in crate::store::critical_namespaces) fn advance(
        &mut self,
        store: &mut ServerStore,
        negative: &mut Option<GovernanceCachedNegative>,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let current = store.current_layout_policy.as_ref().ok_or_else(refused)?;
        let handler = current
            .native_registry
            .as_ref()
            .and_then(|n| n.lifecycle())
            .ok_or_else(refused)?
            .clone();
        handler.require()?;
        handler.require_idle_cached()?;
        if self.complete {
            if !current
                .native_registry
                .as_ref()
                .is_some_and(|h| h.is_governed())
                || negative.as_ref().is_none_or(|n| !n.is_live_cached())
            {
                return Err(refused());
            }
            return self.require_complete(store);
        }
        if self.policy.is_none() {
            if store.governance_layout_policy.is_some()
                || !matches!(
                    current.native_registry.as_ref(),
                    Some(NativeRegistryHandler::Lifecycle(_))
                )
            {
                return Err(refused());
            }
            self.policy = Some(V2GovernanceLayoutPolicy::for_native(current.clone())?);
        }
        let policy = self.policy.as_ref().ok_or_else(refused)?;
        if store.governance_layout_policy.is_none() {
            if self.initial.is_none() {
                let inventory=crate::store::persistence::native_registry::lifecycle::governed::NativeGovernanceInventory::capture(store,policy.native_pool.as_ref().ok_or_else(refused)?)?;
                inventory.require_original_catalog(store)?;
                let original = Zeroizing::new(inventory.catalog().to_vec());
                self.initial = Some(lifecycle::verify_governance_native(
                    store, inventory, &original, None,
                )?);
            }
            self.initial
                .as_ref()
                .ok_or_else(refused)?
                .require_initial_catalog(store)?;
            // No exposed owner is upgraded: the enclosing native continuation
            // was consumed before entry and can no longer yield ordinary3.
            store.governance_layout_policy = Some(policy.clone());
        } else {
            let actual = store
                .governance_layout_policy
                .as_ref()
                .ok_or_else(refused)?;
            if actual.family != policy.family
                || !actual
                    .native_pool
                    .as_ref()
                    .ok_or_else(refused)?
                    .same_account(policy.native_pool.as_ref().ok_or_else(refused)?)
            {
                return Err(refused());
            }
        }
        if self.declaration.is_none() {
            if self.fee.is_none() {
                self.fee = Some(
                    policy
                        .current
                        .archive
                        .pending
                        .pool
                        .try_reserve(codec::DECLARATION_FEE)?,
                );
            }
            let declaration = store.prepare_governance_layout_declaration_with_native(
                self.fee.as_ref().ok_or_else(refused)?,
                rng,
                &mut self.initial,
            )?;
            self.declaration = Some(Box::new(declaration));
        }
        let declaration = self.declaration.as_mut().ok_or_else(refused)?;
        store.write_governance_layout_declaration(
            declaration,
            self.fee.as_ref().ok_or_else(refused)?,
        )?;
        let marker = store.capture_governance_layout_marker(
            declaration.capability,
            declaration.marker_file.as_ref().ok_or_else(refused)?,
        )?;
        store.install_governance_layout_marker(marker)?;
        {
            let mut guard = store.pending_join_catalog.lock().map_err(|_| refused())?;
            let catalog = guard.as_mut().ok_or_else(refused)?;
            if catalog.pending.is_some()
                || (catalog.current.framed != declaration.before.framed
                    && catalog.current.framed != declaration.next.framed)
            {
                return Err(refused());
            }
            catalog.current = declaration.next.clone();
        }
        // Capture real mounted4 before switching parsers. No readiness scalar
        // or decoded descriptor can replace the actual marker's lifetime.
        if negative.is_none() {
            *negative = Some(store.capture_governance_cached_negative()?);
        }
        store.require_native_governance_selection(declaration)?;
        let governed = NativeRegistryHandler::GovernedLifecycle(handler);
        store
            .current_layout_policy
            .as_mut()
            .ok_or_else(refused)?
            .native_registry = Some(governed.clone());
        store
            .governance_layout_policy
            .as_mut()
            .ok_or_else(refused)?
            .current
            .native_registry = Some(governed);
        self.complete = true;
        // Cache completion but retain the original debt and payment through
        // into_vault's last row/index/marker waits. A retry only reproves this
        // same selection; it never creates another declaration or nonce.
        self.require_complete(store)
    }
}
