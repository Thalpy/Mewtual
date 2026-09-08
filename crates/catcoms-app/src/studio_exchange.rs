//! Explicit two-member Studio operation exchange using the existing durable Index/art source.
//! These cooperative adapters do not schedule actor work, discover epochs, fetch pixels or emit
//! UI events. Native integration must supply the same lifecycle custody as local Studio Save.

use crate::store::{EpochStudioBudget, EpochStudioState};
use crate::{AppError, Server, ServerStore};
use catcoms_replication::studio::StudioTarget;
use catcoms_replication::{epoch_zero_id, Admission, DomainOp};
use catcoms_rt::{CryptoRngCore, MeshTransport, PublishSubmission};
use catcoms_sync::{StudioWatch, SyncError};
use std::sync::Arc;

/// Scope is captured, not supplied again at drain. Dropping this handle alone does not revoke
/// its desired transport subscription; explicitly unwatch before replacing lifecycle custody.
pub struct ServerStudioWatch {
    inner: StudioWatch,
    mount: Arc<()>,
    server: u64,
    target: StudioTarget,
}
impl std::fmt::Debug for ServerStudioWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerStudioWatch { .. }")
    }
}
/// Only returned after the existing source persistence barrier. Quarantine/duplicate are not new
/// accepted edits. Missing referenced pixels remain missing; no implicit network fetch occurs.
#[derive(Debug)]
pub struct StudioReceived {
    pub admission: Admission,
    pub state: EpochStudioState,
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn check_studio_channel(&self, target: StudioTarget) -> Result<(), AppError> {
        if !self
            .channels()
            .iter()
            .any(|channel| channel.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown channel"));
        }
        Ok(())
    }
    /// Explicit subscription to an existing checked epoch, or actual absent epoch zero. This
    /// neither creates a source nor trusts an incoming peer's claimed physical epoch/channel.
    /// Dense-source restore is synchronous: run under an off-executor exclusive coordinator,
    /// not directly inside the network event loop. Automatic scheduling remains separate.
    pub fn watch_studio_epoch(
        &mut self,
        store: &ServerStore,
        server: u64,
        target: StudioTarget,
    ) -> Result<ServerStudioWatch, AppError> {
        self.check_studio_channel(target)?;
        let id = self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("receiver is not a current member"));
            }
            let logical = target.document(&group.group_id()).map_err(invalid)?;
            Ok(store
                .load_studio_epoch(server, group, target, device)?
                .map(|state| state.doc_id())
                .unwrap_or_else(|| epoch_zero_id(logical.doc_type, &logical.logical_key)))
        })?;
        Ok(ServerStudioWatch {
            inner: self.sync.watch_studio(target, id)?,
            mount: store.registry_mount(),
            server,
            target,
        })
    }
    /// Uses the existing cancel-safe topic reconciliation, shared with other document families.
    pub async fn flush_studio_subscriptions(&mut self) -> Result<(), AppError> {
        Ok(self.sync.flush_studio_subscriptions().await?)
    }
    /// Revocation needs no live vault; a stale handle can remove only its own exact generation.
    pub fn unwatch_studio_epoch(&mut self, watch: &ServerStudioWatch) -> Result<(), AppError> {
        Ok(self.sync.unwatch_studio(&watch.inner)?)
    }
    /// Drain one authenticated packet through the existing accounted Studio gate. No raw packet
    /// or alternate server/channel can be supplied here. Queue removal is not acceptance: failures
    /// send no ack, and durable own-intent retry/catch-up must recover dropped volatile traffic.
    pub fn receive_studio_step(
        &mut self,
        store: &mut ServerStore,
        watch: &ServerStudioWatch,
        budget: &mut EpochStudioBudget,
    ) -> Result<Option<StudioReceived>, AppError> {
        if !Arc::ptr_eq(&watch.mount, &store.registry_mount())
            || !self.sync.studio_watch_is_current(&watch.inner)
        {
            return Err(invalid(
                "watch belongs to a replaced mount, server or epoch",
            ));
        }
        self.check_studio_channel(watch.target)?;
        self.sync
            .drain_studio_inbound(&watch.inner, |group, device, rng, sealed| {
                store
                    .ingest_studio_epoch(
                        watch.server,
                        group,
                        watch.target,
                        device,
                        sealed,
                        rng,
                        budget,
                    )
                    .map(|(admission, state)| StudioReceived { admission, state })
            })?
            .transpose()
    }
    /// Attempt to send one EXACT OWN operation already present in this saved Open source. This
    /// cannot author a new edit or bypass local Save's current-server/PIX publication ordering.
    /// The original DomainOp, not a projection or marker, is the retry identity. Duplicate,
    /// refusal, cancellation and even Submitted keep the intent; no ciphertext is queued here.
    ///
    /// Hold exclusive Server/store AND native numeric-server/UI/incarnation custody through the
    /// await. This low-level adapter cannot establish those frontend-owned fences. Restoration
    /// and disk work are synchronous and need off-executor preparation before automatic service.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_saved_studio_once(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        expected_doc_id: u128,
        operation: DomainOp,
        budget: &mut EpochStudioBudget,
    ) -> Result<PublishSubmission, AppError> {
        operation.encode().map_err(invalid)?;
        self.check_studio_channel(target)?;
        let sealed = self
            .sync
            .with_registry_context(|group, device, clock, rng| {
                if group.member_signature_key(&device.device_id()).as_deref()
                    != Some(device.public_key_bytes().as_slice())
                {
                    return Err(invalid("sender is not a current member"));
                }
                let held = store
                    .load_studio_epoch(server, group, target, device)?
                    .ok_or_else(|| invalid("operation is not saved"))?;
                if held.doc_id() != expected_doc_id
                    || !held.contains_exact_operation(device.device_id(), &operation)?
                {
                    return Err(invalid("operation is not saved in the requested epoch"));
                }
                // Exact retry rechecks Open/current membership, accounts and flushes BOTH barriers,
                // then freshly seals the original signed change. It cannot create different content.
                let (sealed, _) = store.edit_studio_epoch(
                    server,
                    group,
                    target,
                    expected_doc_id,
                    device,
                    operation,
                    clock.now_ms(),
                    rng,
                    budget,
                )?;
                Ok::<_, AppError>(sealed)
            })?;
        self.sync
            .publish_local_studio_once(target, expected_doc_id, sealed)
            .await
            .map_err(|error: SyncError| error.into())
    }
}
fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("Studio exchange: {error}"))
}

#[cfg(test)]
mod tests;
