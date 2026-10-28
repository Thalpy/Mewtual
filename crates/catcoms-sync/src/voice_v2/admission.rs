//! Original native admission registration and inert HIGH capacity. None of these
//! owners advertises availability, creates an agreement/Action or starts a clock.
//! Native must separately reserve its own ledger row before any future wire grant.
use super::chain::ChainCapacity;
use super::crypto_actions::slots::Slots;
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::{ActorOwner, EpochObservation};
use super::{CallLease, Context, DeviceId, JoinKey, Nonce, Scope, VoiceError, WatchLease};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Separate payment for the registration graph, not slack from Root's2KiB chain
/// metadata or a new group pool. Includes complete retained Tokio watch allocations;
/// the supported Windows inventory is2168B, which does not fit a2KiB payment.
pub const REGISTRATION_METADATA_BYTES: usize = 3 * 1024;

/// Separate ordinary payment for the actual Actor queue/publisher/retirement graph.
/// It is not registration, protocol capacity or the native worker's payment. Native
/// must measure its concrete publisher before Box construction within this allowance.
pub const ADMISSION_RPC_BYTES: usize = 4 * 1024;

/// Opaque fixed contribution. A caller cannot substitute a small arbitrary Credit,
/// another group, or a newly created pool for the selected Root's actual pool.
pub struct AdmissionRpcPermit {
    resources: Arc<Resources>,
    scope: Scope,
    // LAST: borrowed arguments and original pool identity retire before payment.
    _credit: Credit,
}
impl AdmissionRpcPermit {
    pub fn reserve(resources: &Arc<Resources>, scope: Scope) -> Result<Self, VoiceError> {
        let credit = resources.charge(scope.group, ADMISSION_RPC_BYTES, ResourceKind::Retained)?;
        Ok(Self {
            resources: resources.clone(),
            scope,
            _credit: credit,
        })
    }
    pub fn scope(&self) -> Scope {
        self.scope
    }
    pub(crate) fn matches(&self, resources: &Arc<Resources>, scope: Scope) -> bool {
        Arc::ptr_eq(&self.resources, resources) && self.scope == scope
    }
}

/// Actual Root preparation output, not a Ready/ledger-row certificate. Registration
/// is returned independently of capacity failure so native cannot accidentally remint
/// the public incarnation when the ordinary capacity pool is temporarily full.
pub struct AdmissionPrepared {
    registration: Option<RootAdmissionSession>,
    capacity: Result<RootHighPreclaim, VoiceError>,
}
impl AdmissionPrepared {
    pub(crate) fn new(
        registration: Option<RootAdmissionSession>,
        capacity: Result<RootHighPreclaim, VoiceError>,
    ) -> Self {
        Self {
            registration,
            capacity,
        }
    }
    /// Consumes the exact original result; neither owner is cloned or reconstructed.
    pub fn into_parts(
        self,
    ) -> (
        Option<RootAdmissionSession>,
        Result<RootHighPreclaim, VoiceError>,
    ) {
        (self.registration, self.capacity)
    }
}
redacted_debug!(AdmissionRpcPermit, AdmissionPrepared);

struct Registration {
    resources: Arc<Resources>,
    slots: Arc<Slots>,
    scope: Scope,
    local: DeviceId,
    epoch: u64,
    join: Nonce,
    key_ref: Nonce,
    incarnation: Nonce,
    call: CallLease,
    watch: WatchLease,
    observation: EpochObservation,
    closed: AtomicBool,
    cancelled_through: AtomicU64,
    #[cfg(test)]
    before_deallocation: std::sync::Mutex<Option<ReturnBoundary>>,
}
#[cfg(test)]
struct ReturnBoundary {
    entered: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}
#[cfg(test)]
impl Drop for ReturnBoundary {
    fn drop(&mut self) {
        self.entered.wait();
        self.resume.wait();
    }
}
// Plain payment only: no registration, observer, native Row or action backedge.
struct Funding {
    _credit: Credit,
}

/// Affine native-held registration owner. Dropping it irreversibly closes only
/// this admission registration; established calls/media have independent custody.
/// Exact native remount/reply recovery must retain THIS owner, never call prepare again.
pub struct RootAdmissionSession {
    registration: Arc<Registration>,
    // Last, through actual final registration-body destruction and Arc deallocation.
    funding: Arc<Funding>,
}

/// Opaque paid observation for queued Actor work. It contains no Actor/Group/Row,
/// workflow, native Session secret, key, execution lane or capacity reservation.
#[derive(Clone)]
pub struct AdmissionSessionObserver {
    registration: Arc<Registration>,
    _funding: Arc<Funding>,
}

/// Affine HIGH-layout reservation from the original LocalJoin's actual pools.
/// This is neither DeviceId orientation proof nor permission to accept a request.
/// No conversion into legacy pair creation is exposed; agreement/round integration
/// must provide its own reviewed consume boundary later.
pub struct RootHighPreclaim {
    _capacity: ChainCapacity,
    registration: AdmissionSessionObserver,
    generation: u64,
}
redacted_debug!(
    RootAdmissionSession,
    AdmissionSessionObserver,
    RootHighPreclaim
);

pub(in crate::voice_v2) struct Preparation<'a> {
    pub actor: &'a ActorOwner,
    pub context: &'a Context,
    pub resources: &'a Arc<Resources>,
    pub slots: &'a Arc<Slots>,
    pub call: &'a CallLease,
    pub watch: &'a WatchLease,
    pub key: &'a JoinKey,
}

impl RootAdmissionSession {
    pub(in crate::voice_v2) fn prepare(
        original: Preparation<'_>,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        before_final: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        let Preparation {
            actor,
            context,
            resources,
            slots,
            call,
            watch,
            key,
        } = original;
        if !call.is_live() || !watch.is_live() || !actor.uses_resources(resources) {
            return Err(VoiceError::Unauthorized);
        }
        actor.try_with_current_members(context, |member| {
            if member(context.local) {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        })?;
        let credit = resources.charge(
            context.scope.group,
            REGISTRATION_METADATA_BYTES,
            ResourceKind::Retained,
        )?;
        let observation = actor.observe_epoch(context)?;
        let mut incarnation = [0; 32];
        // Actual Root's sanctioned cryptographic RNG; no native/Group/Policy guard.
        rng.fill_bytes(&mut incarnation);
        // Publish the plain anchor first. If later core construction unwinds, its
        // partial body is destroyed before this enclosing payment owner returns.
        let funding = Arc::new(Funding { _credit: credit });
        let owner = Self {
            registration: Arc::new(Registration {
                resources: resources.clone(),
                slots: slots.clone(),
                scope: context.scope,
                local: context.local,
                epoch: context.epoch,
                join: key.join,
                key_ref: key.key_ref,
                incarnation,
                call: call.clone(),
                watch: watch.clone(),
                observation,
                closed: AtomicBool::new(false),
                cancelled_through: AtomicU64::new(0),
                #[cfg(test)]
                before_deallocation: std::sync::Mutex::new(None),
            }),
            funding,
        };
        // Production supplies a zero-sized no-op. The private fixture adapter can
        // revoke the ACTUAL grant/session or unwind at this exact boundary, after
        // allocation but before the final currency check. No callback crosses a guard.
        before_final();
        // RNG/allocation preparation is outside Group/Policy. Keep owner OUTSIDE
        // this borrowed closure so any failure destroys it after those guards return.
        actor.try_with_current_members(context, |member| {
            if member(context.local) && owner.is_current() {
                Ok(())
            } else {
                Err(VoiceError::Unauthorized)
            }
        })?;
        Ok(owner)
    }

    pub fn observer(&self) -> AdmissionSessionObserver {
        AdmissionSessionObserver {
            registration: self.registration.clone(),
            _funding: self.funding.clone(),
        }
    }
    /// New random public registration identity, never a hash of private native Session data.
    pub fn incarnation(&self) -> Nonce {
        self.registration.incarnation
    }
    /// Original Root identity; never supplied by a renderer or inferred from fixture order.
    pub fn local_device(&self) -> DeviceId {
        self.registration.local
    }
    pub fn is_current(&self) -> bool {
        self.registration.current()
    }
    pub fn close(&self) {
        self.registration.closed.store(true, Ordering::Release);
    }
    pub fn owns(&self, observer: &AdmissionSessionObserver) -> bool {
        Arc::ptr_eq(&self.registration, &observer.registration)
    }
    #[cfg(test)]
    pub(crate) fn fixture_pause_final_body_return(
        &self,
        entered: Arc<std::sync::Barrier>,
        resume: Arc<std::sync::Barrier>,
    ) {
        *self.registration.before_deallocation.lock().unwrap() =
            Some(ReturnBoundary { entered, resume });
    }
}
impl Drop for RootAdmissionSession {
    fn drop(&mut self) {
        self.close();
    }
}
impl Registration {
    fn current(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self.observation.is_live()
            && self.call.is_live()
            && self.watch.is_live()
    }
}
impl AdmissionSessionObserver {
    pub fn incarnation(&self) -> Nonce {
        self.registration.incarnation
    }
    pub fn is_current(&self) -> bool {
        self.registration.current()
    }
    /// Negative-only native fence, including cancel-before-reserve. Native must
    /// validate its exact expected ordinal BEFORE calling this. It proves no actual
    /// return/refund and cannot revoke established sibling media. Old cancellation
    /// never clears a later generation; u64::MAX permanently exhausts this registration.
    pub fn cancel_through(&self, generation: u64) -> Result<(), VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        self.registration
            .cancelled_through
            .fetch_max(generation, Ordering::AcqRel);
        if generation == u64::MAX {
            self.registration.closed.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub(in crate::voice_v2) fn matches(&self, actual: &Preparation<'_>) -> bool {
        let original = &self.registration;
        original.current()
            && Arc::ptr_eq(&original.resources, actual.resources)
            && Arc::ptr_eq(&original.slots, actual.slots)
            && original.scope == actual.context.scope
            && original.local == actual.context.local
            && original.epoch == actual.context.epoch
            && original.join == actual.key.join
            && original.key_ref == actual.key.key_ref
            && original.call.is_same(actual.call)
            && original.watch.is_same(actual.watch)
            && actual.actor.uses_resources(actual.resources)
    }
    pub(in crate::voice_v2) fn reserve_high(
        &self,
        actual: Preparation<'_>,
        packets: &super::packets::PacketSlots,
        generation: u64,
    ) -> Result<RootHighPreclaim, VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        if !self.matches(&actual)
            || generation <= self.registration.cancelled_through.load(Ordering::Acquire)
        {
            return Err(VoiceError::Unauthorized);
        }
        let capacity = ChainCapacity::reserve(
            actual.resources.clone(),
            actual.context.scope.group,
            false,
            actual.slots.clone(),
            packets,
        )?;
        let claim = RootHighPreclaim {
            _capacity: capacity,
            registration: self.clone(),
            generation,
        };
        // The exact CURRENT packet/action pools are reborrowed at reserve. Future
        // consumption must reborrow them again through ChainCapacity's existing check.
        actual
            .actor
            .try_with_current_members(actual.context, |member| {
                if member(actual.context.local) && self.matches(&actual) && claim.is_current() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            })?;
        Ok(claim)
    }
}
impl RootHighPreclaim {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Capacity-only observation. Native still checks its canonical Session/slot
    /// allocation; this grants no agreement, ledger row, packet, transcript or RTC work.
    pub fn is_current(&self) -> bool {
        self.registration.is_current()
            && self.generation
                > self
                    .registration
                    .registration
                    .cancelled_through
                    .load(Ordering::Acquire)
    }
    pub fn belongs_to(&self, owner: &RootAdmissionSession) -> bool {
        owner.owns(&self.registration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[repr(C, align(2))]
    struct ArcAllocation<T> {
        strong: usize,
        weak: usize,
        value: T,
    }

    // Tokio1.53.3 sync/watch.rs::Shared<bool>, BigNotify (lines296-317,387-390).
    // Include all nine public Notify bodies and even BigNotify's optional cursor.
    //64B covers the supported std RwLock<bool> and parking_lot0.12.5's
    // RawRwLock(AtomicUsize) + bool/lock_api wrapper. Re-audit on dependency/target
    // changes. Notifications retain no new waiter here: this leaf never subscribes.
    #[repr(C)]
    struct WatchBacking {
        value_lock_and_bool: [usize; 8],
        state: std::sync::atomic::AtomicUsize,
        receivers: std::sync::atomic::AtomicUsize,
        senders: std::sync::atomic::AtomicUsize,
        cursor: std::sync::atomic::AtomicUsize,
        notify_receivers: [tokio::sync::Notify; 8],
        notify_senders: tokio::sync::Notify,
    }

    #[test]
    fn complete_registration_graph_and_preparation_fit_its_independent_payment() {
        // Structural application-owned graph. Include whole retained Arc bodies,
        // not just pointers to an old allocator/lease. There are at most call,
        // watch child and watch parent; count all three conservatively even
        // though the current implicit watcher aliases its parent to call.
        // General allocator/thread/runtime overhead is outside this inventory.
        assert!(size_of::<std::sync::RwLock<bool>>() <= size_of::<[usize; 8]>());
        assert!(
            std::mem::align_of::<std::sync::RwLock<bool>>() <= std::mem::align_of::<WatchBacking>()
        );
        let retained = size_of::<ArcAllocation<Registration>>()
            + size_of::<ArcAllocation<Funding>>()
            + size_of::<ArcAllocation<Slots>>()
            + EpochObservation::retained_graph_bytes()
            + 3 * (size_of::<ArcAllocation<super::super::LeaseState>>()
                + size_of::<ArcAllocation<WatchBacking>>())
            + size_of::<RootAdmissionSession>()
            + 2 * size_of::<AdmissionSessionObserver>()
            + size_of::<RootHighPreclaim>();
        let bytes = retained + size_of::<Preparation<'_>>() + 128;
        assert!(
            bytes <= REGISTRATION_METADATA_BYTES,
            "registration retained/preparation graph {bytes}/{}",
            REGISTRATION_METADATA_BYTES
        );
    }
}
