//! Actual old-generation close/drain/detachment for the paired B retirement path.
//!
//! Only a genuine prepared B supplies its four original O roots. The Current layer
//! proves actual B/WB/advanced-final before calling the canonical mutation below.
//! Closing is monotone; only actual affine reader/IO Ends establish drain. Neither a
//! missing registry entry nor IO-zero alone constructs a deletion/completion permit.
use super::*;
mod finish;
use crate::store::persistence::v2_text::{
    OriginalV2TextAcceptedCreateObservation, PreparedV2TextAcceptedCreate, TextHeadIdData,
    TextRetirementPlanIdData, V2TextRetirementRootsClaim,
};
use crate::store::persistence::{GenesisSourceDataRef, V2AdvancedAppendBasis};

type Generations = [Option<Arc<QueryGeneration>>; MAX_QUERY_GENERATIONS];

struct RootsState {
    snapshot: Box<Generations>,
    // Preallocated outside writers; actual registry ownership moves here at detach.
    removal: Option<Box<Generations>>,
    identity: Arc<()>,
    roots: [TextHeadIdData; 4],
    plan: TextRetirementPlanIdData,
    group: [u8; MAX_GROUP_BYTES],
    group_len: usize,
    signer: [u8; 32],
    source: BootstrapSourceId,
    lane: OriginalV2TextCreateLaneObservation,
    original_retain: crate::store::persistence::v2_text::OriginalV2RetainObservation,
    scan_complete: bool,
    detached: bool,
    #[cfg(all(test, windows))]
    after_close: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    after_detach: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(all(test, windows))]
    before_wait: Option<Box<dyn FnOnce() + Send>>,
    basis: V2AdvancedAppendBasis,
    // LAST: actual query-owned generations above have their OWN payment; this
    // original observation additionally funds every new fixed closure wrapper.
    original: OriginalV2TextAcceptedCreateObservation,
}

pub(in crate::store) struct PreparedV2TextRetirementRootsClose {
    state: Option<Box<RootsState>>,
    // Exactly one preparation claim, with at most the outside Removed alias later.
    // LAST: no new preparation while actual generation metadata still drains.
    claim: Arc<V2TextRetirementRootsClaim>,
}
pub(in crate::store) struct V2TextRetirementRootsClosed {
    state: Box<RootsState>,
    claim: Arc<V2TextRetirementRootsClaim>,
}
pub(in crate::store) struct V2TextRetirementRootsDrained {
    state: Box<RootsState>,
    claim: Arc<V2TextRetirementRootsClaim>,
}
pub(in crate::store) struct RemovedV2TextRetirementRootGenerations {
    _generations: Box<Generations>,
    identity: Weak<()>,
    original: OriginalV2TextAcceptedCreateObservation,
    plan: TextRetirementPlanIdData,
    _claim: Arc<V2TextRetirementRootsClaim>,
}

/// B's independent metadata debit includes BOTH fixed 32-entry arrays and bounded
/// wrapper/move/observation tails before any allocation. No SDK/work credit pays it.
pub(in crate::store) const V2_TEXT_RETIREMENT_ROOTS_LAYOUT_BYTES: usize =
    std::mem::size_of::<RootsState>()
        + 2 * MAX_QUERY_GENERATIONS * std::mem::size_of::<Option<Arc<QueryGeneration>>>()
        + 2 * std::mem::size_of::<PreparedV2TextRetirementRootsClose>()
        + 2 * std::mem::size_of::<V2TextRetirementRootsClosed>()
        + 2 * std::mem::size_of::<V2TextRetirementRootsDrained>()
        + 2 * std::mem::size_of::<RemovedV2TextRetirementRootGenerations>()
        + 8 * 64;

impl PreparedV2TextRetirementRootsClose {
    /// DATA preparation only. Whole B/advanced basis/lane remain borrowed unchanged
    /// on refusal. The maxONE claim precedes both arrays and identity allocation.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        prepared: &PreparedV2TextAcceptedCreate,
        basis: &V2AdvancedAppendBasis,
        lane: &V2TextCreateLane,
    ) -> Result<Self, AppError> {
        prepared.resources().validate_original(store, source)?;
        let original = prepared.observe_original();
        original.validate_prepaid_account()?;
        basis.require_original_accepted(&original)?;
        basis.require_same_original(prepared.accepted_final_basis_data()?)?;
        let claim = prepared.claim_retirement_roots_preparation()?;
        prepared.resources().validate_original(store, source)?;
        let roots = prepared.retirement_roots_data()?;
        let plan = prepared.retirement_plan_id_data()?;
        if roots.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(refused());
        }
        let context = source.data().context.spec_data();
        if context.group.is_empty() || context.group.len() > MAX_GROUP_BYTES {
            return Err(refused());
        }
        let lane_observation = lane.observe_original();
        lane_observation.require_same_original(lane)?;
        let mut group = [0; MAX_GROUP_BYTES];
        group[..context.group.len()].copy_from_slice(context.group);
        let state = Box::new(RootsState {
            snapshot: Box::new(std::array::from_fn(|_| None)),
            removal: Some(Box::new(std::array::from_fn(|_| None))),
            identity: Arc::new(()),
            roots,
            plan,
            group,
            group_len: context.group.len(),
            signer: context.signer,
            source: source.data().source,
            lane: lane_observation,
            original_retain: prepared.original_retain(),
            scan_complete: false,
            detached: false,
            #[cfg(all(test, windows))]
            after_close: None,
            #[cfg(all(test, windows))]
            after_detach: None,
            #[cfg(all(test, windows))]
            before_wait: None,
            basis: basis.clone(),
            original,
        });
        // The claim is declared before paid payload allocation; early-error local
        // drop order drains state first. Ownership keeps the same order thereafter.
        let result = Self {
            state: Some(state),
            claim: Arc::new(claim),
        };
        prepared
            .resources()
            .require_bound_original_negative(store, source)?;
        Ok(result)
    }
    pub(in crate::store) fn require_original(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        basis: &V2AdvancedAppendBasis,
    ) -> Result<(), AppError> {
        let state = self.state.as_ref().ok_or_else(refused)?;
        state.original.require_same_original(original)?;
        state.basis.require_same_original(basis)
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_close_for_test(&mut self, run: Box<dyn FnOnce() + Send>) {
        self.state
            .as_mut()
            .expect("unspent close preparation")
            .after_close = Some(run);
    }
}

impl RootsState {
    fn matches(&self, generation: &QueryGeneration) -> bool {
        generation.key.group() == &self.group[..self.group_len]
            && generation.key.signer == self.signer
            && generation.source == self.source
            && self
                .roots
                .contains(&generation.parent.selected_head_id_data())
    }
    fn require_namespace(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        self.lane.require_same_original(lane)?;
        lane.require_registered(namespace)?;
        let admission = namespace.scope_admission();
        if admission.group() != &self.group[..self.group_len]
            || admission.signer() != &self.signer
            || admission.source().data().source != self.source
        {
            return Err(refused());
        }
        namespace.validate_source_only()
    }
    fn drained(&self) -> bool {
        self.scan_complete
            && self.snapshot.iter().flatten().all(|generation| {
                let state = generation.state.load(Ordering::Acquire);
                state & GENERATION_CLOSING != 0
                    && state & READER_MASK == 0
                    && generation.control_io.load(Ordering::Acquire) == 0
            })
    }
    fn require_original(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        plan: &TextRetirementPlanIdData,
    ) -> Result<(), AppError> {
        self.original.require_same_original(original)?;
        if self.plan != *plan {
            return Err(refused());
        }
        Ok(())
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Called ONLY by the real Current wrapper AFTER its concrete B/WB/final-basis
    /// proof. The authoritative full registry scan spans BOTH query families and
    /// every paid observer of the same group/signer/source and four original O IDs.
    pub(in crate::store) fn close_text_retirement_roots_into(
        &mut self,
        lane: &V2TextCreateLane,
        prepared: &mut Option<PreparedV2TextRetirementRootsClose>,
        closed: &mut Option<V2TextRetirementRootsClosed>,
    ) -> Result<(), AppError> {
        if let Some(existing) = closed.as_ref() {
            if prepared.is_some() {
                return Err(refused());
            }
            existing.state.require_namespace(self, lane)?;
            return self.validate_source_only();
        }
        let input = prepared.as_mut().ok_or_else(refused)?;
        let state = input.state.as_mut().ok_or_else(refused)?;
        state.require_namespace(self, lane)?;
        lane.require_workspace_idle(self, &state.original_retain)?;
        self.require_text_control_write_idle()?;
        if state.scan_complete || state.snapshot.iter().any(Option::is_some) {
            return Err(refused());
        }
        // Capture the complete authoritative set BEFORE the first monotone close.
        // Different paid read resources and BOTH query families participate. Fixed
        // copied O IDs avoid hashing, allocation or fallible work in the close tail.
        if let Some(buffer) = self.text_lane_runtime().queries.as_ref() {
            for (index, generation) in buffer.slots.iter().enumerate() {
                if let Some(generation) = generation.as_ref().filter(|entry| state.matches(entry)) {
                    state.snapshot[index] = Some(generation.clone());
                }
            }
        }
        // From the first flag through BOTH outside caches: infallible native flags
        // and owning moves only. ScopeStop at a later postcheck cannot lose this set.
        for generation in state.snapshot.iter().flatten() {
            generation
                .state
                .fetch_or(GENERATION_CLOSING, Ordering::AcqRel);
        }
        state.scan_complete = true;
        let input = prepared.take().expect("same original close owner checked");
        *closed = Some(V2TextRetirementRootsClosed {
            state: input.state.expect("same paid closure payload"),
            claim: input.claim,
        });
        #[cfg(all(test, windows))]
        if let Some(run) = closed
            .as_mut()
            .expect("actual close cached before hook")
            .state
            .after_close
            .take()
        {
            run();
        }
        Ok(())
    }
}

impl V2TextRetirementRootsClosed {
    /// Observe entry into the REAL nonzero-End wait predicate under its wait mutex.
    /// Configure outside writers; no caller count or synthetic drain is accepted.
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_before_wait_for_test(&mut self, run: Box<dyn FnOnce() + Send>) {
        self.state.before_wait = Some(run);
    }
    #[cfg(all(test, windows))]
    pub(in crate::store) fn snapshot_counts_for_test(&self) -> (usize, usize, usize, usize) {
        let mut count = (0, 0, 0, 0);
        for generation in self.state.snapshot.iter().flatten() {
            match &generation.parent {
                QueryOrigin::Token(_) => count.0 += 1,
                QueryOrigin::Accepted(_) => count.1 += 1,
            }
            count.2 += generation.state.load(Ordering::Acquire) & READER_MASK;
            count.3 += generation.control_io.load(Ordering::Acquire);
        }
        count
    }
    pub(in crate::store) fn require_original(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        basis: &V2AdvancedAppendBasis,
    ) -> Result<(), AppError> {
        self.state.original.require_same_original(original)?;
        self.state.basis.require_same_original(basis)
    }
    /// Nonblocking actual-End join, OUTSIDE all writers. A false return owns the SAME
    /// closed snapshot; it is not an exception, elapsed-time proof or caller count.
    pub(in crate::store) fn poll_drained_into(
        closed: &mut Option<Self>,
        drained: &mut Option<V2TextRetirementRootsDrained>,
    ) -> Result<bool, AppError> {
        if let Some(existing) = drained.as_ref() {
            if closed.is_some() || !existing.state.drained() {
                return Err(refused());
            }
            return Ok(true);
        }
        if !closed.as_ref().ok_or_else(refused)?.state.drained() {
            return Ok(false);
        }
        let closed = closed.take().expect("same actual closed snapshot");
        *drained = Some(V2TextRetirementRootsDrained {
            state: closed.state,
            claim: closed.claim,
        });
        Ok(true)
    }
    /// Blocking actual Condvar join for a native background worker, never AD/Root,
    /// a UI callback or an owner Drop. Scope cancellation cannot fake this drain.
    pub(in crate::store) fn join_drained_into(
        closed: &mut Option<Self>,
        drained: &mut Option<V2TextRetirementRootsDrained>,
    ) -> Result<(), AppError> {
        if drained.is_some() {
            return Self::poll_drained_into(closed, drained).map(|_| ());
        }
        let current = closed.as_mut().ok_or_else(refused)?;
        if !current.state.scan_complete {
            return Err(refused());
        }
        let state = current.state.as_mut();
        for generation in state.snapshot.iter().flatten() {
            let mut wait = generation
                .wait
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            while generation.state.load(Ordering::Acquire) & READER_MASK != 0
                || generation.control_io.load(Ordering::Acquire) != 0
            {
                // This hook is reached ONLY while actual Ends are outstanding,
                // holding the SAME mutex used by End destruction. The test releases
                // its real readers after observing it; Condvar.wait then atomically
                // releases that mutex, so notification cannot precede the predicate.
                #[cfg(all(test, windows))]
                if let Some(run) = state.before_wait.take() {
                    run();
                }
                wait = generation
                    .changed
                    .wait(wait)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        if !Self::poll_drained_into(closed, drained)? {
            return Err(refused());
        }
        Ok(())
    }
}

impl V2TextRetirementRootsDrained {
    #[cfg(all(test, windows))]
    pub(in crate::store) fn set_after_detach_for_test(&mut self, run: Box<dyn FnOnce() + Send>) {
        self.state.after_detach = Some(run);
    }
    /// Fixed original-plan DATA only; no decoder/account wait follows final proof.
    pub(in crate::store) fn retirement_plan_id_data(&self) -> TextRetirementPlanIdData {
        self.state.plan
    }
    /// Historical exact-original actual-drain evidence only. Fresh typed WD/WC
    /// Current/source/basis/idle validation remains mandatory for every effect.
    pub(in crate::store) fn require_original(
        &self,
        original: &OriginalV2TextAcceptedCreateObservation,
        plan: &TextRetirementPlanIdData,
    ) -> Result<(), AppError> {
        self.state.require_original(original, plan)?;
        if !self.state.drained() {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn require_basis(
        &self,
        basis: &V2AdvancedAppendBasis,
    ) -> Result<(), AppError> {
        self.state.basis.require_same_original(basis)
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Detach ONLY exact closed registry Arcs after their real reader/IO Ends drain.
    /// The outside owning receipt is cached before any Current late error; retry
    /// uses that private state, never absence of a registry entry as a drain proof.
    pub(in crate::store) fn detach_text_retirement_roots_into(
        &mut self,
        lane: &V2TextCreateLane,
        drained: &mut V2TextRetirementRootsDrained,
        removed: &mut Option<RemovedV2TextRetirementRootGenerations>,
    ) -> Result<(), AppError> {
        let state = &mut drained.state;
        state.require_namespace(self, lane)?;
        if !state.drained() {
            return Err(refused());
        }
        if state.detached {
            if let Some(existing) = removed.as_ref() {
                state.original.require_same_original(&existing.original)?;
                if state.plan != existing.plan
                    || !existing.identity.ptr_eq(&Arc::downgrade(&state.identity))
                {
                    return Err(refused());
                }
            }
            if self
                .text_lane_runtime()
                .queries
                .as_ref()
                .is_some_and(|buffer| {
                    buffer
                        .slots
                        .iter()
                        .flatten()
                        .any(|entry| state.matches(entry))
                })
            {
                return Err(refused());
            }
            return self.validate_source_only();
        }
        if removed.is_some() || state.removal.is_none() {
            return Err(refused());
        }
        let mut indices = [None; MAX_QUERY_GENERATIONS];
        if let Some(buffer) = self.text_lane_runtime().queries.as_ref() {
            for entry in buffer
                .slots
                .iter()
                .flatten()
                .filter(|entry| state.matches(entry))
            {
                if !state
                    .snapshot
                    .iter()
                    .flatten()
                    .any(|old| Arc::ptr_eq(old, entry))
                {
                    return Err(refused());
                }
            }
            for (index, expected) in state.snapshot.iter().enumerate() {
                if let Some(expected) = expected {
                    indices[index] = Some(
                        buffer
                            .slots
                            .iter()
                            .position(|slot| {
                                slot.as_ref()
                                    .is_some_and(|entry| Arc::ptr_eq(entry, expected))
                            })
                            .ok_or_else(refused)?,
                    );
                }
            }
        } else if state.snapshot.iter().any(Option::is_some) {
            return Err(refused());
        }
        // All comparisons/capacity checks precede the first take. Closed entries
        // cannot be reused by ordinary metadata admission (that accepts state==0).
        let mut generations = state
            .removal
            .take()
            .expect("same preallocated outside array");
        if let Some(buffer) = self.text_lane_runtime_mut().queries.as_mut() {
            for (index, slot) in indices.into_iter().enumerate() {
                if let Some(slot) = slot {
                    generations[index] = buffer.slots[slot].take();
                }
            }
        }
        state.detached = true;
        *removed = Some(RemovedV2TextRetirementRootGenerations {
            _generations: generations,
            identity: Arc::downgrade(&state.identity),
            original: state.original.clone(),
            plan: state.plan,
            _claim: drained.claim.clone(),
        });
        #[cfg(all(test, windows))]
        if let Some(run) = state.after_detach.take() {
            run();
        }
        Ok(())
    }

    /// Final cached native predicate after full concrete WD/WC proof; no new File,
    /// account wait, allocation or join. This does not itself issue any effect role.
    pub(in crate::store) fn require_text_retirement_roots_drained(
        &mut self,
        lane: &V2TextCreateLane,
        drained: &V2TextRetirementRootsDrained,
    ) -> Result<(), AppError> {
        drained.state.require_namespace(self, lane)?;
        if !drained.state.detached || !drained.state.drained() {
            return Err(refused());
        }
        if self
            .text_lane_runtime()
            .queries
            .as_ref()
            .is_some_and(|buffer| {
                buffer
                    .slots
                    .iter()
                    .flatten()
                    .any(|entry| drained.state.matches(entry))
            })
        {
            return Err(refused());
        }
        self.validate_source_only()
    }
}

macro_rules! roots_debug {
    ($($ty:ident),+) => {$(impl std::fmt::Debug for $ty {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct(stringify!($ty)).finish_non_exhaustive()
        }
    })+};
}
roots_debug!(
    PreparedV2TextRetirementRootsClose,
    V2TextRetirementRootsClosed,
    V2TextRetirementRootsDrained,
    RemovedV2TextRetirementRootGenerations
);
