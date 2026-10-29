//! Actual native ledger capacity for the distinct admitted profile.
//!
//! This is intentionally not an Existing ParentClaim or a generic selected-owner
//! adapter. It can reserve capacity before agreement, but cannot select a peer,
//! sign Ready, create a Row/Action or enter any Existing lookup/consumer.
use super::*;
use crate::voice_authority::AdmittedPublishedKey;

/// An original allocator-excluded serial plus the two prepaid contributions a
/// future admitted parent must MOVE. Never drop/recharge these at consumption.
///
/// Owners are destroyed before the SAME private final-return marker can clear
/// the stamp. In particular the actual PreparedPhase/credits and admitted key's
/// outside payer return before reuse, including unwinding destruction. Ledger
/// stores only the scalar stamp, so this strong Ledger owner has no backedge.
#[allow(dead_code)] // Consumed by the separate admitted Slot/31, not Existing.
pub(in crate::voice_handoff) struct AdmittedParentClaim {
    owners: AdmittedOwners,
    returned: Returned,
}
struct AdmittedOwners {
    selected: AdmittedPublishedKey,
    session: Arc<SessionIdentity>,
    generation: u64,
    _prepared: PreparedPhase,
    _metadata: Credit,
}
#[allow(dead_code)] // No public operational consumer is exposed by this primitive.
impl AdmittedParentClaim {
    /// Complete synchronous reservation/custody graph; the native task includes
    /// this in its own preallocation inventory. The phase/row payments are
    /// independent, original contributions, not storage borrowed from that task.
    pub(in crate::voice_handoff) const fn preparation_layout_bytes() -> usize {
        std::mem::size_of::<AdmittedOwners>()
            + std::mem::size_of::<Returned>()
            + std::mem::size_of::<Credit>()
            + std::mem::size_of::<PreparedPhase>()
            + std::mem::size_of::<std::sync::MutexGuard<'static, Index>>()
            // A claim may outlive its physical Row. Sponsor the WHOLE retained
            // identity allocation, not only the Arc pointer in AdmittedOwners.
            + std::mem::size_of::<SessionIdentity>()
            + 2 * std::mem::size_of::<usize>()
    }

    /// Exact immutable provenance only. A live lease here is not NativeActive,
    /// current selection or agreement; the Slot repeats those independent checks
    /// under UI -> Coordinator -> Row before any positive attachment/effect.
    pub(in crate::voice_handoff) fn matches(
        &self,
        selected: &AdmittedPublishedKey,
        session: &Arc<SessionIdentity>,
        generation: u64,
    ) -> bool {
        generation != 0
            && self.owners.generation == generation
            && self.owners.selected.is_same(selected)
            && Arc::ptr_eq(&self.owners.session, session)
            && selected.lease().is_live()
            && selected.lease().is_admitted()
    }
    pub(in crate::voice_handoff) fn belongs_to(&self, ledger: &Arc<Ledger>) -> bool {
        Arc::ptr_eq(&self.returned.ledger, ledger)
    }

    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_id(&self) -> RowId {
        self.returned.id
    }
}

#[cfg(test)]
pub(in crate::voice_handoff) fn fixture_existing_allocators_are_excluded(
    ledger: &Arc<Ledger>,
    reserved: RowId,
) {
    let (_authority, selected, selector) = crate::voice_handoff::ledger::tests::selection();
    assert!(matches!(
        ledger.row(reserved),
        Err(VoiceError::Unauthorized)
    ));
    assert!(matches!(
        ledger.begin(selected.clone(), Purpose::Derive(selector)),
        Err(VoiceError::Capacity)
    ));
    // The actual shared child allocator only: no valid confirmation parent,
    // Root authorization or SDP execution is claimed by this pressure oracle.
    let mut index = ledger.index.lock().unwrap();
    assert!(index.vacant().is_none());
    assert!(matches!(
        ledger.begin_indexed(
            selected,
            Purpose::Confirmation {
                parent: reserved,
                request: ConfirmationPurpose::Start(None),
            },
            None,
            &mut index,
        ),
        Err(VoiceError::Capacity)
    ));
    assert!(index.rows.iter().all(Option::is_none));
}

#[cfg(test)]
pub(in crate::voice_handoff) fn fixture_existing_claim(ledger: &Arc<Ledger>) -> ParentClaim {
    let (_authority, selected, _) = crate::voice_handoff::ledger::tests::selection();
    ledger
        .reserve_parent(selected, Arc::new(SessionIdentity([71; 32])), 1)
        .unwrap()
}

impl Ledger {
    /// Reserve a REAL shared allocator position without a dummy selector, Row,
    /// Action or Root invocation. The private admitted key type cannot be
    /// constructed from renderer fields or an Existing selected owner.
    ///
    /// Do not call under native locks: speculative payments/preparation must
    /// drop outside Index on refusal. The caller rechecks its original actual
    /// activation, UI generation, physical Session and canonical g at attachment.
    /// A future consume must atomically replace this SAME stamp with an admitted
    /// Row before disarming Returned; ordinary Begin is never a substitute.
    #[allow(dead_code)] // Admission Slot/31 integration is a separate typed consumer.
    pub(in crate::voice_handoff) fn reserve_admitted_parent(
        self: &Arc<Self>,
        selected: AdmittedPublishedKey,
        session: Arc<SessionIdentity>,
        generation: u64,
    ) -> Result<super::super::AdmittedParentClaim, VoiceError> {
        if generation == 0 || !selected.lease().is_live() || !selected.lease().is_admitted() {
            return Err(VoiceError::Unauthorized);
        }
        // Enforce the complete wrapper graph in production too, before charging
        // either contribution. No invoice term is removed to make a fee fit.
        let bytes = std::mem::size_of::<AdmittedParentClaim>()
            + AdmittedParentClaim::preparation_layout_bytes();
        if bytes > ROW_BYTES {
            return Err(VoiceError::Capacity);
        }
        let resources = selected.resources();
        let scope = selected.binding().scope;
        let metadata = resources.charge(scope.group, ROW_BYTES, ResourceKind::Retained)?;
        let prepared = PreparedPhase::new(resources, scope, selected.lease().clone())?;
        let owners = AdmittedOwners {
            selected,
            session,
            generation,
            _prepared: prepared,
            _metadata: metadata,
        };
        // Declare Index last so every failure releases its guard before owners.
        let mut index = transition::try_lock(&self.index)?;
        if !owners.selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        let slot = index.vacant().ok_or(VoiceError::Capacity)?;
        let serial = index.serial.checked_add(1).ok_or(VoiceError::Capacity)?;
        let stamp = NonZeroU64::new(serial).ok_or(VoiceError::Capacity)?;
        index.serial = serial;
        index.reserved[slot] = Some(stamp);
        drop(index);
        Ok(AdmittedParentClaim {
            owners,
            returned: Returned {
                ledger: self.clone(),
                id: RowId { slot, serial },
                #[cfg(test)]
                before_release: None,
            },
        })
    }

    #[cfg(test)]
    pub(in crate::voice_handoff) fn fixture_admitted_claim_has_only_reserved_stamp(
        &self,
        id: RowId,
    ) -> bool {
        let index = self.index.lock().unwrap();
        index.rows[id.slot].is_none()
            && index.reserved[id.slot].map(NonZeroU64::get) == Some(id.serial)
    }
}
