//! Paid strong negative text-work exclusions, separate from genesis group claims.
//!
//! A lane claim grants NO Current, selected-O, signing, work or retirement permission.
//! It only excludes another same-host/group/signer worker. The registry strongly retains
//! every admitted entry after caller cancellation/Drop. Removal requires the concrete
//! physical WC/quiescent-continuation path, or the distinct fully drained successful empty
//! promotion path. Weak expiry is never a removal predicate.
use super::*;
mod create;
pub(in crate::store) use create::{
    RetiredV2TextCreateLane, V2_TEXT_CREATE_RETIREMENT_LAYOUT_BYTES,
};
mod promotion;
pub(in crate::store) use create::{
    OriginalV2TextCreateLaneObservation, PreparedV2TextCreateLane, V2TextCreateBinding,
    V2TextCreateControlWriteEnd, V2TextCreateLane, V2TextCreateWorkspaceEnd,
    V2_TEXT_CREATE_LANE_BYTES,
};
mod query;
use crate::store::critical_namespaces::read_pool::CatalogReadReservation;
use crate::store::owner_evidence::canonical::current_vacancy::text::V2TextOwnerNamespace;
use crate::store::persistence::{V2TextOwnerRef, V2TextSourceNegativeObserver};
use catcoms_sync::owner_v2::BootstrapSourceId;
pub(in crate::store) use promotion::{
    RetiredV2TextPromotionLane, V2TextPromotionLaneDrain, V2TextPromotionLaneDrained,
    V2TextPromotionWorkEnd,
};
#[cfg(all(test, windows))]
pub(in crate::store) use query::text_query_control_io_for_test;
#[cfg(all(test, windows))]
pub(in crate::store) use query::text_query_registry_counts_for_test;
pub(in crate::store) use query::V2TextQueryControlIoEnd;
pub(in crate::store) use query::{
    PreparedV2TextAcceptedQueryLease, V2TextAcceptedQueryLease,
    V2_TEXT_ACCEPTED_QUERY_READER_LAYOUT_BYTES,
};
pub(in crate::store) use query::{
    PreparedV2TextQueryLease, V2TextQueryLease, V2_TEXT_QUERY_READER_LAYOUT_BYTES,
    V2_TEXT_QUERY_REGISTRY_BACKING_BYTES,
};
pub(in crate::store) use query::{
    PreparedV2TextRetirementRootsClose, RemovedV2TextRetirementRootGenerations,
    V2TextRetirementRootsClosed, V2TextRetirementRootsDrained,
    V2_TEXT_RETIREMENT_ROOTS_LAYOUT_BYTES,
};
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_ACTIVE_TEXT_LANES: usize = 16;
const MAX_GROUP_BYTES: usize = 256;

/// Owned only by this Store's SAME canonical Runtime. The backing is paid/constructed
/// outside writers and transferred on the first negative admission, never implicitly grown.
#[derive(Default)]
pub(super) struct TextLaneRuntime {
    buffer: Option<LaneBuffer>,
    queries: Option<query::QueryGenerationBuffer>,
}
struct LaneBuffer {
    slots: Box<[Option<Arc<LaneBody>>; MAX_ACTIVE_TEXT_LANES]>,
    // LAST: array and all strong inert entries drain before the backing payment.
    _fee: CatalogReadReservation,
}
struct LaneKey {
    group: [u8; MAX_GROUP_BYTES],
    group_len: usize,
    signer: [u8; 32],
}
impl LaneKey {
    fn group(&self) -> &[u8] {
        &self.group[..self.group_len]
    }
    fn same(&self, other: &Self) -> bool {
        self.group() == other.group() && self.signer == other.signer
    }
}
struct LaneBody {
    identity: Arc<()>,
    cancelled: Arc<AtomicBool>,
    key: LaneKey,
    source_id: BootstrapSourceId,
    source: V2TextSourceNegativeObserver,
    promotion: Arc<promotion::PromotionWorkControl>,
    // Only actual later finite stage/commit ownership sets this under canonical.
    control_write_active: AtomicBool,
    // LAST, also held by any tiny escaping observation after the original slot retires.
    _fee: Arc<CatalogReadReservation>,
}

/// Unique original caller. Dropping this value only closes new original work: the strong
/// registry debt remains. There is no Clone or raw identity/slot constructor.
pub(in crate::store) struct V2TextLane {
    body: Arc<LaneBody>,
}
pub(in crate::store) struct PreparedV2TextLane {
    original: Option<V2TextLane>,
    original_observation: OriginalV2TextLaneObservation,
    expansion: Option<LaneBuffer>,
    // Resource disposition ONLY. Never read by a Current/work/retirement proof.
    expansion_redundant: bool,
}

/// Negative exact-original identity only. No sender, graph, Store, session or running work.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextLaneObservation {
    identity: Weak<()>,
    cancelled: Arc<AtomicBool>,
    source: V2TextSourceNegativeObserver,
    _fee: Arc<CatalogReadReservation>,
}

pub(in crate::store) const V2_TEXT_LANE_ENTRY_BYTES: usize = std::mem::size_of::<LaneBody>()
    + std::mem::size_of::<PreparedV2TextLane>()
    + std::mem::size_of::<V2TextLane>()
    + std::mem::size_of::<OriginalV2TextLaneObservation>()
    + promotion::PROMOTION_WORK_CONTROL_METADATA_BYTES
    + 5 * 64;
pub(in crate::store) const V2_TEXT_LANE_BUFFER_BYTES: usize = std::mem::size_of::<LaneBuffer>()
    + MAX_ACTIVE_TEXT_LANES * std::mem::size_of::<Option<Arc<LaneBody>>>()
    + 64;

impl PreparedV2TextLane {
    /// Actual independent entry/backing fees precede every new allocation. Source observer
    /// has its OWN fee; retaining that exact observer never borrows SDK/source-work credit.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        source: &V2TextSourceNegativeObserver,
    ) -> Result<Self, AppError> {
        owner.require_closed_family()?;
        owner.require_original_negative(store, &owner.observe_writer())?;
        if !source.is_live() {
            return Err(refused());
        }
        let data = owner.source().data();
        let context = data.context.spec_data();
        if context.group.is_empty() || context.group.len() > MAX_GROUP_BYTES {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let entry_fee = pool.try_reserve(V2_TEXT_LANE_ENTRY_BYTES)?;
        let buffer_fee = pool.try_reserve(V2_TEXT_LANE_BUFFER_BYTES)?;
        entry_fee.validate(pool, V2_TEXT_LANE_ENTRY_BYTES)?;
        buffer_fee.validate(pool, V2_TEXT_LANE_BUFFER_BYTES)?;
        owner.require_original_negative(store, &owner.observe_writer())?;
        if !source.is_live() {
            return Err(refused());
        }
        let mut group = [0; MAX_GROUP_BYTES];
        group[..context.group.len()].copy_from_slice(context.group);
        let entry_fee = Arc::new(entry_fee);
        let body = Arc::new(LaneBody {
            identity: Arc::new(()),
            cancelled: Arc::new(AtomicBool::new(false)),
            key: LaneKey {
                group,
                group_len: context.group.len(),
                signer: context.signer,
            },
            source_id: data.source,
            source: source.clone(),
            promotion: Arc::new(promotion::PromotionWorkControl::new(entry_fee.clone())),
            control_write_active: AtomicBool::new(false),
            _fee: entry_fee,
        });
        let original = V2TextLane { body };
        let original_observation = original.observe_original();
        Ok(Self {
            original: Some(original),
            original_observation,
            expansion: Some(LaneBuffer {
                slots: Box::new(std::array::from_fn(|_| None)),
                _fee: buffer_fee,
            }),
            expansion_redundant: false,
        })
    }

    /// Only caller-owned unused backing drains here, OUTSIDE every global writer. Keep it
    /// across first-admission failures; otherwise a same-instance retry could lack backing.
    pub(in crate::store) fn drain_unused_outside_writers(&mut self) {
        if self.expansion_redundant {
            drop(self.expansion.take());
        }
    }
}

impl LaneBody {
    fn require_admission(&self, namespace: &V2TextOwnerNamespace<'_, '_>) -> Result<(), AppError> {
        namespace.validate_source_only()?;
        let admission = namespace.scope_admission();
        self.source.require_same_original(admission)?;
        if self.cancelled.load(Ordering::Acquire)
            || self.key.group() != admission.group()
            || &self.key.signer != admission.signer()
            || self.source_id != admission.source().data().source
        {
            return Err(refused());
        }
        namespace.require_installed_text_writer(admission.writer_observation()?)
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Register only negative exclusion under the actual same AD/pins/canonical closure.
    /// No O/H proof or permission follows from this slot; effect admissions still require
    /// the real physical selected-source/Ready/commit receipts. Full signed group history
    /// and persistent pendingTxn are NOT stored or bounded by this sixteen-entry registry.
    pub(in crate::store) fn claim_text_lane_into(
        &mut self,
        prepared: &mut PreparedV2TextLane,
        retained: &mut Option<V2TextLane>,
    ) -> Result<(), AppError> {
        // Cache the unique result in the OUTSIDE caller's actual phase before its Scope
        // wrapper can run a fallible final negative. No returned owning lane can be lost.
        if let Some(lane) = retained.as_ref() {
            prepared.original_observation.require_same_original(lane)?;
            return lane.require_registered(self);
        }
        let original = prepared.original.as_ref().ok_or_else(refused)?;
        original.body.require_admission(self)?;
        let pool = &self
            .scope_admission()
            .store()
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        original
            .body
            ._fee
            .validate(pool, V2_TEXT_LANE_ENTRY_BYTES)?;
        if let Some(expansion) = &prepared.expansion {
            expansion._fee.validate(pool, V2_TEXT_LANE_BUFFER_BYTES)?;
        }
        original.body.require_admission(self)?;
        let runtime = self.text_lane_runtime_mut();
        // Any existing same-key strong inert slot excludes us, even cancelled/dead caller.
        // A public parent change or cleared pendingTxn at B cannot evade physical drain.
        let index = if let Some(buffer) = &runtime.buffer {
            if buffer
                .slots
                .iter()
                .flatten()
                .any(|slot| slot.key.same(&original.body.key))
            {
                return Err(refused());
            }
            buffer
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or_else(refused)?
        } else {
            if prepared.expansion.is_none() {
                return Err(refused());
            }
            0
        };
        // Every fallible original/capacity/account check precedes transfer. No allocation,
        // validation, refund or callback intervenes between slot install and owning return.
        if runtime.buffer.is_none() {
            runtime.buffer = prepared.expansion.take();
        }
        runtime
            .buffer
            .as_mut()
            .expect("backing transferred before entry")
            .slots[index] = Some(original.body.clone());
        prepared.expansion_redundant = true;
        *retained = prepared.original.take();
        Ok(())
    }
}

impl V2TextLane {
    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextLaneObservation {
        OriginalV2TextLaneObservation {
            identity: Arc::downgrade(&self.body.identity),
            cancelled: self.body.cancelled.clone(),
            source: self.body.source.clone(),
            _fee: self.body._fee.clone(),
        }
    }
    pub(in crate::store) fn require_registered(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
    ) -> Result<(), AppError> {
        self.body.require_admission(namespace)?;
        let runtime = namespace.text_lane_runtime_mut();
        if !runtime
            .buffer
            .as_ref()
            .ok_or_else(refused)?
            .slots
            .iter()
            .flatten()
            .any(|slot| Arc::ptr_eq(slot, &self.body))
        {
            return Err(refused());
        }
        self.body.require_admission(namespace)
    }
}
impl Drop for V2TextLane {
    fn drop(&mut self) {
        self.body.cancelled.store(true, Ordering::Release);
    }
}
impl OriginalV2TextLaneObservation {
    pub(in crate::store) fn is_cancelled(&self) -> bool {
        self.identity.strong_count() == 0
            || self.cancelled.load(Ordering::Acquire)
            || !self.source.is_live()
    }
    pub(in crate::store) fn require_same_original(
        &self,
        lane: &V2TextLane,
    ) -> Result<(), AppError> {
        if self.is_cancelled() || !self.identity.ptr_eq(&Arc::downgrade(&lane.body.identity)) {
            return Err(refused());
        }
        Ok(())
    }
}

macro_rules! redacted_debug {
    ($($ty:ident),+ $(,)?) => {$ (
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_debug!(
    PreparedV2TextLane,
    V2TextLane,
    OriginalV2TextLaneObservation
);

/// Negative test observation only; no slot/Runtime/permission escapes this bounded count.
#[cfg(all(test, any(windows, target_os = "linux")))]
pub(in crate::store) fn active_text_lane_count_for_test(store: &ServerStore) -> usize {
    let canonical = store.owner_evidence.canonical.lock().unwrap();
    canonical
        .text_lanes
        .buffer
        .as_ref()
        .map_or(0, |buffer| buffer.slots.iter().flatten().count())
}
