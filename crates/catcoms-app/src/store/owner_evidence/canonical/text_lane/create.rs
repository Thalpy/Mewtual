//! Closed ordinary-text ownership of an original, unused negative work lane.
//!
//! Conversion consumes the unique raw caller. There is deliberately no raw-lane getter,
//! reverse conversion, parsed-plan adopter, or positive effect admission here. In
//! particular, this exclusion does not establish a token miss, clear workspace, selected
//! private root, signer permission, or permission to abandon durable pending work.
use super::*;
mod retirement;
pub(in crate::store) use retirement::{
    RetiredV2TextCreateLane, V2_TEXT_CREATE_RETIREMENT_LAYOUT_BYTES,
};
mod workspace;
pub(in crate::store) use workspace::{
    V2TextCreateBinding, V2TextCreateControlWriteEnd, V2TextCreateWorkspaceEnd,
};

struct CreateLaneControl {
    identity: Arc<()>,
    work: workspace::CreateWorkControl,
    // LAST: all original-control metadata survives on its own independent payment.
    fee: Arc<CatalogReadReservation>,
}

/// Preallocated outside all host writers. A failed conversion preserves both this owner
/// and the caller's SAME raw lane. It cannot manufacture a lane or adopt a registry slot.
pub(in crate::store) struct PreparedV2TextCreateLane {
    control: Option<Arc<CreateLaneControl>>,
    original_lane: OriginalV2TextLaneObservation,
    prepared_identity: Weak<()>,
    // LAST, including after the control moved into the outside retained result slot.
    fee: Arc<CatalogReadReservation>,
}

/// Owns the SAME unique registered exclusion. The raw caller is private and never lent
/// to promotion APIs. Dropping it cancels future admissions but leaves the real strong
/// registry debt intact; ordinary WD/WC or durable suspension must retire it later.
pub(in crate::store) struct V2TextCreateLane {
    original_lane: V2TextLane,
    control: Arc<CreateLaneControl>,
}

/// Exact negative identity only. This weak observation neither keeps a caller alive nor
/// opens a root, acquires work, releases the registry entry, or renews Current.
#[derive(Clone)]
pub(in crate::store) struct OriginalV2TextCreateLaneObservation {
    identity: Weak<()>,
    original_lane: OriginalV2TextLaneObservation,
    // LAST: tiny weak/control tails do not borrow source/SDK or a future16MiB work debit.
    _fee: Arc<CatalogReadReservation>,
}

/// Separate actual debit for the concrete ordinary ownership shell. Producer, query,
/// immutable work, signing, reader/work-end and retirement payments remain additional.
/// This is an exact logical metadata invoice, not an allocator or operating-system limit.
pub(in crate::store) const V2_TEXT_CREATE_LANE_BYTES: usize =
    std::mem::size_of::<CreateLaneControl>()
        + std::mem::size_of::<PreparedV2TextCreateLane>()
        + std::mem::size_of::<V2TextCreateLane>()
        + 2 * std::mem::size_of::<OriginalV2TextCreateLaneObservation>()
        + workspace::CREATE_WORKSPACE_CONTROL_LAYOUT_BYTES
        + 4 * 64;

impl PreparedV2TextCreateLane {
    /// Reserve before ANY new Arc/control allocation and repeat the original negatives
    /// after the account wait. Exact real registration and unused-promotion checks are
    /// performed again by the actual canonical conversion, never inferred from these DATA.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        owner: &V2TextOwnerRef<'_>,
        lane: &V2TextLane,
    ) -> Result<Self, AppError> {
        owner.require_closed_family()?;
        let writer = owner.observe_writer();
        owner.require_original_negative(store, &writer)?;
        let original_lane = lane.observe_original();
        original_lane.require_same_original(lane)?;
        let data = owner.source().data();
        let context = data.context.spec_data();
        if lane.body.source_id != data.source
            || lane.body.key.group() != context.group
            || lane.body.key.signer != context.signer
        {
            return Err(refused());
        }
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = pool.try_reserve(V2_TEXT_CREATE_LANE_BYTES)?;
        fee.validate(pool, V2_TEXT_CREATE_LANE_BYTES)?;
        owner.require_original_negative(store, &writer)?;
        original_lane.require_same_original(lane)?;
        let fee = Arc::new(fee);
        let control = Arc::new(CreateLaneControl {
            identity: Arc::new(()),
            work: workspace::CreateWorkControl::new(),
            fee: fee.clone(),
        });
        Ok(Self {
            prepared_identity: Arc::downgrade(&control.identity),
            control: Some(control),
            original_lane,
            fee,
        })
    }

    /// This may wait on the account. The caller must repeat the actual namespace proof
    /// afterward, before transferring either unique outside owner under Current.
    pub(in crate::store) fn validate_prepaid_account(
        &self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        self.fee.validate(pool, V2_TEXT_CREATE_LANE_BYTES)
    }

    fn require_same_result(&self, lane: &V2TextCreateLane) -> Result<(), AppError> {
        self.original_lane
            .require_same_original(&lane.original_lane)?;
        if !self
            .prepared_identity
            .ptr_eq(&Arc::downgrade(&lane.control.identity))
        {
            return Err(refused());
        }
        Ok(())
    }
}

impl V2TextOwnerNamespace<'_, '_> {
    /// Negative exclusion conversion only. All account/physical checks happen before
    /// this pure final edge. No effect permission follows from the newly retained owner.
    pub(in crate::store) fn convert_text_create_lane_into(
        &mut self,
        prepared: &mut PreparedV2TextCreateLane,
        caller: &mut Option<V2TextLane>,
        retained: &mut Option<V2TextCreateLane>,
    ) -> Result<(), AppError> {
        if let Some(existing) = retained.as_ref() {
            if caller.is_some() || prepared.control.is_some() {
                return Err(refused());
            }
            prepared.require_same_result(existing)?;
            return existing.require_registered(self);
        }
        let lane = caller.as_ref().ok_or_else(refused)?;
        prepared.original_lane.require_same_original(lane)?;
        lane.require_registered(self)?;
        lane.body.promotion.require_unused_create_conversion()?;
        let control = prepared.control.as_ref().ok_or_else(refused)?;
        if !prepared
            .prepared_identity
            .ptr_eq(&Arc::downgrade(&control.identity))
        {
            return Err(refused());
        }
        // Canonical ownership serializes both conversion and every promotion admission.
        // The non-Clone raw caller is moved, with no destructor, callback, allocation or
        // fallible operation between that move and the OUTSIDE result cache. A late Scope
        // failure therefore cannot lose the exclusion or expose a reusable raw caller.
        *retained = Some(V2TextCreateLane {
            original_lane: caller.take().expect("same unique caller checked above"),
            control: prepared
                .control
                .take()
                .expect("same prepared control checked"),
        });
        Ok(())
    }
}

impl V2TextCreateLane {
    pub(in crate::store) fn require_registered(
        &self,
        namespace: &mut V2TextOwnerNamespace<'_, '_>,
    ) -> Result<(), AppError> {
        self.original_lane.require_registered(namespace)
    }

    pub(in crate::store) fn observe_original(&self) -> OriginalV2TextCreateLaneObservation {
        OriginalV2TextCreateLaneObservation {
            identity: Arc::downgrade(&self.control.identity),
            original_lane: self.original_lane.observe_original(),
            _fee: self.control.fee.clone(),
        }
    }
}

impl OriginalV2TextCreateLaneObservation {
    pub(in crate::store) fn is_cancelled(&self) -> bool {
        self.identity.strong_count() == 0 || self.original_lane.is_cancelled()
    }

    pub(in crate::store) fn require_same_original(
        &self,
        lane: &V2TextCreateLane,
    ) -> Result<(), AppError> {
        self.original_lane
            .require_same_original(&lane.original_lane)?;
        if self.is_cancelled()
            || !self
                .identity
                .ptr_eq(&Arc::downgrade(&lane.control.identity))
        {
            return Err(refused());
        }
        Ok(())
    }
}

macro_rules! redacted_create_debug {
    ($($ty:ident),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
redacted_create_debug!(
    PreparedV2TextCreateLane,
    V2TextCreateLane,
    OriginalV2TextCreateLaneObservation,
);
