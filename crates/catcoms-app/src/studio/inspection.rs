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
/// What a read may say about a retained draft: design section 11's `OverlayInspection`.
///
/// `eligibility` carries P2's `eligibility` and `manualReason` together, and `unconfirmed` design
/// 8.6's `unconfirmedState`, which is `None` exactly for a Closing branch or no branch. `archived`
/// is not here, because the inspection capture deliberately holds one record and the archive is a
/// different one; `studio_overlay_lifecycle` answers that question from the record that actually
/// knows.
///
/// A struct rather than a widening tuple because these are eight values of four types, and a
/// caller destructuring them positionally would be one reordering away from reporting a branch id
/// as a content digest.
#[derive(Debug)]
pub struct StudioOverlayInspected<'a> {
    pub target: StudioTarget,
    /// A transfer is staged. `prepared` does not mean the transfer happened.
    pub prepared: bool,
    /// `None` when nothing is retained, **and also** when the branch could not be reconstructed:
    /// `replayable` is what tells those apart.
    pub draft: Option<&'a types::StudioLocalDraft>,
    pub branch: Option<[u8; 32]>,
    pub content: Option<[u8; 32]>,
    /// The last generation this vault used, which outlives the branch that used it. Meaningful
    /// beside `branch`, not on its own.
    pub generation: u64,
    pub provenance: Option<types::StudioOverlayProvenance>,
    /// A retained terminal disposal. "This was disposed of" and "there is nothing here" are
    /// different answers and a read has to be able to give the first one.
    pub disposed: Option<&'a types::StudioOverlayDisposal>,
    /// Whether typed reconstruction succeeded. `false` with a branch present is the shape design
    /// finding 5 asks for: every structural field, and a null typed projection.
    pub replayable: bool,
    /// P2: whether the automatic handoff would take this branch now, and if not, why. `None` when
    /// no branch is live. A branch that did not reconstruct is `Manual(NotReplayable)` whatever
    /// else holds, because that is the one condition no amount of waiting changes.
    pub eligibility: Option<types::StudioOverlayEligibility>,
    /// Design 8.6: how an Unconfirmed branch's base relates to the installed source, derived on
    /// read. `None` for a Closing branch or no branch.
    pub unconfirmed: Option<types::StudioOverlayUnconfirmedState>,
}

#[derive(Debug)]
pub struct StudioOverlayInspection {
    read: Arc<Retained>,
    delivery: Option<StudioInspectionDelivery>,
    eligibility: Option<types::StudioOverlayEligibility>,
    unconfirmed: Option<types::StudioOverlayUnconfirmedState>,
}
#[derive(Clone, Debug)]
pub struct StudioInspectionDelivery(Arc<Delivery>);
struct Delivery {
    valid: Arc<AtomicBool>,
    clock: Arc<dyn catcoms_rt::Clock + Send>,
    expires: u64,
    /// Whatever owns the job's shared preparation slot: an inspection's `Retained`, or a copy's
    /// permit. Type-erased because the guarantee is the same for every result that carries one -
    /// the slot is not returned to the pool while native still holds the result.
    _retained: Arc<dyn std::any::Any + Send + Sync>,
    _ack: oneshot::Sender<()>,
}

/// Begin the actor's bounded handoff for one result, keeping `retained` alive through it.
///
/// One definition for every overlay result that needs it, so an inspection, an export, an archive
/// and a copy preview are all fenced the same way: the actor waits in `PreviewHandoff::finish`
/// (processing no membership, MLS or source change) until native drops the delivery, the request is
/// cancelled, or the fixed timeout fires, and a timeout revokes every copy of the delivery.
fn begin(
    retained: Arc<dyn std::any::Any + Send + Sync>,
    clock: Arc<dyn catcoms_rt::Clock + Send>,
) -> (StudioInspectionDelivery, super::preview::PreviewHandoff) {
    let (handoff, valid, ack) = super::preview::PreviewHandoff::new();
    let delivery = StudioInspectionDelivery(Arc::new(Delivery {
        valid,
        expires: clock.monotonic_ms().saturating_add(5_000),
        clock,
        _retained: retained,
        _ack: ack,
    }));
    (delivery, handoff)
}

/// A finished overlay result delivered the way an inspection is.
///
/// Export, archive and copy preview used to copy their payload out of the job and return it bare.
/// That released the shared preparation permit as soon as the finish visit returned, while a
/// multi-megabyte result was still waiting for native conversion, and it skipped the delivery
/// fence entirely: the actor went straight back to processing membership and MLS changes while
/// native was still converting a result whose final validation those changes could invalidate.
///
/// This keeps the job's ORIGINAL permit (no second acquisition, no second pool) until the last copy
/// of the delivery is dropped, and the value is reachable only through [`Self::inspect`], which
/// refuses once the delivery has expired or been revoked.
pub struct StudioDelivered<T> {
    value: T,
    retained: Arc<dyn std::any::Any + Send + Sync>,
    delivery: Option<StudioInspectionDelivery>,
}
impl<T> std::fmt::Debug for StudioDelivered<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioDelivered { .. }")
    }
}
impl<T> StudioDelivered<T> {
    pub(crate) fn new(value: T, retained: Arc<dyn std::any::Any + Send + Sync>) -> Self {
        Self {
            value,
            retained,
            delivery: None,
        }
    }
    /// `None` until the actor has begun the handoff. Native takes this before conversion and
    /// checks it again after.
    pub fn delivery(&self) -> Option<StudioInspectionDelivery> {
        self.delivery.clone()
    }
    /// The value, but only while its delivery is current. A result that never went through the
    /// actor's handoff has no delivery and is refused, rather than read unfenced.
    pub fn inspect<O>(&self, inspect: impl FnOnce(&T) -> O) -> Result<O, String> {
        if !self.delivery.as_ref().is_some_and(|d| d.is_current()) {
            return Err("overlay result delivery expired; refresh".into());
        }
        Ok(inspect(&self.value))
    }
    pub(crate) fn begin_delivery(
        &mut self,
        clock: Arc<dyn catcoms_rt::Clock + Send>,
    ) -> super::preview::PreviewHandoff {
        let (delivery, handoff) = begin(self.retained.clone(), clock);
        self.delivery = Some(delivery);
        handoff
    }
    /// For tests that drive the stages directly, below the actor.
    #[cfg(test)]
    pub(crate) fn value(&self) -> &T {
        &self.value
    }
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
    /// Rebuild for archiving instead. Same capture, same permit, same currency contract; the only
    /// difference is that typed reconstruction becomes an observation rather than a requirement,
    /// so a branch that cannot be replayed can still be preserved.
    pub async fn rebuild_for_archive(self) -> Result<StudioPreparedInspection, AppError> {
        self.rebuild_with(|capture| {
            capture.rebuild_for(crate::store::StudioInspectionPurpose::Archive)
        })
        .await
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
        inspect: impl FnOnce(StudioOverlayInspected<'_>) -> O,
    ) -> Result<O, String> {
        if !self.delivery().is_current() {
            return Err("overlay inspection delivery expired; refresh".into());
        }
        let value = &self.read.value;
        Ok(inspect(StudioOverlayInspected {
            target: value.target,
            prepared: value.prepared,
            draft: value.draft.as_ref(),
            branch: value.branch,
            content: value.content,
            generation: value.generation,
            provenance: value.provenance,
            disposed: value.disposed.as_ref(),
            replayable: value.replayable.is_ok(),
            eligibility: self.eligibility,
            unconfirmed: self.unconfirmed,
        }))
    }
    /// Attach design 8.6's reconciliation, computed under the same custody visit that finished this
    /// read, for the same reason as [`Self::with_eligibility`].
    pub(crate) fn with_unconfirmed(
        mut self,
        unconfirmed: Option<types::StudioOverlayUnconfirmedState>,
    ) -> Self {
        self.unconfirmed = unconfirmed;
        self
    }
    /// Attach P2's classification, computed under the same custody visit that finished this read.
    ///
    /// Separate from `finish_studio_inspection` because export and archive finish through it too
    /// and do not need it; the source load it costs is paid only by a read that will show it.
    pub(crate) fn with_eligibility(
        mut self,
        eligibility: Option<types::StudioOverlayEligibility>,
    ) -> Self {
        self.eligibility = match (eligibility, &self.read.value.replayable) {
            (Some(_), Err(_)) => Some(types::StudioOverlayEligibility::Manual(
                types::StudioOverlayManualReason::NotReplayable,
            )),
            (eligibility, _) => eligibility,
        };
        self
    }
    /// The archive this inspection built, for the durable write in the same custody visit.
    ///
    /// Not behind the delivery fence, unlike [`Self::inspect`]: that fence exists because a
    /// renderer's conversion of a projection can outlive the state it describes. This value never
    /// leaves the actor, and the check that matters for it - that the record has not changed under
    /// the rebuild - is `finish_studio_inspection`'s, which has already run.
    pub(crate) fn archive(&self) -> Result<&types::StudioDraftArchive, AppError> {
        let value = &self.read.value;
        value.archive.as_ref().ok_or_else(|| {
            if value.purpose != crate::store::StudioInspectionPurpose::Archive {
                invalid("this inspection was not prepared for archiving")
            } else {
                invalid("no local draft to archive")
            }
        })
    }
    /// What typed reconstruction found. An `Err` does not stop an archive being written; it is
    /// recorded in the archive so a later reader knows the branch was already unreplayable when it
    /// was preserved, rather than suspecting the archive of having broken it.
    pub(crate) fn replayable(&self) -> Result<(), String> {
        self.read.value.replayable.clone()
    }
    pub(crate) fn begin_delivery(
        &mut self,
        clock: Arc<dyn catcoms_rt::Clock + Send>,
    ) -> super::preview::PreviewHandoff {
        let (delivery, handoff) = begin(self.read.clone(), clock);
        self.delivery = Some(delivery);
        handoff
    }
    /// The delivery if the actor has begun one. Unlike [`Self::delivery`] this does not panic, so
    /// a caller handling every response variant can ask uniformly.
    pub(crate) fn delivery_if_begun(&self) -> Option<StudioInspectionDelivery> {
        self.delivery.clone()
    }
    /// What owns this job's preparation slot, for a result derived from this inspection that must
    /// keep the same slot through its own delivery.
    pub(crate) fn retained(&self) -> Arc<dyn std::any::Any + Send + Sync> {
        self.read.clone()
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
    /// Inspection against a given pool; see `begin_copy_with_pool` for why the seam exists.
    pub(in crate::studio) fn begin_inspection_with_pool(
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
            eligibility: None,
            unconfirmed: None,
        })
    }
}

#[cfg(test)]
mod tests;
