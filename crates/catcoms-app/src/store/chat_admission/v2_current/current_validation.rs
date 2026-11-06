//! Final read-only edge AFTER caller-owned temporary scope credits/retired metadata have
//! drained outside writers. Paired with the actual Current owner; this is no dirty bypass,
//! scope flush, head callback, binding constructor or replacement-owner path.
use super::*;

impl V2CurrentPreparation {
    /// Negatively close ONLY this original scope before Root drops its node/source payloads.
    /// The actual owning Pending/Current Drop path calls this before payload/account cleanup.
    /// It allocates no new trait wrapper and cannot affect a replacement owner at this key.
    pub(in crate::store) fn cancel_original(&self) {
        ManagedChatScopeRevocation::revoke(&self.inner.revoker);
    }
}

impl ServerStore {
    /// The owning caller performs pair -> actual resync if needed -> drop temporary credits
    /// and drain retired metadata -> THIS final proof -> field moves only. Credit refund can
    /// wait, so a positive proof taken before that refund cannot be the publication edge.
    pub(in crate::store) fn validate_v2_genesis_current(
        &self,
        preparation: &V2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        if !matches!(&preparation.progress, Progress::CurrentSaved { .. }) {
            return Err(refused());
        }
        preparation
            .activation
            .retains_original(self, source, task)?;
        let runtime = self.chat_mount.locked()?;
        require_base(self, &runtime)?;
        if runtime.dirty {
            // Another group's failed write may dirty this SAME latest table. Preserve the
            // node/preparation and require an explicit newly funded real resync attempt.
            return Err(refused());
        }
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        validate_current_head(self, &runtime, source, task, preparation)?;
        require_owned_row(self, &runtime, preparation)?;
        let binding = preparation.current_binding()?;
        if binding.inner.key != preparation.inner.key
            || binding.inner.nonce != preparation.inner.record.nonce
            || binding.inner.revision != 2
            || binding.inner.domain != preparation.inner.domain
            || !Arc::ptr_eq(
                &binding.inner.owner_identity,
                &preparation.inner.owner_identity,
            )
            || !Arc::ptr_eq(&binding.inner.mount_identity, &self.chat_mount.identity)
            || binding.reservation_observer().is_cancelled()
        {
            return Err(refused());
        }
        // The complete Head/source proof performs any permanent artifact-fee validation
        // BEFORE its final named-object/negative checks. Only closed cached negatives and
        // metadata comparisons follow here; there is no temporary credit to refund.
        V2CurrentAdmission {
            store: self,
            runtime: &runtime,
            source,
            task,
            preparation,
            _not_send: PhantomData,
        }
        .validate_source_only()
    }
}
