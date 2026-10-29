//! Immutable originals, with independent query-currentness and source-send gates.
//! Neither a transport ACK nor a FreshPeer is a Support observation.
use super::*;
use futures::FutureExt;
use paid::{Payment, Retained};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};

pub(super) const RECORD_BYTES: usize = 8192;
// The actual affine byte quantum is also the lifetime occupancy mechanism,
// including escaped superseded handles. A later cap raise or fee reduction must
// introduce an explicit affine counter instead of silently weakening64/256.
const _: () = assert!(super::super::resources::GROUP_BYTES / RECORD_BYTES <= 64);
const _: () = assert!(super::super::resources::PROCESS_BYTES / RECORD_BYTES <= 256);
pub(super) const LIFETIME_MS: u64 = 15_000;
pub(super) const GAP_MS: u64 = 500;

pub(crate) enum Binding {
    Endpoint(super::super::presence::FreshPeer),
    // Captured from the actual direct connected route, never a roster descriptor.
    DirectHelper(Arc<dyn RequestAuthority>),
}
pub(super) struct Query {
    pub(super) original: super::super::call_activation::ActivationObserver,
    pub(super) local_key: JoinKey,
    pub(super) binding: Binding,
    pub(super) ordinal: u64,
}
pub(super) enum Role {
    Query(Query),
    Response { request: Object },
    Forward { permit: wire::Permit },
}
pub(super) struct State {
    pub(super) attempts: u8,
    pub(super) next: u64,
    pub(super) in_flight: bool,
    pub(super) settled: bool,
    pub(super) support: Option<Object>,
    pub(super) published: bool,
    pub(super) helper_cursor: Option<DeviceId>,
}
pub(super) struct Record {
    pub(super) object: Object,
    pub(super) role: Role,
    pub(super) room: RoomAuthority,
    pub(super) epoch: EpochObservation,
    pub(super) effect: Arc<EffectAuthority>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) deadline: u64,
    pub(super) live: CallLease,
    pub(super) sending: CallLease,
    pub(super) state: Mutex<State>,
    // Driver currency never locks mutable cache state. Bit15 is internal known
    // observation (wire bits remain0..2); expiry is published before that flag.
    observed: AtomicU16,
    observed_expires: AtomicU64,
}
/// The outside SAME payer survives body/Arc destruction. Raw Record Arcs and
/// Weaks never escape this module. A late driver authority retains this wrapper.
#[derive(Clone)]
pub(crate) struct Owner(Retained<Record>);
impl Owner {
    pub(super) fn inventory_bytes() -> usize {
        // Full value/Arc publication overlap, canonical Control7 and original
        // Query/Support cache bytes, both lease/watch allocations and all wrapper
        // headers. Keep this in production before payment/allocation, not a test
        // assertion that could silently stop protecting a new platform layout.
        let watch = 64
            + 4 * std::mem::size_of::<usize>()
            + 9 * std::mem::size_of::<tokio::sync::Notify>()
            + 64;
        2 * std::mem::size_of::<Record>()
            + 2048
            + 2 * 512
            + 2 * (std::mem::size_of::<super::super::LeaseState>() + watch)
            + 8 * std::mem::size_of::<usize>()
            + 256
    }
    pub(super) fn reserve(resources: &Arc<Resources>, group: Nonce) -> Result<Payment, VoiceError> {
        if Self::inventory_bytes() > RECORD_BYTES {
            return Err(VoiceError::Bounds);
        }
        Payment::reserve(resources, group, RECORD_BYTES, ResourceKind::Retained)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        payment: Payment,
        object: Object,
        role: Role,
        room: RoomAuthority,
        epoch: EpochObservation,
        effect: Arc<EffectAuthority>,
        clock: Arc<dyn Clock>,
        deadline: u64,
    ) -> Self {
        Self(payment.retain(Record {
            object,
            role,
            room,
            epoch,
            effect,
            clock,
            deadline,
            live: CallLease::default(),
            sending: CallLease::default(),
            state: Mutex::new(State {
                attempts: 0,
                next: 0,
                in_flight: false,
                settled: false,
                support: None,
                published: false,
                helper_cursor: None,
            }),
            observed: AtomicU16::new(0),
            observed_expires: AtomicU64::new(0),
        }))
    }
    pub(super) fn get(&self) -> &Record {
        self.0.get()
    }
    pub(crate) fn object(&self) -> &Object {
        &self.get().object
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0)
    }
    pub(crate) fn samples(&self) -> (u64, u64) {
        (self.get().clock.now_ms(), self.get().clock.monotonic_ms())
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        let r = self.get();
        let m = r.object.metadata();
        if !r.live.is_live()
            || !r.room.lease.is_live()
            || !r.epoch.is_live()
            || !r.effect.authorized_at(wall, mono)
            || mono >= r.deadline
            || m.fresh(wall).is_err()
        {
            return false;
        }
        match &r.role {
            Role::Query(q) => {
                if !q.original.is_active() {
                    return false;
                }
                match &q.binding {
                    Binding::DirectHelper(route) => route.authorized(),
                    Binding::Endpoint(peer) => peer
                        .authorize_at(
                            &Context {
                                scope: m.scope(),
                                local: m.issuer(),
                                epoch: m.epoch(),
                                now: wall,
                                indirect: m.scope().policy.is_some(),
                            },
                            wall,
                            mono,
                        )
                        .is_ok(),
                }
            }
            Role::Response { request } => request.metadata().fresh(wall).is_ok(),
            Role::Forward { permit } => wall <= permit.expires().saturating_add(CLOCK_SKEW_MS),
        }
    }
    pub(crate) fn authorized(&self) -> bool {
        let (wall, mono) = self.samples();
        self.authorized_at(wall, mono)
    }
    /// Negative state is installed while committing a replacement; arbitrary
    /// watch wakers run only after every native/table guard has been released.
    pub(super) fn invalidate(&self) {
        self.get().live.invalidate();
        self.get().sending.invalidate();
    }
    pub(crate) fn notify(&self) {
        self.get().live.notify_revoked();
        self.get().sending.notify_revoked();
    }
    pub(crate) fn matches_original(
        &self,
        original: &super::super::call_activation::ActivationObserver,
        key: &JoinKey,
    ) -> bool {
        matches!(&self.get().role, Role::Query(query) if query.original.is_same(original) && &query.local_key == key)
    }
    pub(crate) fn features_at(&self, wall: u64, mono: u64) -> Option<u16> {
        if !self.authorized_at(wall, mono) {
            return None;
        }
        let observed = self.get().observed.load(Ordering::Acquire);
        if observed & 0x8000 == 0
            || wall
                > self
                    .get()
                    .observed_expires
                    .load(Ordering::Acquire)
                    .saturating_add(CLOCK_SKEW_MS)
        {
            None
        } else {
            Some(observed & 7)
        }
    }
    pub(crate) fn route_cursor(&self) -> (u8, Option<DeviceId>) {
        let state = self.get().state.lock().unwrap_or_else(|e| e.into_inner());
        (state.attempts, state.helper_cursor)
    }
    pub(crate) fn accept_at(
        &self,
        support: &mut Option<Object>,
        wall: u64,
        mono: u64,
        publish: impl FnOnce(&mut dyn FnMut()) -> Result<(), VoiceError>,
    ) -> Result<(), VoiceError> {
        let query = self.object().metadata();
        let prepared = support.as_ref().ok_or(VoiceError::Malformed)?;
        let m = prepared.metadata();
        if !matches!(self.get().role, Role::Query(_))
            || !self.authorized_at(wall, mono)
            || m.scope() != query.scope()
            || m.epoch() != query.epoch()
            || m.issuer() != query.target()
            || m.target() != query.issuer()
            || m.expires() > query.expires()
            || !matches!(m.operation(), ProtocolOperation::Support { query: id, .. } if id == query.id())
        {
            return Err(VoiceError::Unauthorized);
        }
        m.fresh(wall)?;
        let mut state = self.get().state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = &state.support {
            return if old.hash() == prepared.hash() {
                Ok(())
            } else {
                Err(VoiceError::RetryMismatch)
            };
        }
        // Do not wake the lower from beneath this state/native fence.
        publish(&mut || {
            let value = support.as_ref().expect("checked original Support");
            let ProtocolOperation::Support { features, .. } = value.metadata().operation() else {
                unreachable!("checked Support");
            };
            self.get()
                .observed_expires
                .store(value.metadata().expires(), Ordering::Release);
            self.get()
                .observed
                .store(0x8000 | features, Ordering::Release);
            state.support = support.take();
            self.get().sending.invalidate();
        })
    }
    pub(crate) async fn revoked(&self) {
        if !self.authorized() {
            return;
        }
        let r = self.get();
        let mut waits = vec![
            r.live.wait_revoked().boxed(),
            r.room.lease.wait_revoked().boxed(),
            r.effect.revoked().boxed(),
            r.clock.sleep(std::time::Duration::from_millis(
                r.deadline.saturating_sub(r.clock.monotonic_ms()),
            )),
        ];
        if let Role::Query(q) = &r.role {
            waits.push(q.original.original_lease().wait_revoked().boxed());
            if let Binding::DirectHelper(route) = &q.binding {
                waits.push(route.revoked().boxed());
            }
        }
        futures::future::select_all(waits).await;
    }
}
/// ACK custody only. Possession of this opaque receipt cannot create a query or
/// turn HelperQueued into a remote support/Availability observation.
#[derive(Clone)]
pub(crate) struct Receipt(pub(super) Owner);
impl Receipt {
    pub(crate) fn new(owner: Owner) -> Self {
        Self(owner)
    }
    pub(crate) fn authorized_at(&self, wall: u64, mono: u64) -> bool {
        self.0.authorized_at(wall, mono)
    }
    pub(crate) async fn revoked(&self) {
        self.0.revoked().await
    }
}
redacted_debug!(Owner, Receipt);
