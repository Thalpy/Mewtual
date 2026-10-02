//! Copy into current, stages C2 to C4.
//!
//! Two phases, mirroring the accepted recovery `Preview` -> `Apply` shape, because copy is the same
//! kind of act: a proposal built off custody, shown to a user, then re-derived under custody before
//! anything is written. Nothing here writes to the branch's own record. Copy is never a precondition
//! for destroying anything, and no count of copied items ever establishes that a branch was
//! preserved; only an archive does that.
//!
//! **Copy is projection-level and lossy on purpose (C-P).** It recovers the selected value of an
//! element as a new operation authored by the copier. A branch entry superseded within the branch, a
//! conflict alternative, the original authorship of an accepted envelope and the accepted ordering
//! all have no representation in what this produces. `source_ops` reports exactly which source
//! operations the proposal resolved and nothing more.
use super::*;
use crate::store::{StudioOverlayCopyCapture, StudioOverlayCopyChoice, StudioOverlayCopyPlan};
use crate::studio::restore::PlanScope;
use catcoms_crypto::DeviceId;
use catcoms_sync::RegistrySyncInstance;
use tokio::sync::OwnedSemaphorePermit;

/// The live context a copy is admitted under. Compared again at C3 and C4: a plan minted while this
/// device was a member is not a plan a removed device may finish.
#[derive(PartialEq, Eq)]
struct Context {
    group: Vec<u8>,
    device: DeviceId,
}

pub struct StudioCopyPreparation {
    capture: StudioOverlayCopyCapture,
    choice: StudioOverlayCopyChoice,
    scope: PlanScope,
    instance: RegistrySyncInstance,
    context: Context,
    permit: OwnedSemaphorePermit,
}
pub struct StudioPreparedCopy {
    plan: StudioOverlayCopyPlan,
    instance: RegistrySyncInstance,
    context: Context,
    _permit: OwnedSemaphorePermit,
}
impl std::fmt::Debug for StudioCopyPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioCopyPreparation { .. }")
    }
}
impl std::fmt::Debug for StudioPreparedCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioPreparedCopy { .. }")
    }
}

impl StudioCopyPreparation {
    /// C2, on a blocking worker. No actor or vault lease may be held by a caller awaiting this.
    pub async fn plan(self) -> Result<StudioPreparedCopy, AppError> {
        let Self {
            capture,
            choice,
            scope,
            instance,
            context,
            permit,
        } = self;
        tokio::task::spawn_blocking(move || {
            Ok(StudioPreparedCopy {
                plan: capture.plan(choice, scope)?,
                instance,
                context,
                _permit: permit,
            })
        })
        .await
        .map_err(|_| invalid("overlay copy worker failed"))?
    }
}

/// The renderer's apply echo. A new preview is a NEW decision and needs a new nonce; never rewrite a
/// predecessor's body under an earlier nonce.
pub struct StudioOverlayCopyApply {
    pub destination: StudioTarget,
    pub item: StudioRecoveryItem,
    pub mode: StudioRecoveryMode,
    pub epoch_id: u128,
    pub expected_projection: [u8; 32],
    pub nonce: [u8; 16],
    pub body: Vec<u8>,
}
impl std::fmt::Debug for StudioOverlayCopyApply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlayCopyApply { .. }")
    }
}

/// What C3 hands back: a bounded proposed body, or an explicit hold. Saves nothing.
#[derive(Debug)]
pub struct StudioOverlayCopyPreview {
    pub source: StudioTarget,
    pub destination: StudioTarget,
    pub epoch_id: u128,
    pub expected_projection: [u8; 32],
    pub disposition: StudioRecoveryDisposition,
    pub body: Option<Vec<u8>>,
    pub original_author: Option<DeviceId>,
    /// Exactly which source operations the proposal resolved. Not a preservation claim (C-P).
    pub source_ops: Vec<[u8; 32]>,
}

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    fn copy_context(&mut self, target: StudioTarget) -> Result<Context, AppError> {
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
            })
        })
    }

    /// An `Object` put must name a Flipnote that actually exists, and only custody can tell.
    ///
    /// Recovery already refuses exactly this: an index entry proves nothing about its object, so a
    /// dangling or differently scoped historical object must not be republished however well formed
    /// the `PutObject` is. Copy reached `restore::plan` from the detached worker, which has no
    /// store and therefore cannot probe, and neither C3 nor C4 put the probe back. The result was
    /// an Index branch retained across Closing whose object had since been cleaned up: `Ready`,
    /// applied, and a durable Index entry naming a source that does not exist.
    ///
    /// Run at C3 **and** C4 rather than once. At C3 it downgrades, so the user is told the target
    /// is missing instead of being offered a copy that will fail; at C4 it refuses, because the
    /// object can disappear between the two.
    fn probe_copy_object(
        store: &mut ServerStore,
        server: u64,
        group: &catcoms_mls::ServerGroup,
        device: &catcoms_mls::MlsDevice,
        plan: &StudioOverlayCopyPlan,
    ) -> Result<bool, AppError> {
        let StudioRecoveryItem::Object { id } = plan.choice().item else {
            return Ok(true);
        };
        if plan.disposition() != StudioRecoveryDisposition::Ready {
            return Ok(true);
        }
        let object = StudioTarget::Flipnote {
            channel: plan.destination_target().channel(),
            object: id,
        };
        Ok(store
            .with_studio_source(server, group, object, device, |s| {
                Ok(s.op_count() > 0 || s.epoch() > 0)
            })?
            .unwrap_or(false))
    }

    /// The destination's scope, decided here rather than accepted from a caller.
    ///
    /// A renderer that could choose `CrossDocument` for a same-document copy would be choosing which
    /// checks apply to its own request. Both targets are already known, so the answer is derivable
    /// and is derived.
    fn copy_scope(source: StudioTarget, destination: StudioTarget) -> Result<PlanScope, AppError> {
        if source.channel() != destination.channel() {
            return Err(invalid("a copy destination is in another channel"));
        }
        Ok(match (source, destination) {
            _ if source == destination => PlanScope::SameDocument,
            (StudioTarget::Flipnote { .. }, StudioTarget::Flipnote { .. }) => {
                PlanScope::CrossDocument
            }
            // The Index is one document per channel, so "another Index" does not exist, and an
            // Index/Flipnote pair is a document-type change that the planner refuses anyway. Saying
            // so here gives the caller the real reason.
            _ => return Err(invalid("Index copies are same-document only")),
        })
    }

    /// C1: both captures, one permit, one custody visit.
    pub(crate) fn begin_studio_copy(
        &mut self,
        store: &ServerStore,
        server: u64,
        source: StudioTarget,
        choice: StudioOverlayCopyChoice,
    ) -> Result<StudioCopyPreparation, AppError> {
        self.begin_copy_with_pool(
            store,
            server,
            source,
            choice,
            crate::registry_catchup::preparation_pool(),
        )
    }
    /// C1 against a given pool. Production always passes the shared preparation pool; the seam
    /// exists so a capacity test can count one job's slot without racing every other test in the
    /// process for the global one, as inspection's capacity test already does.
    fn begin_copy_with_pool(
        &mut self,
        store: &ServerStore,
        server: u64,
        source: StudioTarget,
        choice: StudioOverlayCopyChoice,
        pool: &std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<StudioCopyPreparation, AppError> {
        let scope = Self::copy_scope(source, choice.destination)?;
        let context = self.copy_context(source)?;
        self.copy_context(choice.destination)?;
        let permit = pool
            .clone()
            .try_acquire_owned()
            .map_err(|_| invalid("overlay copy capacity exhausted; retry"))?;
        let capture = self.sync.with_registry_context(|group, device, _, _| {
            store.capture_studio_overlay_copy(server, group, source, choice.destination, device)
        })?;
        Ok(StudioCopyPreparation {
            capture,
            choice,
            scope,
            instance: self.sync.registry_instance(),
            context,
            permit,
        })
    }

    /// C3: revalidate all three records and the live context, then hand back one bounded proposal.
    ///
    /// Three records, not one. The branch the work came from, and the destination's Studio and
    /// recovery records. A proposal built from any of them after it moved is stale however well
    /// formed it is.
    pub(crate) fn finish_studio_copy_preview(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        source: StudioTarget,
        prepared: StudioPreparedCopy,
    ) -> Result<super::StudioDelivered<StudioOverlayCopyPreview>, AppError> {
        let StudioPreparedCopy {
            plan,
            instance,
            context: prepared_context,
            _permit: permit,
        } = prepared;
        let context = self.copy_context(source)?;
        // The destination's channel is rechecked here too, not only at C1 and C4. A channel this
        // device has left between the capture and the preview is not a channel it may still be
        // offered a copy into.
        self.copy_context(plan.destination_target())?;
        if !self.sync.matches_registry_instance(&instance) || context != prepared_context {
            return Err(invalid("overlay copy changed; preview again"));
        }
        let mut plan = plan;
        let current = self.sync.with_registry_context(|group, device, _, _| {
            store.studio_copy_is_current(server, group, device, &plan)
        })?;
        if !current {
            return Err(invalid("overlay copy changed; preview again"));
        }
        // The destination must still be the epoch the proposal was built for, and still Open. An
        // Open check alone would let a proposal for one epoch land in its successor.
        if plan.phase() != EpochPhase::Open {
            return Err(invalid("Studio copy requires an Open destination epoch"));
        }
        if !self.sync.with_registry_context(|group, device, _, _| {
            Self::probe_copy_object(store, server, group, device, &plan)
        })? {
            plan.hold(StudioRecoveryDisposition::MissingTarget);
        }
        // The preview keeps the job's ORIGINAL permit through native conversion and delivery,
        // rather than returning it to the shared pool the moment this visit returns.
        Ok(super::StudioDelivered::new(
            StudioOverlayCopyPreview {
                source,
                destination: plan.destination_target(),
                epoch_id: plan.epoch_id(),
                expected_projection: plan.fingerprint(),
                disposition: plan.disposition(),
                body: plan.body().cloned(),
                original_author: plan.original_author(),
                source_ops: plan.source_ops().to_vec(),
            },
            std::sync::Arc::new(permit),
        ))
    }

    /// C4. Exact-retry shortcut first, then re-plan from durable state and demand the echo match.
    ///
    /// The echo is revalidated rather than trusted: the renderer sends back an epoch, a projection
    /// fingerprint and a body, and all three must agree with a plan derived **here** from records
    /// read **now**. A preview is a proposal, and the gap between a proposal and a write is exactly
    /// where the destination can change.
    ///
    /// The exact-retry check comes first for the same reason recovery's does: a caller whose apply
    /// returned uncertainly must be able to resend the identical request and be told it already
    /// landed, rather than being refused because the operation it is retrying is now in the way.
    pub(crate) fn prepare_studio_copy_apply(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        source: StudioTarget,
        apply: StudioOverlayCopyApply,
    ) -> Result<(StudioRequest, bool), AppError> {
        if apply.body.len() > catcoms_replication::epoch::MAX_DOMAIN_OP_BYTES {
            return Err(invalid("copy body exceeds the operation bound"));
        }
        let choice = StudioOverlayCopyChoice {
            destination: apply.destination,
            item: apply.item,
            mode: apply.mode,
        };
        let scope = Self::copy_scope(source, apply.destination)?;
        // The publication targets the DESTINATION. The source is where the value came from and has
        // nothing written to it: copy writes nothing to the branch's record.
        let request = StudioRequest::Apply {
            target: apply.destination,
            epoch_id: apply.epoch_id,
            nonce: apply.nonce,
            body: apply.body.clone(),
        };
        request.validate()?;
        self.copy_context(source)?;
        self.copy_context(apply.destination)?;
        let capture = self.sync.with_registry_context(|group, device, _, _| {
            store.capture_studio_overlay_copy(server, group, source, apply.destination, device)
        })?;
        let already_saved = self.sync.with_registry_context(|group, device, _, _| {
            let op = super::domain(apply.destination, apply.nonce, apply.body.clone());
            let exact = store
                .with_studio_source(server, group, apply.destination, device, |state| {
                    if state.doc_id() != apply.epoch_id || state.phase() != EpochPhase::Open {
                        return Err(invalid("copy preview epoch is no longer Open/current"));
                    }
                    state.contains_exact_operation(device.device_id(), &op)
                })?
                .unwrap_or(false);
            if exact {
                return Ok(true);
            }
            let plan = capture.plan(choice, scope)?;
            if plan.epoch_id() != apply.epoch_id || plan.fingerprint() != apply.expected_projection
            {
                return Err(invalid("copy preview is stale; preview again"));
            }
            if plan.disposition() != StudioRecoveryDisposition::Ready
                || plan.body() != Some(&apply.body)
            {
                return Err(invalid("copy choice is no longer Ready or body differs"));
            }
            // Refused rather than downgraded here: at C4 there is nothing left to offer the user,
            // and an object that vanished between the preview and the apply is exactly the race
            // this probe exists for.
            if !Self::probe_copy_object(store, server, group, device, &plan)? {
                return Err(invalid(
                    "the object this copy would publish no longer exists; re-preview",
                ));
            }
            Ok(false)
        })?;
        Ok((request, already_saved))
    }
}

#[cfg(test)]
mod tests;
