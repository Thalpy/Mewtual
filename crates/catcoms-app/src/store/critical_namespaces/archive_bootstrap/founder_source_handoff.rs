//! Closed SourceSaved-to-original-runtime handoff. No Server/SDK escapes Root.
use super::*;
use crate::store::chat_admission::v2_current::V2CurrentPreparation;
use crate::store::persistence::{GenesisSourceDataRef, OriginalBootstrapReadObservation};
use crate::store::RetainedBootstrapSourceData;
use catcoms_rt::{RequestAuthority, RequestCancellation};
use catcoms_sync::local_chat_scope::{ManagedChatScopeReservation, ManagedChatScopeRevocation};
use catcoms_sync::owner_v2::V2GenesisRuntimeParts;

/// Issued only below actual SavedTerminal/read checks. This inert bridge keeps
/// the old reservation identity and COMPLETE negative; it grants no Current.
pub(in crate::store) struct CompletedWarmFounderOrigin {
    identity: Arc<()>,
    negative: RequestCancellation,
    revoker: Arc<dyn ManagedChatScopeRevocation>,
    context: BootstrapSourceContextData,
    source: BootstrapSourceId,
    read: OriginalBootstrapReadObservation,
}

/// A reservation-only alias for receiver attachment. Real Current proofs keep
/// using their distinct AdmissionOwner identity. Sync sees the original owner
/// pointer plus BOTH original/current negatives, so its pointer checks stay intact.
pub(in crate::store) struct WarmFounderCurrentReservation {
    identity: Arc<()>,
    current_identity: Arc<()>,
    observer: RequestCancellation,
    revoker: Arc<dyn ManagedChatScopeRevocation>,
    context: BootstrapSourceContextData,
    signer: [u8; 32],
}

pub(in crate::store) fn bridge_layout_bytes() -> usize {
    std::mem::size_of::<CompletedWarmFounderOrigin>()
        + std::mem::size_of::<WarmFounderCurrentReservation>()
        + std::mem::size_of::<BothReservations>()
        + std::mem::size_of::<BothRevokers>()
        + 2 * 256
        + 4 * 64 // bounded group copies and Arc/weak controls
}

#[derive(Debug)]
struct BothReservations {
    old: RequestCancellation,
    current: RequestCancellation,
}
#[async_trait::async_trait]
impl RequestAuthority for BothReservations {
    fn authorized(&self) -> bool {
        !self.old.is_cancelled() && !self.current.is_cancelled()
    }
    async fn revoked(&self) {
        let mut old = self.old.clone();
        let mut current = self.current.clone();
        if self.authorized() {
            tokio::select! { _ = old.cancelled() => {}, _ = current.cancelled() => {} }
        }
    }
}
struct BothRevokers {
    old: Arc<dyn ManagedChatScopeRevocation>,
    current: Arc<dyn ManagedChatScopeRevocation>,
}
impl ManagedChatScopeRevocation for BothRevokers {
    fn revoke(&self) {
        self.old.revoke();
        self.current.revoke();
    }
}

impl CompletedWarmFounderOrigin {
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn negative_for_test(&self) -> RequestCancellation {
        self.negative.clone()
    }
    fn validate_source(&self, source: &GenesisSourceDataRef<'_>) -> Result<(), AppError> {
        self.read.require_same_original(source.source())?;
        if self.negative.is_cancelled()
            || source.data().source != self.source
            || source.data().context != &self.context
        {
            return Err(refused());
        }
        Ok(())
    }

    /// Only the concrete already-saved Current preparation may become this
    /// successor. Caller proves its actual source/head/task/writer both before
    /// and after these allocations, outside all writers.
    pub(in crate::store) fn prepare_current(
        &self,
        current: &V2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<WarmFounderCurrentReservation, AppError> {
        self.validate_source(source)?;
        let spec = self.context.spec_data();
        if current.group() != spec.group
            || current.signer() != &spec.signer
            || current.domain() != spec.scope_domain
            || current.nonce() != spec.scope_nonce
            || current.planned_revision_data() != Some(spec.planned_revision)
        {
            return Err(refused());
        }
        let current_identity = current.owner_identity();
        let complete_current = current.reservation_observer();
        // with_authority REPLACES an authority, so retain the FULL previous
        // Current observer inside the composite; its mount/owner checks survive.
        let observer = complete_current
            .clone()
            .with_authority(Arc::new(BothReservations {
                old: self.negative.clone(),
                current: complete_current,
            }));
        let revoker = Arc::new(BothRevokers {
            old: self.revoker.clone(),
            current: current.revocation(),
        });
        let adapter = WarmFounderCurrentReservation {
            identity: self.identity.clone(),
            current_identity,
            observer,
            revoker,
            context: self.context.clone(),
            signer: spec.signer,
        };
        adapter.validate_current(self, current, source)?;
        Ok(adapter)
    }
}
impl WarmFounderCurrentReservation {
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn corrupt_current_identity_for_test(&mut self) {
        self.current_identity = Arc::new(());
    }
    pub(in crate::store) fn validate_current(
        &self,
        origin: &CompletedWarmFounderOrigin,
        current: &V2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
    ) -> Result<(), AppError> {
        origin.validate_source(source)?;
        if !Arc::ptr_eq(&self.identity, &origin.identity)
            || !Arc::ptr_eq(&self.current_identity, &current.owner_identity())
            || self.context != origin.context
            || self.observer.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
}
impl ManagedChatScopeReservation for WarmFounderCurrentReservation {
    fn group(&self) -> &[u8] {
        self.context.spec_data().group
    }
    fn signer(&self) -> &[u8; 32] {
        &self.signer
    }
    fn domain(&self) -> [u8; 32] {
        self.context.spec_data().scope_domain
    }
    fn nonce(&self) -> [u8; 32] {
        self.context.spec_data().scope_nonce
    }
    fn planned_revision_data(&self) -> Option<u64> {
        Some(self.context.spec_data().planned_revision)
    }
    fn owner_identity(&self) -> Arc<()> {
        self.identity.clone()
    }
    fn reservation_observer(&self) -> RequestCancellation {
        self.observer.clone()
    }
    fn revocation(&self) -> Arc<dyn ManagedChatScopeRevocation> {
        self.revoker.clone()
    }
}

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderSourcePreparation<T, R> {
    /// The caller retains this SAME creator through the resulting whole Host.
    /// Its separate invoice precedes both possible runtime/refusal allocations.
    pub(in crate::store) fn take_completed_runtime(
        &mut self,
        store: &ServerStore,
        read: &RetainedBootstrapSourceData,
        fee: &CatalogReadReservation,
    ) -> Result<(V2GenesisRuntimeParts<T, R>, CompletedWarmFounderOrigin), AppError> {
        let bytes = V2GenesisRuntimeParts::<T, R>::allocation_bytes()
            .checked_add(V2GenesisRuntimeParts::<T, R>::warm_refusal_allocation_bytes())
            .and_then(|n| n.checked_add(bridge_layout_bytes()))
            .ok_or_else(refused)?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        fee.validate(pool, bytes)?;
        read.validate_original(store)?;
        {
            let key = Zeroizing::new(store.keys.db_key()?);
            let SourcePhase::Transferred {
                source,
                context,
                id,
            } = &self.phase
            else {
                return Err(refused());
            };
            if read.data().source_id_data() != *id
                || read.data().view_data().context != context.as_ref()
            {
                return Err(refused());
            }
            let source = ActualFounderSource {
                creator: &self.original.creator,
                phase: ActualSourcePhase::Transferred {
                    source,
                    key: &key,
                    context,
                    id: *id,
                },
            };
            store.with_original_archive_founder(
                &source,
                self.original.plan.as_ref().ok_or_else(refused)?,
                |admission| {
                    let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                    self.saved_terminal
                        .as_ref()
                        .ok_or_else(refused)?
                        .validate_original(
                            store,
                            &self.original.creator.original,
                            &vacancy,
                            self.registered.as_ref().ok_or_else(refused)?,
                        )
                },
            )?;
        }
        // No writer/Root/key/pool guard survives into SDK destruction. A failed
        // checked transfer returns BOTH original objects, not recreated state.
        self.original.creator.original.validate_original(store)?;
        let reservation = self
            .original
            .plan
            .as_ref()
            .ok_or_else(refused)?
            .reservation_for_saved_source(
                &self.original.creator.original,
                self.original.saved_founding.as_ref().ok_or_else(refused)?,
            )?;
        let SourcePhase::Transferred { context, id, .. } = &self.phase else {
            return Err(refused());
        };
        let origin = CompletedWarmFounderOrigin {
            identity: reservation.owner_identity(),
            negative: reservation.reservation_observer(),
            revoker: reservation.revocation(),
            context: (**context).clone(),
            source: *id,
            read: read.observe_original(),
        };
        let server = self.original.creator.server.take().ok_or_else(refused)?;
        let SourcePhase::Transferred {
            source,
            context,
            id,
        } = std::mem::replace(&mut self.phase, SourcePhase::HeldConsumingRefusal)
        else {
            self.original.creator.server = Some(server);
            return Err(refused());
        };
        let Server {
            sync,
            display_name,
            device_id,
            own_message_changes,
            messages_cache,
            admin_chat_ledger,
            delivery_snapshot_revision,
            devices_sig,
            cache,
        } = server;
        match source.into_genesis_runtime_parts(sync, id) {
            Ok(parts) => {
                // No fallible observation follows this successful move. The
                // driver caches parts before any late original-negative check.
                self.phase = SourcePhase::RuntimeTransferred;
                Ok((parts, origin))
            }
            Err(returned) => {
                let (source, sync) = returned.into_originals();
                self.phase = SourcePhase::Transferred {
                    source: Box::new(source),
                    context,
                    id,
                };
                self.original.creator.server = Some(Server {
                    sync,
                    display_name,
                    device_id,
                    own_message_changes,
                    messages_cache,
                    admin_chat_ledger,
                    delivery_snapshot_revision,
                    devices_sig,
                    cache,
                });
                Err(refused())
            }
        }
    }
}
