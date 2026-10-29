//! Inactive admitted-call selection. This owns local profile/activation custody only:
//! it creates no key delegation, packet, Ready, SDP or RTC permission. The separate
//! Root-current admitted-presence API may publish discovery after NativeActive.
//! Native keeps its original UI commit guard while selecting and finally activating.
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::EpochObservation;
use super::{CallGrant, CallLease, CallProfile, Scope, VoiceError, WatchLease};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Weak};

/// Independently paid complete original activation/weak-allocation graph. Its compiled
/// inventory test is required before this staged API may be enabled by production.
pub const ACTIVATION_BYTES: usize = 4096;
/// Actual Actor argument/callback/retirement graph, separate from retained activation.
pub const ACTIVATION_RPC_BYTES: usize = 4096;
pub(super) const CANDIDATE: u8 = 0;
const ROOT_SELECTED: u8 = 1;
pub(super) const NATIVE_ACTIVE: u8 = 2;
pub(super) const CLOSED: u8 = 3;
const RESIDENCE_AVAILABLE: u8 = 0;
const RESIDENCE_HELD: u8 = 1;
const RESIDENCE_SEALED: u8 = 2;

pub(crate) struct Funding {
    resources: Arc<Resources>,
    group: [u8; 32],
    epoch: u64,
    _credit: Credit,
}
impl Funding {
    fn reserve(
        resources: &Arc<Resources>,
        scope: Scope,
        epoch: u64,
    ) -> Result<Arc<Self>, VoiceError> {
        Self::reserve_bytes(resources, scope, epoch, ACTIVATION_BYTES)
    }
    fn reserve_bytes(
        resources: &Arc<Resources>,
        scope: Scope,
        epoch: u64,
        bytes: usize,
    ) -> Result<Arc<Self>, VoiceError> {
        Ok(Arc::new(Self {
            resources: resources.clone(),
            group: scope.group,
            epoch,
            _credit: resources.charge(scope.group, bytes, ResourceKind::Retained)?,
        }))
    }
    /// Separate original Presence payment, never activation's existing4KiB. The
    /// private watcher child carries this SAME payer through escaped Source/peer
    /// aliases and final lease allocation destruction; no Weak Funding escapes.
    pub(super) fn reserve_presence(original: &ActivationObserver) -> Result<Arc<Self>, VoiceError> {
        Self::reserve_bytes(
            &original.core.resources,
            original.scope(),
            original.funding.epoch,
            super::admitted_presence::PRESENCE_BYTES,
        )
    }
    #[cfg(test)]
    pub(super) const fn allocation_bytes() -> usize {
        std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>()
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) fn matches(&self, resources: &Arc<Resources>, group: [u8; 32]) -> bool {
        Arc::ptr_eq(&self.resources, resources) && self.group == group
    }
}
struct ReturnState {
    ticket: AtomicU8,
    returned: AtomicBool,
    #[cfg(test)]
    before_return: std::sync::Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
}
struct Core {
    resources: Arc<Resources>,
    // The original Root's small identity allocation; no Root/Room/Actor backedge.
    instance: Arc<()>,
    scope: Scope,
    grant: Weak<CallGrant>,
    call: CallLease,
    epoch: EpochObservation,
    returned: Arc<ReturnState>,
}

/// The only holder of a raw Core Arc. Every clone pairs its allocation/Weak tail with
/// the SAME plain payer, declared last so actual Core deallocation precedes refund.
#[derive(Clone)]
pub struct ActivationObserver {
    core: Arc<Core>,
    funding: Arc<Funding>,
}
/// Affine native custody. Metadata observers cannot keep an abandoned call authorized.
pub struct ActivationOwner {
    original: ActivationObserver,
}
/// Negative-control routing identity only. It retains no call lease, Core,
/// Grant/Room or Actor. Signature/role/epoch/readiness cannot be inferred from it.
/// An escaping native/App holder must independently sponsor this small original
/// allocation and resources handle through its own actual final destruction.
#[derive(Clone)]
pub struct ActivationInstancePin {
    instance: Arc<()>,
    resources: Arc<Resources>,
    group: [u8; 32],
}
impl ActivationInstancePin {
    pub fn matches(&self, original: &ActivationObserver) -> bool {
        Arc::ptr_eq(&self.instance, &original.core.instance)
            && Arc::ptr_eq(&self.resources, &original.core.resources)
            && self.group == original.scope().group
    }
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.instance, &other.instance)
            && Arc::ptr_eq(&self.resources, &other.resources)
            && self.group == other.group
    }
}
impl Drop for ActivationOwner {
    fn drop(&mut self) {
        self.original.close();
    }
}
/// One original fresh child and activation. No Root-current selection precedes native CAS.
pub struct PreparedAdmittedCall {
    grant: Arc<CallGrant>,
    owner: ActivationOwner,
}
impl PreparedAdmittedCall {
    pub(crate) fn prepare(
        directory: &super::directory::Directory,
        resources: Arc<Resources>,
        instance: Arc<()>,
        actor: &super::runtime::ActorOwner,
        context: &super::Context,
    ) -> Result<Self, VoiceError> {
        let epoch = actor.observe_epoch(context)?;
        actor.collect_closed_children();
        directory.collect_all_closed_children();
        // Funding precedes child ALLOCATION/issuer registration. Even construction
        // failure leaves every real issuer alias carrying its original plain payer.
        let funding = Funding::reserve(&resources, context.scope, context.epoch)?;
        let grant =
            Arc::new(directory.admitted_call_grant(actor, context.scope, funding.clone())?);
        if !grant.lease().is_admitted() || !grant.lease().is_live() || !epoch.is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let original = ActivationObserver {
            core: Arc::new(Core {
                resources,
                instance,
                scope: grant.scope(),
                grant: Arc::downgrade(&grant),
                call: grant.lease(),
                epoch,
                returned: Arc::new(ReturnState {
                    ticket: AtomicU8::new(RESIDENCE_AVAILABLE),
                    returned: AtomicBool::new(false),
                    #[cfg(test)]
                    before_return: std::sync::Mutex::new(None),
                }),
            }),
            funding,
        };
        Ok(Self {
            grant,
            owner: ActivationOwner { original },
        })
    }
    pub fn grant(&self) -> &Arc<CallGrant> {
        &self.grant
    }
    pub fn observer(&self) -> ActivationObserver {
        self.owner.observer()
    }
    /// Move the SAME native pair; never wrap its lease in a replacement CallGrant Arc.
    pub fn into_parts(self) -> (Arc<CallGrant>, ActivationOwner) {
        (self.grant, self.owner)
    }
}
impl ActivationOwner {
    pub fn observer(&self) -> ActivationObserver {
        self.original.clone()
    }
}
impl ActivationObserver {
    pub fn instance_pin(&self) -> ActivationInstancePin {
        ActivationInstancePin {
            instance: self.core.instance.clone(),
            resources: self.core.resources.clone(),
            group: self.scope().group,
        }
    }
    pub fn scope(&self) -> Scope {
        self.core.scope
    }
    pub fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.core, &other.core)
    }
    pub fn matches_grant(&self, grant: &Arc<CallGrant>) -> bool {
        self.core.grant.ptr_eq(&Arc::downgrade(grant))
            && self.core.call.is_same(&grant.lease())
            && grant.scope() == self.scope()
            && grant.lease().is_admitted()
    }
    pub fn is_closed(&self) -> bool {
        self.core.call.activation_phase().load(Ordering::Acquire) == CLOSED
            || !self.core.call.is_live()
            || !self.core.epoch.is_live()
    }
    /// Logical currency only, never an owner-return certificate or endpoint permission.
    pub fn is_active(&self) -> bool {
        self.core.call.activation_phase().load(Ordering::Acquire) == NATIVE_ACTIVE
            && !self.is_closed()
    }
    /// Whole-call negative fence. Ordinary admission-generation cancellation must not call it.
    pub fn close(&self) {
        // Reuse the original paid lease notification. No second winner cell or async
        // callback registry is needed to wake a guard-owning native task on lock/End.
        self.core.call.revoke();
    }
    /// Pure negative half for native admission/exceptional teardown while an
    /// ORIGINAL UI/Coordinator guard is held. It closes the SAME issuer phase,
    /// without executing a watch waker. The owner must call notify_closed only
    /// after releasing its guards; no new positive authority can race this split.
    pub fn invalidate(&self) {
        self.core.call.invalidate();
    }
    /// Complete an already-negative fence outside all admission mutexes. This is
    /// only a wake, never an owner-return certificate or an implicit result ACK.
    pub fn notify_closed(&self) {
        if self.is_closed() {
            self.core.call.notify_revoked();
        }
    }
    // The inactive native task deliberately uses its original paid Clock, not
    // this child watch: inherited native/Runtime revocation may notify while
    // holding guards. Keep direct arbitrary-waker probing inside these unit tests.
    #[cfg(test)]
    pub(in crate::voice_v2) async fn wait_closed(&self) {
        self.core.call.wait_revoked().await;
    }
    pub(crate) fn belongs_to(&self, resources: &Arc<Resources>, instance: &Arc<()>) -> bool {
        self.uses_resources(resources) && Arc::ptr_eq(&self.core.instance, instance)
    }
    /// Pure identity of the original funding pool, including after closure. Native
    /// may refuse a foreign-pool Handle before reserving index/generation fees.
    /// This grants no Root-instance, grant, selection or present-time authority;
    /// those exact current checks remain independently required at every effect.
    pub fn uses_resources(&self, resources: &Arc<Resources>) -> bool {
        Arc::ptr_eq(&self.core.resources, resources)
    }
    pub(crate) fn root_instance(&self) -> &Arc<()> {
        &self.core.instance
    }
    pub(crate) fn original_lease(&self) -> &CallLease {
        &self.core.call
    }
    pub(crate) fn grant(&self) -> Option<Arc<CallGrant>> {
        self.core.grant.upgrade()
    }
    pub fn reserve_rpc(&self) -> Result<ActivationRpcPermit, VoiceError> {
        Ok(ActivationRpcPermit {
            resources: self.core.resources.clone(),
            scope: self.scope(),
            _credit: self.core.resources.charge(
                self.scope().group,
                ACTIVATION_RPC_BYTES,
                ResourceKind::Retained,
            )?,
        })
    }
    /// Native task/canonical-cell funding must be reserved BEFORE their allocations;
    /// it is local trusted bookkeeping, never a peer/renderer-selected budget increase.
    pub fn reserve_retained(&self, bytes: usize) -> Result<Credit, VoiceError> {
        self.core
            .resources
            .charge(self.scope().group, bytes, ResourceKind::Retained)
    }
    pub fn root_returned(&self) -> Option<RootResidenceReturned> {
        if self.core.returned.returned.load(Ordering::Acquire) {
            Some(RootResidenceReturned {
                cell: self.core.returned.clone(),
                _funding: self.funding.clone(),
            })
        } else {
            None
        }
    }
    /// Close first, then consume the SAME original acquisition right without giving
    /// it to Root. This is a distinct zero-residence proof, NOT actual Work/argument/
    /// Actor/grant return. Once Root acquired the ticket only its destructor can certify.
    pub fn seal_unclaimed_residence(&self) -> Option<RootResidenceUnclaimed> {
        self.close();
        match self.core.returned.ticket.compare_exchange(
            RESIDENCE_AVAILABLE,
            RESIDENCE_SEALED,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) | Err(RESIDENCE_SEALED) => Some(RootResidenceUnclaimed {
                cell: self.core.returned.clone(),
                _funding: self.funding.clone(),
            }),
            Err(_) => None,
        }
    }
    pub(crate) fn select(&self) -> Result<(RootResidence, RootSelectedCall), VoiceError> {
        if self.is_closed() {
            return Err(VoiceError::Unauthorized);
        }
        // Construct the paid unarmed marker BEFORE acquisition. After winning there
        // is only a nonpanicking local arm; no hook/allocation/fallible step may be
        // inserted in that gap. An entered unwind must never strand an ownerless Held.
        let mut returned = ReturnMarker {
            cell: self.core.returned.clone(),
            _funding: self.funding.clone(),
            armed: false,
        };
        self.core
            .returned
            .ticket
            .compare_exchange(
                RESIDENCE_AVAILABLE,
                RESIDENCE_HELD,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| VoiceError::RetryMismatch)?;
        returned.armed = true;
        // Own the marker BEFORE the phase CAS. Closing immediately after a winning CAS can
        // leave a closed residence; it is still real ownership, never "never entered".
        let residence = RootResidence {
            original: Some(self.clone()),
            presence: None,
            _returned: returned,
        };
        self.core
            .call
            .activation_phase()
            .compare_exchange(
                CANDIDATE,
                ROOT_SELECTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| VoiceError::Unauthorized)?;
        Ok((
            residence,
            RootSelectedCall {
                original: self.clone(),
            },
        ))
    }
    pub(crate) fn selected_receipt(&self) -> Result<RootSelectedCall, VoiceError> {
        if !self.is_closed()
            && matches!(
                self.core.call.activation_phase().load(Ordering::Acquire),
                ROOT_SELECTED | NATIVE_ACTIVE
            )
        {
            Ok(RootSelectedCall {
                original: self.clone(),
            })
        } else {
            Err(VoiceError::Unauthorized)
        }
    }
}

/// The actual Root selection result. Consumption belongs only inside the native
/// ORIGINAL UI-guard/exact-owner/generation fence after actual Actor Work retirement.
/// This does not itself publish a key/endpoint; the distinct admitted-presence
/// factory must revalidate the exact current winner after this native commit.
pub struct RootSelectedCall {
    original: ActivationObserver,
}
impl RootSelectedCall {
    pub fn original(&self) -> &ActivationObserver {
        &self.original
    }
    pub fn commit_native(self, grant: &Arc<CallGrant>) -> Result<(), VoiceError> {
        if self.original.is_closed() || !self.original.matches_grant(grant) {
            return Err(VoiceError::Unauthorized);
        }
        match self.original.core.call.activation_phase().compare_exchange(
            ROOT_SELECTED,
            NATIVE_ACTIVE,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(NATIVE_ACTIVE) if self.original.is_active() => Ok(()),
            Err(_) => Err(VoiceError::Unauthorized),
        }
    }
}

/// Original Root storage alias. Its unique marker certifies ONLY this residence's
/// actual release, not native task/argument/key/SDK return or complete fee refund.
pub(crate) struct RootResidence {
    original: Option<ActivationObserver>,
    pub(crate) presence: Option<super::admitted_presence::PresenceStorage>,
    _returned: ReturnMarker,
}
impl RootResidence {
    pub(crate) fn original(&self) -> &ActivationObserver {
        self.original.as_ref().expect("original residence")
    }
    pub(crate) fn watch_lease(&self, funding: Arc<Funding>) -> Result<WatchLease, VoiceError> {
        let original = self.original();
        if !original.is_active()
            || !funding.matches(&original.core.resources, original.scope().group)
            || funding.epoch != original.funding.epoch
        {
            return Err(VoiceError::Unauthorized);
        }
        // This private unregistered child supplies only Watcher's independent
        // one-shot claim. It is NEVER a directory/Actor-issued call grant. The
        // actual admitted parent already supplies all issuer/room/epoch closure.
        let watch = WatchLease::new(CallLease::new(CallProfile::Existing, Some(funding)));
        watch.with_admitted_parent(original)
    }
}
impl Drop for RootResidence {
    fn drop(&mut self) {
        // Caller detaches outside Group/Policy/native guards. The Core alias and all
        // its owned fields are actually destroyed BEFORE the marker is published.
        if let Some(original) = self.original.take() {
            original.close();
            // The watcher and boxed presence are part of THIS residence's return,
            // not sibling endpoint state. Its external plain payer outlives Box free.
            drop(self.presence.take());
            drop(original);
        }
        // `_returned` drops after this body, including if an original field unwinds.
    }
}
struct ReturnMarker {
    cell: Arc<ReturnState>,
    _funding: Arc<Funding>,
    armed: bool,
}
impl Drop for ReturnMarker {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(test)]
        if let Some((entered, resume)) = self.cell.before_return.lock().unwrap().take() {
            entered.wait();
            resume.wait();
        }
        self.cell.returned.store(true, Ordering::Release);
    }
}
/// Opaque original marker receipt; constructed only after actual residence destruction.
pub struct RootResidenceReturned {
    cell: Arc<ReturnState>,
    _funding: Arc<Funding>,
}
/// The original Root-storage acquisition right was permanently consumed negatively.
/// It proves no Root residence existed, while real command/task custody stays separate.
pub struct RootResidenceUnclaimed {
    cell: Arc<ReturnState>,
    _funding: Arc<Funding>,
}
impl RootResidenceUnclaimed {
    pub fn matches(&self, original: &ActivationObserver) -> bool {
        Arc::ptr_eq(&self.cell, &original.core.returned)
    }
}
impl RootResidenceReturned {
    pub fn matches(&self, original: &ActivationObserver) -> bool {
        Arc::ptr_eq(&self.cell, &original.core.returned)
    }
}
pub struct ActivationRpcPermit {
    resources: Arc<Resources>,
    scope: Scope,
    _credit: Credit,
}
impl ActivationRpcPermit {
    /// Local trusted preparation funding, before a child exists. It does not issue
    /// a grant or choose profile; the real Root validates its pool/scope before use.
    pub fn reserve(resources: &Arc<Resources>, scope: Scope) -> Result<Self, VoiceError> {
        Ok(Self {
            resources: resources.clone(),
            scope,
            _credit: resources.charge(scope.group, ACTIVATION_RPC_BYTES, ResourceKind::Retained)?,
        })
    }
    pub fn matches(&self, original: &ActivationObserver) -> bool {
        self.scope == original.scope() && Arc::ptr_eq(&self.resources, &original.core.resources)
    }
    pub(crate) fn matches_scope(&self, resources: &Arc<Resources>, scope: Scope) -> bool {
        self.scope == scope && Arc::ptr_eq(&self.resources, resources)
    }
}
redacted_debug!(
    PreparedAdmittedCall,
    ActivationOwner,
    ActivationObserver,
    ActivationInstancePin,
    RootSelectedCall,
    RootResidenceReturned,
    RootResidenceUnclaimed,
    ActivationRpcPermit
);

#[cfg(test)]
mod tests;
