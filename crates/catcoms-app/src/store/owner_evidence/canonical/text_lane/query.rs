//! Bounded, strong private-root generation reader ownership for actual text queries.
//!
//! Only the paired physical query can name a generation. A codec row/Empty result,
//! caller-selected page ID or weak expiry cannot install or retire a generation. A full
//! registry can forget an exact metadata pin only after its REAL affine readers drained;
//! that edge deletes NO page and grants no WD, workspace-clear or Create-miss permission.
use super::*;
mod accepted;
mod io;
mod origin;
mod retirement;
use crate::store::persistence::v2_text::{
    OriginalV2TextTokenQueryObservation, PreparedV2TextTokenQuery, V2TextTokenQueryReaderClaim,
    V2TextTokenQueryResources,
};
pub(in crate::store) use accepted::{
    PreparedV2TextAcceptedQueryLease, V2TextAcceptedQueryLease,
    V2_TEXT_ACCEPTED_QUERY_READER_LAYOUT_BYTES,
};
#[cfg(all(test, windows))]
pub(in crate::store) use io::text_query_control_io_for_test;
pub(in crate::store) use io::V2TextQueryControlIoEnd;
use origin::{QueryOrigin, QueryReaderClaimAlias, QueryResourcesOrigin};
pub(in crate::store) use retirement::{
    PreparedV2TextRetirementRootsClose, RemovedV2TextRetirementRootGenerations,
    V2TextRetirementRootsClosed, V2TextRetirementRootsDrained,
    V2_TEXT_RETIREMENT_ROOTS_LAYOUT_BYTES,
};
use std::sync::atomic::AtomicUsize;
use std::sync::{Condvar, Mutex};

const MAX_QUERY_GENERATIONS: usize = 32;
const MAX_QUERY_READERS: usize = 64;
const GENERATION_CLOSING: usize = 1 << (usize::BITS - 1);
const READER_MASK: usize = !GENERATION_CLOSING;

pub(super) struct QueryGenerationBuffer {
    slots: Box<[Option<Arc<QueryGeneration>>; MAX_QUERY_GENERATIONS]>,
    // LAST: the complete fixed array and all strong entries precede backing refund.
    fee: CatalogReadReservation,
}

struct QueryGeneration {
    key: LaneKey,
    source: BootstrapSourceId,
    parent: QueryOrigin,
    // Only real reader admission increments; only its affine end decrements. The high
    // bit permanently closes an exact drained metadata pin before slot reuse. It does
    // not prove private-page retirement, which needs its later genuine basis consumer.
    state: AtomicUsize,
    // Source/key/OS cleanup ends separately from logical completed-query readers.
    control_io: AtomicUsize,
    wait: Mutex<()>,
    changed: Condvar,
    // LAST: generation parent frames and control tails retain their OWN query payment.
    _resources: QueryResourcesOrigin,
}

/// A preallocated actual-generation candidate, fixed backing and one unspent reader.
/// Every fallible admission preserves this SAME owner and the outside result slot.
pub(in crate::store) struct PreparedV2TextQueryLease {
    candidate: Option<Arc<QueryGeneration>>,
    original: QueryOrigin,
    reader_original: Option<QueryOrigin>,
    expansion: Option<QueryGenerationBuffer>,
    expansion_redundant: bool,
    candidate_redundant: bool,
    // At most one exact old metadata pin removed by this admission; drop it OUTSIDE
    // writers. This is not an old private-page/root retirement receipt.
    displaced: Option<Arc<QueryGeneration>>,
    // LAST, including when candidate/backing have moved into the strong Runtime.
    resources: QueryResourcesOrigin,
    // LAST: the claim cannot re-open another same-query preparation while this wrapper
    // or its SAME actual result lease still owns paid metadata. Exactly two aliases.
    claim: QueryReaderClaimAlias,
}

/// One actual negative capture/root pin, never query/result authority by itself. This
/// owner is non-Clone and has no adopting constructor. The active outside capture acquires
/// it BEFORE retained Files/key can open, after a fresh exact selected-parent proof. A
/// finite admitted IO marker and its result retain the SAME owner without a drain gap.
/// No network sender/ACK may retain this physical pin.
pub(in crate::store) struct V2TextQueryLease {
    original: QueryOrigin,
    generation: Arc<QueryGeneration>,
    _resources: QueryResourcesOrigin,
    // LAST: end follows every physical child and every reader/query wrapper tail.
    _end: QueryReaderEnd,
    // LAST: release same-query preparation exclusion only AFTER its actual reader end.
    _claim: QueryReaderClaimAlias,
}

struct QueryReaderEnd {
    generation: Arc<QueryGeneration>,
}
impl Drop for QueryReaderEnd {
    fn drop(&mut self) {
        // This runs only outside all host writers. The tiny wait lock synchronizes the
        // future real closed-generation join with its final actual end, never user IO.
        let _wait = self
            .generation
            .wait
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = self.generation.state.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous & READER_MASK > 0, "one affine end per real reader");
        self.generation.changed.notify_all();
    }
}

pub(in crate::store) const V2_TEXT_QUERY_REGISTRY_BACKING_BYTES: usize =
    std::mem::size_of::<QueryGenerationBuffer>()
        + MAX_QUERY_GENERATIONS * std::mem::size_of::<Option<Arc<QueryGeneration>>>()
        + 64;

/// The physical query's NEW1MiB simultaneous invoice includes these per-query/candidate
/// wrappers. The fixed shared backing has a separate actual debit above; SDK/read/work
/// fees cannot pay either. Each retained generation and reader keeps its original fee.
pub(in crate::store) const V2_TEXT_QUERY_READER_LAYOUT_BYTES: usize =
    std::mem::size_of::<QueryGeneration>()
        + std::mem::size_of::<PreparedV2TextQueryLease>()
        + std::mem::size_of::<V2TextQueryLease>()
        + std::mem::size_of::<QueryReaderEnd>()
        + std::mem::size_of::<V2TextQueryControlIoEnd>()
        + std::mem::size_of::<V2TextTokenQueryReaderClaim>()
        + 6 * 64;

impl PreparedV2TextQueryLease {
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &crate::store::persistence::GenesisSourceDataRef<'_>,
        query: &PreparedV2TextTokenQuery,
    ) -> Result<Self, AppError> {
        query.validate_preparation(store, source)?;
        // This real per-query claim precedes candidate/backing/control allocation. Its
        // last private alias covers BOTH this preparer and the final affine reader.
        let claim = query.claim_reader_preparation()?;
        let context = source.data().context.spec_data();
        if context.group.is_empty() || context.group.len() > MAX_GROUP_BYTES {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_TEXT_QUERY_REGISTRY_BACKING_BYTES)?;
        fee.validate(pool, V2_TEXT_QUERY_REGISTRY_BACKING_BYTES)?;
        query.query_resources().validate_prepaid_account()?;
        query.validate_preparation(store, source)?;
        let mut group = [0; MAX_GROUP_BYTES];
        group[..context.group.len()].copy_from_slice(context.group);
        let original = QueryOrigin::Token(query.observe_original());
        let resources = QueryResourcesOrigin::Token(query.query_resources().clone());
        let claim = QueryReaderClaimAlias::Token(Arc::new(claim));
        let candidate = Arc::new(QueryGeneration {
            key: LaneKey {
                group,
                group_len: context.group.len(),
                signer: context.signer,
            },
            source: source.data().source,
            parent: original.clone(),
            state: AtomicUsize::new(0),
            control_io: AtomicUsize::new(0),
            wait: Mutex::new(()),
            changed: Condvar::new(),
            _resources: resources.clone(),
        });
        Ok(Self {
            candidate: Some(candidate),
            original: original.clone(),
            reader_original: Some(original),
            expansion: Some(QueryGenerationBuffer {
                slots: Box::new(std::array::from_fn(|_| None)),
                fee,
            }),
            expansion_redundant: false,
            candidate_redundant: false,
            displaced: None,
            resources,
            claim,
        })
    }

    /// Account waits precede physical Source/selected proofs in the actual guard.
    pub(in crate::store) fn validate_prepaid_account(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        self.resources.validate_prepaid_account()?;
        if let Some(expansion) = &self.expansion {
            let pool = &store
                .archive_bootstrap_policy
                .as_ref()
                .ok_or_else(refused)?
                .pending
                .pool;
            expansion
                .fee
                .validate(pool, V2_TEXT_QUERY_REGISTRY_BACKING_BYTES)?;
        }
        Ok(())
    }

    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.original.require_token_original(original)
    }

    /// Free only known unused caller backing/candidate, outside all writers. Neither a
    /// zero reader count nor weak expiry permits THIS cleanup method to remove an
    /// installed generation; the guarded exact metadata-reuse edge is distinct.
    pub(in crate::store) fn drain_unused_outside_writers(&mut self) {
        if self.expansion_redundant {
            drop(self.expansion.take());
        }
        if self.candidate_redundant {
            drop(self.candidate.take());
        }
        drop(self.displaced.take());
    }
}

impl QueryGeneration {
    fn require_source(&self, namespace: &V2TextOwnerNamespace<'_, '_>) -> Result<(), AppError> {
        namespace.validate_source_only()?;
        let admission = namespace.scope_admission();
        if self.key.group() != admission.group()
            || &self.key.signer != admission.signer()
            || self.source != admission.source().data().source
        {
            return Err(refused());
        }
        Ok(())
    }

    fn reserve_reader(&self) -> Result<(), AppError> {
        let mut current = self.state.load(Ordering::Acquire);
        // Only the held canonical writer can increment. Concurrent real outside ends can
        // lower at most64 times; no unbounded contention loop or untrusted retry input.
        for _ in 0..=MAX_QUERY_READERS {
            if current & GENERATION_CLOSING != 0 || current & READER_MASK >= MAX_QUERY_READERS {
                return Err(refused());
            }
            match self.state.compare_exchange(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(next) => current = next,
            }
        }
        Err(refused())
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// The paired actual guard has just verified the SAME selected parent using bounded
    /// ephemeral Files that have already closed. Cache a NEGATIVE pin outside BEFORE any
    /// retained Source File/key opens. Only the later complete captured Source proof can
    /// construct an Admitted lookup. On failed capture this pin still covers actual cleanup.
    pub(in crate::store) fn pin_text_query_capture_into(
        &mut self,
        prepared: &mut PreparedV2TextQueryLease,
        retained: &mut Option<V2TextQueryLease>,
        control_io: &mut Option<V2TextQueryControlIoEnd>,
    ) -> Result<(), AppError> {
        prepared.original.require_token_kind()?;
        self.pin_query_origin_capture_into(prepared, retained, control_io)
    }

    // PRIVATE shared counter edge. Only the two sealed typed producer wrappers can
    // construct its origin/resource/claim variants; it never accepts a caller root.
    fn pin_query_origin_capture_into(
        &mut self,
        prepared: &mut PreparedV2TextQueryLease,
        retained: &mut Option<V2TextQueryLease>,
        control_io: &mut Option<V2TextQueryControlIoEnd>,
    ) -> Result<(), AppError> {
        self.require_text_control_write_idle()?;
        if let Some(existing) = retained.as_ref() {
            control_io
                .as_ref()
                .ok_or_else(refused)?
                .require_same_reader(existing)?;
            return existing.require_origin_under_namespace(self, &prepared.original);
        }
        if control_io.is_some() {
            return Err(refused());
        }
        let candidate = prepared.candidate.as_ref().ok_or_else(refused)?;
        candidate.require_source(self)?;
        candidate
            .parent
            .require_same_selected_parent(&prepared.original)?;
        prepared.reader_original.as_ref().ok_or_else(refused)?;
        let runtime = self.text_lane_runtime_mut();
        let existing = runtime.queries.as_ref().and_then(|buffer| {
            buffer.slots.iter().enumerate().find_map(|(index, slot)| {
                slot.as_ref()
                    .filter(|entry| {
                        entry.key.same(&candidate.key)
                            && entry.source == candidate.source
                            && entry
                                .parent
                                .require_same_selected_parent(&prepared.original)
                                .is_ok()
                    })
                    .map(|_| index)
            })
        });
        let mut replace_drained = false;
        let index = match (existing, runtime.queries.as_ref()) {
            (Some(index), _) => index,
            (None, Some(buffer)) => match buffer.slots.iter().position(Option::is_none) {
                Some(index) => index,
                None if prepared.displaced.is_none() => {
                    let index = buffer
                        .slots
                        .iter()
                        .position(|slot| {
                            slot.as_ref()
                                .is_some_and(|entry| entry.state.load(Ordering::Acquire) == 0)
                        })
                        .ok_or_else(refused)?;
                    replace_drained = true;
                    index
                }
                None => return Err(refused()),
            },
            (None, None) if prepared.expansion.is_some() => 0,
            _ => return Err(refused()),
        };
        let generation = match existing {
            Some(_) => runtime.queries.as_ref().unwrap().slots[index]
                .as_ref()
                .unwrap()
                .clone(),
            None => candidate.clone(),
        };
        // All account/FS/codec and capacity checks have completed. After this actual
        // reservation only infallible paid Arc clones and owning transfers may follow.
        generation.reserve_reader()?;
        let io_end = generation.reserve_control_io_end();
        if runtime.queries.is_none() {
            runtime.queries = prepared.expansion.take();
        };
        if existing.is_none() {
            if replace_drained {
                let slot = &mut runtime.queries.as_mut().unwrap().slots[index];
                let old = slot.as_ref().expect("same actual drained slot checked");
                // Only this SAME canonical writer can increment; actual End only lowers.
                // The zero is therefore still exact, with all physical children drained.
                // Closing is permanent BEFORE removal. Old paid negative observations
                // cannot adopt the successor Arc even if it occupies the same array slot.
                old.state
                    .compare_exchange(0, GENERATION_CLOSING, Ordering::AcqRel, Ordering::Acquire)
                    .expect("same canonical writer, no reader can increment concurrently");
                prepared.displaced = slot.take();
            }
            runtime.queries.as_mut().unwrap().slots[index] = prepared.candidate.take();
        } else {
            prepared.candidate_redundant = true;
        }
        prepared.expansion_redundant = prepared.expansion.is_some();
        *control_io = Some(io_end);
        *retained = Some(V2TextQueryLease {
            original: prepared
                .reader_original
                .take()
                .expect("unspent original checked"),
            generation: generation.clone(),
            _resources: prepared.resources.clone(),
            _end: QueryReaderEnd { generation },
            _claim: prepared.claim.clone(),
        });
        Ok(())
    }
}

impl V2TextQueryLease {
    pub(in crate::store) fn require_same_query(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.original.require_token_original(original)
    }

    /// Cached negatives ONLY. This is not a fresh selected-O/root, registry or Current
    /// proof. A completed query must use the actual guard after its full selected proof.
    pub(in crate::store) fn require_live_query(
        &self,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.original.require_token_original(original)?;
        self.require_live_origin(&self.original)
    }

    fn require_live_origin(&self, original: &QueryOrigin) -> Result<(), AppError> {
        self.original.require_same_original(original)?;
        self.generation
            .parent
            .require_same_selected_parent(original)?;
        let state = self.generation.state.load(Ordering::Acquire);
        if state & GENERATION_CLOSING != 0 || state & READER_MASK == 0 {
            return Err(refused());
        }
        Ok(())
    }

    pub(in crate::store) fn require_under_namespace(
        &self,
        namespace: &V2TextOwnerNamespace<'_, '_>,
        original: &OriginalV2TextTokenQueryObservation,
    ) -> Result<(), AppError> {
        self.original.require_token_original(original)?;
        self.require_origin_under_namespace(namespace, &self.original)
    }

    fn require_origin_under_namespace(
        &self,
        namespace: &V2TextOwnerNamespace<'_, '_>,
        original: &QueryOrigin,
    ) -> Result<(), AppError> {
        self.require_live_origin(original)?;
        self.generation.require_source(namespace)?;
        let present = namespace
            .text_lane_runtime()
            .queries
            .as_ref()
            .is_some_and(|buffer| {
                buffer
                    .slots
                    .iter()
                    .flatten()
                    .any(|entry| Arc::ptr_eq(entry, &self.generation))
            });
        if !present {
            return Err(refused());
        }
        // No new wait follows complete physical selection: only native slot/source
        // negatives. The resource-only tail keeps the original query fee alive.
        namespace.validate_source_only()?;
        self.require_live_origin(original)
    }
}

macro_rules! redacted_query_debug {
    ($($ty:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_query_debug!(PreparedV2TextQueryLease, V2TextQueryLease);

/// Negative bounded test observation only. Neither this sum nor any caller integer is
/// accepted by admission, cleanup or retirement. Counts come from actual affine ends.
#[cfg(all(test, windows))]
pub(in crate::store) fn text_query_registry_counts_for_test(store: &ServerStore) -> (usize, usize) {
    let canonical = store.owner_evidence.canonical.lock().unwrap();
    canonical
        .text_lanes
        .queries
        .as_ref()
        .map_or((0, 0), |buffer| {
            buffer
                .slots
                .iter()
                .flatten()
                .fold((0, 0), |(slots, readers), entry| {
                    (
                        slots + 1,
                        readers + (entry.state.load(Ordering::Acquire) & READER_MASK),
                    )
                })
        })
}
