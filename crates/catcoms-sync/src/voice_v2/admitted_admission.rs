//! Inert admission for the original admitted call. These owners reserve capacity
//! only; they cannot enter Existing pair creation or advertise a committed round.
use super::admission::AdmissionRpcPermit;
use super::call_activation::ActivationObserver;
use super::chain::ChainCapacity;
use super::crypto_actions::slots::Slots;
use super::packets::PacketSlots;
use super::resources::{Credit, ResourceKind, Resources};
use super::runtime::{ActorOwner, EpochObservation};
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub(super) const POOLS_BYTES: usize = 2048;
pub(super) const REGISTRATION_BYTES: usize = 3072;

struct Funding {
    _credit: Credit,
}

/// No raw funding Arc/Weak escapes. The final extraction frees its allocation
/// before returning the credit; a concurrent pair of last droppers cannot both
/// miss that extraction, as they could with a try_unwrap fallback.
struct FundingOwner(Option<Arc<Funding>>);
impl Clone for FundingOwner {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl Drop for FundingOwner {
    fn drop(&mut self) {
        if let Some(funding) = self.0.take() {
            drop(Arc::into_inner(funding));
        }
    }
}
impl FundingOwner {
    fn reserve(
        resources: &Arc<Resources>,
        group: [u8; 32],
        bytes: usize,
    ) -> Result<Self, VoiceError> {
        let credit = resources.charge(group, bytes, ResourceKind::Retained)?;
        Ok(Self(Some(Arc::new(Funding { _credit: credit }))))
    }
}

struct Pools {
    resources: Arc<Resources>,
    group: [u8; 32],
    actions: Arc<Slots>,
    packets: PacketSlots,
    #[cfg(test)]
    before_return: std::sync::Mutex<Option<ReturnBoundary>>,
}
#[cfg(test)]
struct ReturnBoundary {
    entered: tokio::sync::oneshot::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}
#[cfg(test)]
impl Drop for Pools {
    fn drop(&mut self) {
        let boundary = self.before_return.lock().unwrap().take();
        if let Some(boundary) = boundary {
            let _ = boundary.entered.send(());
            let _ = boundary.resume.recv();
        }
    }
}
/// One lazy pool set per original Presence, never per registration or retry.
/// All escaping registrations/preclaims carry this same body-first payment.
#[derive(Clone)]
pub(crate) struct PoolsOwner {
    body: Arc<Pools>,
    _funding: FundingOwner,
}
impl PoolsOwner {
    pub(crate) fn reserve(resources: &Arc<Resources>, group: [u8; 32]) -> Result<Self, VoiceError> {
        if pools_layout_bytes() > POOLS_BYTES {
            return Err(VoiceError::Capacity);
        }
        let funding = FundingOwner::reserve(resources, group, POOLS_BYTES)?;
        Ok(Self {
            body: Arc::new(Pools {
                resources: resources.clone(),
                group,
                actions: Arc::new(Slots::default()),
                packets: PacketSlots::new(),
                #[cfg(test)]
                before_return: std::sync::Mutex::new(None),
            }),
            _funding: funding,
        })
    }
    fn matches(&self, resources: &Arc<Resources>, group: [u8; 32]) -> bool {
        Arc::ptr_eq(&self.body.resources, resources) && self.body.group == group
    }
    fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.body, &other.body)
    }
    #[cfg(test)]
    fn pause_return(&self) -> (tokio::sync::oneshot::Receiver<()>, test_support::Resume) {
        let (entered, receiver) = tokio::sync::oneshot::channel();
        let (sender, resume) = std::sync::mpsc::sync_channel(1);
        *self.body.before_return.lock().unwrap() = Some(ReturnBoundary { entered, resume });
        (receiver, test_support::Resume(Some(sender)))
    }
}

struct Registration {
    original: ActivationObserver,
    pools: PoolsOwner,
    scope: Scope,
    local: DeviceId,
    epoch: u64,
    key: JoinKey,
    call: CallLease,
    watch: WatchLease,
    observation: EpochObservation,
    incarnation: Nonce,
    closed: AtomicBool,
    cancelled_through: AtomicU64,
}
impl Registration {
    fn current(&self) -> bool {
        // Native phase closure precedes some lease notifications. The original
        // NativeActive fact must therefore remain an independent conjunct.
        !self.closed.load(Ordering::Acquire)
            && self.original.is_active()
            && self.observation.is_live()
            && self.call.is_live()
            && self.watch.is_live()
    }
}

/// Affine native-held admission identity. Drop closes fresh admission only; it
/// must not revoke the underlying call or independently indexed sibling media.
pub struct AdmittedAdmissionSession {
    body: Arc<Registration>,
    _funding: FundingOwner,
}
/// Paid observation for the exact original native task, not a fresh registration
/// mint, current-generation proof, role, availability or Ready certificate.
#[derive(Clone)]
pub struct AdmittedAdmissionObserver {
    body: Arc<Registration>,
    _funding: FundingOwner,
}
/// Neutral6/6 and twelve Action claims from the original admitted call's pools.
/// There is no conversion into RootHighPreclaim or an Existing chain consumer.
pub struct RootRoundPreclaim {
    _capacity: ChainCapacity,
    generation: u64,
    // LAST: capacity may retain raw internal slot Arcs; the actual original
    // pools and their payment must survive its full destruction/deallocation.
    original: AdmittedAdmissionObserver,
}
/// Root returns the affine registration even if its later capacity reservation
/// fails. Native must stage it and retry with its SAME observer/incarnation.
pub struct AdmittedAdmissionPrepared {
    registration: Option<AdmittedAdmissionSession>,
    capacity: Result<RootRoundPreclaim, VoiceError>,
}
redacted_debug!(
    AdmittedAdmissionSession,
    AdmittedAdmissionObserver,
    RootRoundPreclaim,
    AdmittedAdmissionPrepared
);

pub(in crate::voice_v2) struct Preparation<'a> {
    pub actor: &'a ActorOwner,
    pub context: &'a Context,
    pub resources: &'a Arc<Resources>,
    pub pools: &'a PoolsOwner,
    pub original: &'a ActivationObserver,
    pub key: &'a JoinKey,
    pub call: &'a CallLease,
    pub watch: &'a WatchLease,
}
impl Preparation<'_> {
    fn current(&self) -> bool {
        self.original.is_active()
            && self.original.scope() == self.context.scope
            && self.original.uses_resources(self.resources)
            && self.call.is_admitted()
            && self.call.is_same(self.original.original_lease())
            && self.call.is_live()
            && self.watch.is_live()
            && self
                .watch
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_same(self.call))
            && self.actor.uses_resources(self.resources)
            && self.pools.matches(self.resources, self.context.scope.group)
    }
}
impl AdmittedAdmissionPrepared {
    pub(crate) fn new(
        registration: Option<AdmittedAdmissionSession>,
        capacity: Result<RootRoundPreclaim, VoiceError>,
    ) -> Self {
        Self {
            registration,
            capacity,
        }
    }
    pub fn into_parts(
        self,
    ) -> (
        Option<AdmittedAdmissionSession>,
        Result<RootRoundPreclaim, VoiceError>,
    ) {
        (self.registration, self.capacity)
    }
}
impl AdmittedAdmissionSession {
    pub(in crate::voice_v2) fn prepare(
        actual: Preparation<'_>,
        rng: &mut impl catcoms_rt::CryptoRngCore,
        before_final: impl FnOnce(),
    ) -> Result<Self, VoiceError> {
        if !actual.current() {
            return Err(VoiceError::Unauthorized);
        }
        actual
            .actor
            .try_with_current_members(actual.context, |member| {
                if member(actual.context.local) {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            })?;
        if registration_layout_bytes() > REGISTRATION_BYTES {
            return Err(VoiceError::Capacity);
        }
        let funding = FundingOwner::reserve(
            actual.resources,
            actual.context.scope.group,
            REGISTRATION_BYTES,
        )?;
        let observation = actual.actor.observe_epoch(actual.context)?;
        let mut incarnation = [0; 32];
        rng.fill_bytes(&mut incarnation);
        let owner = Self {
            body: Arc::new(Registration {
                original: actual.original.clone(),
                pools: actual.pools.clone(),
                scope: actual.context.scope,
                local: actual.context.local,
                epoch: actual.context.epoch,
                key: actual.key.clone(),
                call: actual.call.clone(),
                watch: actual.watch.clone(),
                observation,
                incarnation,
                closed: AtomicBool::new(false),
                cancelled_through: AtomicU64::new(0),
            }),
            _funding: funding,
        };
        // Injected RNG/fixture callbacks are outside Group/Policy guards. Their
        // revocation cannot leave a still-current registration behind.
        before_final();
        actual
            .actor
            .try_with_current_members(actual.context, |member| {
                if member(actual.context.local) && actual.current() && owner.is_current() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            })?;
        Ok(owner)
    }
    pub fn observer(&self) -> AdmittedAdmissionObserver {
        AdmittedAdmissionObserver {
            body: self.body.clone(),
            _funding: self._funding.clone(),
        }
    }
    pub fn incarnation(&self) -> Nonce {
        self.body.incarnation
    }
    pub fn local_device(&self) -> DeviceId {
        self.body.local
    }
    pub fn is_current(&self) -> bool {
        self.body.current()
    }
    pub fn close(&self) {
        self.body.closed.store(true, Ordering::Release);
    }
    pub fn owns(&self, observer: &AdmittedAdmissionObserver) -> bool {
        Arc::ptr_eq(&self.body, &observer.body)
    }
}
impl Drop for AdmittedAdmissionSession {
    fn drop(&mut self) {
        self.close();
    }
}
impl AdmittedAdmissionObserver {
    pub fn incarnation(&self) -> Nonce {
        self.body.incarnation
    }
    pub fn is_current(&self) -> bool {
        self.body.current()
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn unreserved_counts(&self) -> (usize, usize, usize) {
        let actions: Vec<_> = (0..32)
            .map_while(|_| self.body.pools.body.actions.charge(1).ok())
            .collect();
        let (incoming, outgoing) = self.body.pools.body.packets.fill_unreserved_for_test();
        (actions.len(), incoming.len(), outgoing.len())
    }
    pub fn cancel_through(&self, generation: u64) -> Result<(), VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        self.body
            .cancelled_through
            .fetch_max(generation, Ordering::AcqRel);
        if generation == u64::MAX {
            self.body.closed.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn matches(&self, actual: &Preparation<'_>) -> bool {
        let held = &self.body;
        self.is_current()
            && actual.current()
            && held.original.is_same(actual.original)
            && held.pools.is_same(actual.pools)
            && held.scope == actual.context.scope
            && held.local == actual.context.local
            && held.epoch == actual.context.epoch
            && held.key == *actual.key
            && held.call.is_same(actual.call)
            && held.watch.is_same(actual.watch)
    }
    pub(in crate::voice_v2) fn reserve_neutral(
        &self,
        actual: Preparation<'_>,
        generation: u64,
    ) -> Result<RootRoundPreclaim, VoiceError> {
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        if !self.matches(&actual)
            || generation <= self.body.cancelled_through.load(Ordering::Acquire)
        {
            return Err(VoiceError::Unauthorized);
        }
        let capacity = ChainCapacity::reserve_neutral(
            actual.resources.clone(),
            actual.context.scope.group,
            actual.pools.body.actions.clone(),
            &actual.pools.body.packets,
        )?;
        let owner = RootRoundPreclaim {
            _capacity: capacity,
            generation,
            original: self.clone(),
        };
        actual
            .actor
            .try_with_current_members(actual.context, |member| {
                if member(actual.context.local) && self.matches(&actual) && owner.is_current() {
                    Ok(())
                } else {
                    Err(VoiceError::Unauthorized)
                }
            })?;
        Ok(owner)
    }
}
impl RootRoundPreclaim {
    #[cfg(test)]
    pub(in crate::voice_v2) fn pause_pool_return(
        &self,
    ) -> (tokio::sync::oneshot::Receiver<()>, test_support::Resume) {
        self.original.body.pools.pause_return()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn belongs_to(&self, session: &AdmittedAdmissionSession) -> bool {
        session.owns(&self.original)
    }
    pub fn is_current(&self) -> bool {
        self.original.is_current()
            && self.generation > self.original.body.cancelled_through.load(Ordering::Acquire)
    }
}

/// Harness-owned rendezvous/OS thread allocations are not production capacity.
/// A watchdog failure never certifies that an actual producer returned. Resume is
/// nonblocking RAII, so a failed observation still unpauses the real destructor.
#[cfg(test)]
pub(in crate::voice_v2) mod test_support {
    use catcoms_rt::{Clock, SystemClock};
    use std::future::Future;
    use std::time::Duration;
    pub(in crate::voice_v2) struct Resume(pub(super) Option<std::sync::mpsc::SyncSender<()>>);
    impl Drop for Resume {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.try_send(());
            }
        }
    }
    pub(in crate::voice_v2) async fn bounded<T>(future: impl Future<Output = T>) -> T {
        tokio::select! {
            value = future => value,
            _ = SystemClock.sleep(Duration::from_secs(30)) => panic!("admitted neutral actual-return fixture exceeded its bound"),
        }
    }
    pub(in crate::voice_v2) struct Producer<T> {
        returned: tokio::sync::oneshot::Receiver<T>,
        thread: std::thread::JoinHandle<()>,
    }
    impl<T: Send + 'static> Producer<T> {
        pub(in crate::voice_v2) fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
            let (send, returned) = tokio::sync::oneshot::channel();
            let thread = std::thread::spawn(move || {
                let value = work();
                let _ = send.send(value);
            });
            Self { returned, thread }
        }
        pub(in crate::voice_v2) async fn join(self) -> T {
            let value = bounded(self.returned)
                .await
                .expect("actual producer panicked before returning");
            bounded(async {
                while !self.thread.is_finished() {
                    SystemClock.sleep(Duration::from_millis(1)).await;
                }
            })
            .await;
            self.thread.join().expect("actual producer thread failed");
            value
        }
    }
}

#[repr(C, align(2))]
struct ArcAllocation<T> {
    strong: usize,
    weak: usize,
    value: T,
}
fn pools_layout_bytes() -> usize {
    use std::mem::size_of;
    size_of::<ArcAllocation<Pools>>()
        + size_of::<ArcAllocation<Funding>>()
        + 3 * size_of::<ArcAllocation<Slots>>()
        + size_of::<Pools>()
        + 4 * size_of::<PoolsOwner>()
        + 2 * size_of::<FundingOwner>()
        + 128
}
fn registration_layout_bytes() -> usize {
    use std::mem::size_of;
    size_of::<ArcAllocation<Registration>>()
        + size_of::<ArcAllocation<Funding>>()
        + size_of::<Registration>()
        + size_of::<Preparation<'_>>()
        + size_of::<AdmittedAdmissionSession>()
        + 3 * size_of::<AdmittedAdmissionObserver>()
        + size_of::<AdmittedAdmissionPrepared>()
        + size_of::<RootRoundPreclaim>()
        + EpochObservation::retained_graph_bytes()
        + size_of::<AdmissionRpcPermit>()
        + 128
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::{bounded, Producer};

    #[test]
    fn admitted_neutral_complete_pools_and_registration_graphs_fit_original_payments() {
        let pools = pools_layout_bytes();
        let registration = registration_layout_bytes();
        eprintln!("admitted neutral pools {pools}/{POOLS_BYTES}; registration {registration}/{REGISTRATION_BYTES}");
        assert!(pools <= POOLS_BYTES);
        assert!(registration <= REGISTRATION_BYTES);
    }

    #[tokio::test]
    async fn admitted_neutral_pool_payment_survives_actual_last_body_destruction() {
        let resources = Arc::new(Resources::default());
        let pools = PoolsOwner::reserve(&resources, [71; 32]).unwrap();
        let escaped = pools.clone();
        let (entered, resume) = pools.pause_return();
        drop(pools);
        let actual = Producer::spawn(move || drop(escaped));
        bounded(entered).await.unwrap();
        let sampler = resources.clone();
        let during_destruction = Producer::spawn(move || sampler.counts()).join().await;
        drop(resume);
        actual.join().await;
        assert_eq!(
            during_destruction,
            (POOLS_BYTES, 0),
            "strong_count zero is not actual body/header return"
        );
        assert_eq!(resources.counts(), (0, 0));
    }

    #[tokio::test]
    async fn admitted_neutral_concurrent_final_funding_owners_return_the_one_original_payment() {
        let resources = Arc::new(Resources::default());
        let original = FundingOwner::reserve(&resources, [72; 32], POOLS_BYTES).unwrap();
        let other = original.clone();
        let (a_send, a_start) = std::sync::mpsc::sync_channel(1);
        let (b_send, b_start) = std::sync::mpsc::sync_channel(1);
        let a = Producer::spawn(move || {
            let _ = a_start.recv();
            drop(original);
        });
        let b = Producer::spawn(move || {
            let _ = b_start.recv();
            drop(other);
        });
        let a_resume = test_support::Resume(Some(a_send));
        let b_resume = test_support::Resume(Some(b_send));
        assert_eq!(resources.counts(), (POOLS_BYTES, 0));
        drop((a_resume, b_resume));
        a.join().await;
        b.join().await;
        assert_eq!(resources.counts(), (0, 0));
    }
}
