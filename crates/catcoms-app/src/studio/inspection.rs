//! One read job owns a shared preparation slot from capture through native conversion.
use super::*;
use crate::store::{StudioInspectedDraft, StudioInspectionCapture, StudioInspectionStamp};
use catcoms_crypto::DeviceId;
use catcoms_sync::RegistrySyncInstance;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::OwnedSemaphorePermit;

#[derive(PartialEq, Eq)]
struct Context {
    group: Vec<u8>,
    device: DeviceId,
    owner: Option<DeviceId>,
    mls: u64,
}

pub struct StudioInspectionPreparation {
    capture: StudioInspectionCapture,
    instance: RegistrySyncInstance,
    context: Context,
    permit: OwnedSemaphorePermit,
}
pub struct StudioPreparedInspection {
    stamp: StudioInspectionStamp,
    instance: RegistrySyncInstance,
    context: Context,
    read: Arc<Retained>,
}
#[derive(Debug)]
struct Retained {
    value: StudioInspectedDraft,
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug)]
pub struct StudioOverlayInspection {
    read: Arc<Retained>,
    delivery: Option<StudioInspectionDelivery>,
}
#[derive(Clone, Debug)]
pub struct StudioInspectionDelivery(Arc<Delivery>);
struct Delivery {
    valid: Arc<AtomicBool>,
    clock: Arc<dyn catcoms_rt::Clock + Send>,
    expires: u64,
    _read: Arc<Retained>,
    _ack: oneshot::Sender<()>,
}
impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Delivery { .. }")
    }
}
impl std::fmt::Debug for StudioInspectionPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioInspectionPreparation { .. }")
    }
}
impl std::fmt::Debug for StudioPreparedInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioPreparedInspection { .. }")
    }
}
impl StudioInspectionPreparation {
    /// No actor or vault lease may be held by a caller awaiting this job. Aborting the waiter
    /// leaves the original permit owned by the actual blocking worker until destruction.
    pub async fn rebuild(self) -> Result<StudioPreparedInspection, AppError> {
        self.rebuild_with(StudioInspectionCapture::rebuild).await
    }
    async fn rebuild_with(
        self,
        rebuild: impl FnOnce(
                StudioInspectionCapture,
            ) -> Result<(StudioInspectionStamp, StudioInspectedDraft), AppError>
            + Send
            + 'static,
    ) -> Result<StudioPreparedInspection, AppError> {
        tokio::task::spawn_blocking(move || {
            let (stamp, value) = rebuild(self.capture)?;
            Ok(StudioPreparedInspection {
                stamp,
                instance: self.instance,
                context: self.context,
                read: Arc::new(Retained {
                    value,
                    _permit: self.permit,
                }),
            })
        })
        .await
        .map_err(|_| invalid("overlay inspection worker failed"))?
    }
}
impl StudioInspectionDelivery {
    pub fn is_current(&self) -> bool {
        self.0.valid.load(Ordering::Acquire) && self.0.clock.monotonic_ms() < self.0.expires
    }
}
impl StudioOverlayInspection {
    pub fn delivery(&self) -> StudioInspectionDelivery {
        self.delivery
            .as_ref()
            .expect("actor inspection delivery")
            .clone()
    }
    /// A local projection only. No append basis, installed epoch or signed authority escapes.
    pub fn inspect<O>(
        &self,
        inspect: impl FnOnce(StudioTarget, bool, Option<&types::StudioLocalDraft>) -> O,
    ) -> Result<O, String> {
        if !self.delivery().is_current() {
            return Err("overlay inspection delivery expired; refresh".into());
        }
        let value = &self.read.value;
        Ok(inspect(value.target, value.prepared, value.draft.as_ref()))
    }
    pub(crate) fn begin_delivery(
        &mut self,
        clock: Arc<dyn catcoms_rt::Clock + Send>,
    ) -> super::preview::PreviewHandoff {
        let (handoff, valid, ack) = super::preview::PreviewHandoff::new();
        self.delivery = Some(StudioInspectionDelivery(Arc::new(Delivery {
            valid,
            expires: clock.monotonic_ms().saturating_add(5_000),
            clock,
            _read: self.read.clone(),
            _ack: ack,
        })));
        handoff
    }
}
impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn inspection_context(&mut self, target: StudioTarget) -> Result<Context, AppError> {
        if !self
            .channels()
            .iter()
            .any(|c| c.id == u128::from_be_bytes(target.channel()))
        {
            return Err(invalid("unknown Studio channel"));
        }
        self.sync.with_registry_context(|group, device, _, _| {
            if group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
            {
                return Err(invalid("Studio requires current membership"));
            }
            Ok(Context {
                group: group.group_id(),
                device: device.device_id(),
                owner: group.designated_committer(),
                mls: group.epoch(),
            })
        })
    }
    pub(crate) fn begin_studio_inspection(
        &mut self,
        store: &ServerStore,
        server: u64,
        target: StudioTarget,
    ) -> Result<StudioInspectionPreparation, AppError> {
        self.begin_inspection_with_pool(
            store,
            server,
            target,
            crate::registry_catchup::preparation_pool(),
        )
    }
    fn begin_inspection_with_pool(
        &mut self,
        store: &ServerStore,
        server: u64,
        target: StudioTarget,
        pool: &Arc<tokio::sync::Semaphore>,
    ) -> Result<StudioInspectionPreparation, AppError> {
        let context = self.inspection_context(target)?;
        let permit = pool
            .clone()
            .try_acquire_owned()
            .map_err(|_| invalid("overlay inspection capacity exhausted; retry"))?;
        let capture =
            store.capture_studio_inspection(server, &context.group, target, context.device)?;
        Ok(StudioInspectionPreparation {
            capture,
            instance: self.sync.registry_instance(),
            context,
            permit,
        })
    }
    pub(crate) fn finish_studio_inspection(
        &mut self,
        store: &ServerStore,
        server: u64,
        target: StudioTarget,
        prepared: StudioPreparedInspection,
    ) -> Result<StudioOverlayInspection, AppError> {
        let context = self.inspection_context(target)?;
        if !self.sync.matches_registry_instance(&prepared.instance)
            || context != prepared.context
            || !store.studio_inspection_is_current(
                server,
                &context.group,
                target,
                context.device,
                &prepared.stamp,
            )?
        {
            return Err(invalid("overlay inspection changed; refresh"));
        }
        Ok(StudioOverlayInspection {
            read: prepared.read,
            delivery: None,
        })
    }
}

#[cfg(test)]
mod tests;
