//! Original-parent reservations for the native confirmation adapter. A copied
//! RowId selects history only; the indexed strong parent and its opaque Root plan establish
//! provenance. The common Index -> parent State gate orders reservation against retirement.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;

/// Native-selected exact request identity. These values are not constructors for Root
/// actions or incoming tokens. Incoming names one of the nine supported protected roles;
/// its actual Action still has to arrive from authenticated native Root intake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmationPurpose {
    Start(Option<[u8; 32]>),
    Respond(RowId),
    Incoming(u8),
}

impl ResolvedRow {
    /// Immutable original parent purpose for native adapter/test projection. It supplies
    /// no current grant or factory authority, including after ordinary Return.
    pub(in crate::voice_handoff) fn confirmation_selector(
        &self,
    ) -> Result<PairSelector, VoiceError> {
        let _index = self.indexed()?;
        match self.row.key.purpose {
            Purpose::Derive(selector) if self.row.key.parent.is_none() => Ok(selector),
            _ => Err(VoiceError::Unauthorized),
        }
    }

    /// Native routing hint only: the retained publication is still resolved/rechecked by
    /// resume_known_child under Index at final admission. A browser cannot supply this bit.
    pub(in crate::voice_handoff) fn has_pending_publication(&self) -> Result<bool, VoiceError> {
        let _index = self.indexed()?;
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        Ok(matches!(&state.body, Body::Pending(pending) if pending.published.is_some()))
    }

    /// Fixed historical binding from the SAME original opaque plan, including after
    /// Derive compaction. It supplies no current-call or child-reservation permission.
    pub(crate) fn confirmation_handshake(&self) -> Result<[u8; 32], VoiceError> {
        let _index = self.indexed()?;
        let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        if self.row.key.parent.is_some() {
            return Err(VoiceError::Unauthorized);
        }
        Ok(state
            .plan()
            .ok_or(VoiceError::Unauthorized)?
            .identity
            .handshake())
    }
}

impl Work {
    /// Fixed identity of this actual child's retained original parent. In particular this
    /// is NOT the child's serial, the renderer's role number, or a newly selected pair.
    /// Work already owns the real child; both immutable keys and the strong parent survive
    /// native Return/End. Returning metadata here grants no new engine or plan authority.
    pub(in crate::voice_handoff) fn confirmation_parent_id(&self) -> Result<RowId, VoiceError> {
        let Purpose::Confirmation { parent, .. } = self.row.key.purpose else {
            return Err(VoiceError::Unauthorized);
        };
        let owner = self
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        if owner.id != parent
            || owner.key.parent.is_some()
            || !matches!(owner.key.purpose, Purpose::Derive(_))
        {
            return Err(VoiceError::Unauthorized);
        }
        Ok(owner.id)
    }
}

impl Ledger {
    /// Resume ONLY an already indexed child with a known original publication. Unlike
    /// begin_child, a role hint here can never reserve an empty position or call Root. The
    /// actual parent/child allocations are rechecked together under Index before rearming
    /// attachment. Missing/uncertain publications require a real native intake snapshot.
    pub(in crate::voice_handoff) fn resume_known_child(
        self: &Arc<Self>,
        selected: Arc<SelectedHandoffOwner>,
        parent: &ResolvedRow,
        child: &ResolvedRow,
        request: ConfirmationPurpose,
    ) -> Result<Begin, VoiceError> {
        let purpose = Purpose::Confirmation {
            parent: parent.id(),
            request,
        };
        if !Arc::ptr_eq(self, &parent.ledger)
            || !Arc::ptr_eq(self, &child.ledger)
            || !selected.lease().is_live()
            || parent.row.key.parent.is_some()
            || !matches!(parent.row.key.purpose, Purpose::Derive(_))
            || !parent.row.key.matches(&selected, parent.row.key.purpose)
            || !child.row.key.matches(&selected, purpose)
            || !child
                .row
                .key
                .parent
                .as_ref()
                .is_some_and(|actual| Arc::ptr_eq(actual, &parent.row))
            || !matches!(child.row.key.purpose, Purpose::Confirmation { parent: id, .. } if id == parent.id())
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        for row in [&parent.row, &child.row] {
            if !index
                .rows
                .get(row.id.slot)
                .and_then(Option::as_ref)
                .is_some_and(|actual| actual.id == row.id && Arc::ptr_eq(actual, row))
            {
                return Err(VoiceError::Unauthorized);
            }
        }
        {
            let state = child.row.state.lock().unwrap_or_else(|e| e.into_inner());
            match &state.body {
                Body::Pending(pending) if pending.published.is_none() || pending.worker => {
                    return Err(VoiceError::Capacity)
                }
                Body::ClosingPending => return Err(VoiceError::Capacity),
                Body::Pending(_) => {}
                _ => return Ok(Begin::Existing(child.id())),
            }
        }
        // Index still pins the EXACT existing row, so begin_indexed cannot allocate a new
        // purpose in the gap between provenance validation and original attachment retry.
        self.begin_indexed(
            selected,
            child.row.key.purpose,
            Some(&parent.row),
            &mut index,
        )
    }

    /// Recover only a descendant of the actual original indexed parent. Public purpose
    /// fields are lookup hints; equal RowIds in another ledger or a detached parent cannot
    /// substitute for the strong parent allocation captured before child publication.
    ///
    /// This is historical resolution, not admission: no live lease, plan control, State
    /// guard, Root callback, allocation, or factory is needed. `id=None` recovers a lost
    /// first reply; later operations must also match their authenticated child header.
    pub(in crate::voice_handoff) fn lookup_child(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        parent: &ResolvedRow,
        request: ConfirmationPurpose,
        id: Option<RowId>,
    ) -> Result<ResolvedRow, VoiceError> {
        if !Arc::ptr_eq(self, &parent.ledger)
            || parent.row.key.parent.is_some()
            || !matches!(parent.row.key.purpose, Purpose::Derive(_))
            || !parent.row.key.matches(selected, parent.row.key.purpose)
        {
            return Err(VoiceError::Unauthorized);
        }
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if !index
            .rows
            .get(parent.id().slot)
            .and_then(Option::as_ref)
            .is_some_and(|row| row.id == parent.id() && Arc::ptr_eq(row, &parent.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let purpose = Purpose::Confirmation {
            parent: parent.id(),
            request,
        };
        let row = index
            .rows
            .iter()
            .flatten()
            .find(|row| {
                id.is_none_or(|id| row.id == id)
                    && row.key.matches(selected, purpose)
                    && row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|owner| Arc::ptr_eq(owner, &parent.row))
            })
            .ok_or(VoiceError::Unauthorized)?;
        Ok(ResolvedRow {
            ledger: self.clone(),
            row: row.clone(),
        })
    }

    /// Reserve before Actor work. The returned response trigger is the actual retained
    /// same-parent decrypt, never an action reconstructed from serials or receipt fields.
    /// Caller must already own its physical/factory payment and current UI commit.
    pub(crate) fn begin_child(
        self: &Arc<Self>,
        selected: Arc<SelectedHandoffOwner>,
        parent: &ResolvedRow,
        request: ConfirmationPurpose,
    ) -> Result<(Begin, Option<Arc<CryptoAction>>), VoiceError> {
        if !Arc::ptr_eq(self, &parent.ledger) || !selected.lease().is_live() {
            return Err(VoiceError::Unauthorized);
        }
        // Reconcile outside Index/State: Root may sample its Clock and enter Engine.
        let trigger = if let ConfirmationPurpose::Respond(id) = request {
            let row = self.row(id)?;
            let resolved = ResolvedRow {
                ledger: self.clone(),
                row,
            };
            resolved.reconcile()?;
            Some(resolved)
        } else {
            None
        };
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if !index
            .rows
            .iter()
            .flatten()
            .any(|row| Arc::ptr_eq(row, &parent.row))
            || parent.row.key.parent.is_some()
            || !parent.row.key.matches(&selected, parent.row.key.purpose)
            || !matches!(parent.row.key.purpose, Purpose::Derive(_))
        {
            return Err(VoiceError::Unauthorized);
        }
        let parent_state = parent.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let purpose = Purpose::Confirmation {
            parent: parent.id(),
            request,
        };
        // Exact completed/published history cannot require a discarded trigger just to
        // recover attachment. In particular it must not re-enter the Actor after a lost reply.
        if let Some(row) = index
            .rows
            .iter()
            .flatten()
            .find(|row| row.key.matches(&selected, purpose))
        {
            let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
            let cached = match &state.body {
                Body::Pending(pending) => pending.published.is_some(),
                _ => true,
            };
            drop(state);
            if cached {
                return self
                    .begin_indexed(selected, purpose, Some(&parent.row), &mut index)
                    .map(|begin| (begin, None));
            }
        }
        // Only an ordinary Derive Return permits the next phase. Terminal intent or frozen
        // plan control can never be converted back into a child-adoption capability.
        if !parent_state.status.drained
            || parent_state.final_phase.is_some()
            || parent_state.final_delivery().is_some()
            || parent_state
                .plan()
                .is_none_or(|plan| plan.control.is_none())
        {
            return Err(VoiceError::Capacity);
        }
        if let ConfirmationPurpose::Incoming(role) = request {
            if !(1..10).contains(&role) {
                return Err(VoiceError::Unauthorized);
            }
        }
        let trigger_state = if let Some(trigger) = &trigger {
            if !trigger
                .row
                .key
                .parent
                .as_ref()
                .is_some_and(|row| Arc::ptr_eq(row, &parent.row))
                || !index
                    .rows
                    .iter()
                    .flatten()
                    .any(|row| Arc::ptr_eq(row, &trigger.row))
            {
                return Err(VoiceError::Unauthorized);
            }
            let state = trigger.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let action = match &state.body {
                Body::Live(live) => &live.action,
                Body::Drained(_, _, Some(action)) => action,
                _ => return Err(VoiceError::Unauthorized),
            };
            if action.operation() != CryptoOperation::DecryptConfirmation
                || !state.status.engine.receipt.as_ref().is_some_and(|receipt| {
                    // Native compaction marks its local projection retired. That bit is
                    // not Root's current-trigger decision; Root will recheck the ACTUAL
                    // retained accepted action, pair and token in its response factory.
                    receipt.outcome == CryptoOutcome::Opened
                })
                || !parent_state
                    .plan()
                    .is_some_and(|plan| plan.identity.contains(action))
            {
                return Err(VoiceError::Unauthorized);
            }
            // Only accepted Hello on the high side or Proof on the low side has a
            // successor. Finish is terminal. Reject that impossible request BEFORE
            // reserving/handing a purpose to Actor: an Actor refusal would otherwise
            // leave counted uncertain history which can unnecessarily block plan End.
            // Both fields come from the ACTUAL attached Root action, not renderer data.
            let role = state.child.ok_or(VoiceError::Unauthorized)?.role;
            if !matches!(
                (action.protected_binding()?.local_is_low, role),
                (false, 1 | 4 | 7) | (true, 2 | 5 | 8)
            ) {
                return Err(VoiceError::Unauthorized);
            }
            Some(state)
        } else {
            None
        };
        // All fallible validation/payment precedes cloning the actual trigger. A refusing
        // allocation must never destroy an Action beneath native Index/parent/child gates.
        let begin = self.begin_indexed(selected, purpose, Some(&parent.row), &mut index)?;
        let action = trigger_state.as_ref().map(|state| match &state.body {
            Body::Live(live) => live.action.clone(),
            Body::Drained(_, _, Some(action)) => action.clone(),
            _ => unreachable!("same guarded trigger"),
        });
        Ok((begin, action))
    }

    /// Original indexed relationships, not browser counts or weak-zero, determine whether
    /// the parent may freeze adoption. Call while Index and parent State are held, always
    /// in that order. This bounded scan adds no parent -> child strong ownership cycle.
    pub(super) fn children_locally_closed(index: &Index, parent: &Arc<Row>) -> bool {
        index
            .rows
            .iter()
            .flatten()
            .filter(|row| {
                row.key
                    .parent
                    .as_ref()
                    .is_some_and(|owner| Arc::ptr_eq(owner, parent))
            })
            .all(|row| {
                let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
                matches!(state.body, Body::ClosedPending)
                    || (matches!(state.body, Body::Drained(..)) && state.status.drained)
            })
    }
}

/// Paid fixed history, derived only from the original Root Action/input. This is not an
/// engine/packet capability. None message on an outgoing role means capture is incomplete.
#[derive(Clone, Copy)]
pub(super) struct ChildMetadata {
    pub(super) role: u8,
    message: Option<[u8; 32]>,
}

impl Registration {
    /// One transaction validates the normalized purpose and claims the actual role. No
    /// Clock, Root engine/input call or affine owner destruction occurs under these gates.
    /// `claim=false` only verifies a previously stored original publication for attachment.
    fn child_action_fence(
        &self,
        action: &Arc<CryptoAction>,
        claim: bool,
    ) -> Result<(), VoiceError> {
        let parent = self
            .row
            .key
            .parent
            .as_ref()
            .ok_or(VoiceError::Unauthorized)?;
        let Purpose::Confirmation {
            parent: parent_id,
            request,
        } = self.row.key.purpose
        else {
            return Err(VoiceError::Unauthorized);
        };
        let role = action.protected_row()?;
        let binding = action.protected_binding()?;
        if !(1..10).contains(&role) {
            return Err(VoiceError::Unauthorized);
        }
        let index = self.ledger.index.lock().unwrap_or_else(|e| e.into_inner());
        if parent.id != parent_id
            || !index
                .rows
                .iter()
                .flatten()
                .any(|row| Arc::ptr_eq(row, parent))
            || !index
                .rows
                .iter()
                .flatten()
                .any(|row| Arc::ptr_eq(row, &self.row))
        {
            return Err(VoiceError::Unauthorized);
        }
        let parent_state = parent.state.lock().unwrap_or_else(|e| e.into_inner());
        if !parent_state
            .plan()
            .is_some_and(|plan| plan.control.is_some() && plan.identity.contains(action))
        {
            return Err(VoiceError::Unauthorized);
        }
        // Root's protected history uses three candidate lanes, each Hello/Proof/Finish.
        // This is an explicit native adapter contract: roles10/11 remain unsupported.
        let column = (role - 1) % 3 + 1;
        let expected_operation = match (binding.local_is_low, column) {
            (true, 1 | 3) | (false, 2) => CryptoOperation::EncryptConfirmation,
            _ => CryptoOperation::DecryptConfirmation,
        };
        if action.operation() != expected_operation {
            return Err(VoiceError::Unauthorized);
        }
        let mut expected_role = None;
        match request {
            ConfirmationPurpose::Start(predecessor) => {
                if !binding.local_is_low || column != 1 {
                    return Err(VoiceError::Unauthorized);
                }
                if predecessor.is_none() {
                    expected_role = Some(1);
                }
                for row in index.rows.iter().flatten().filter(|row| {
                    !Arc::ptr_eq(row, &self.row)
                        && row
                            .key
                            .parent
                            .as_ref()
                            .is_some_and(|owner| Arc::ptr_eq(owner, parent))
                }) {
                    let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(metadata) = state.child {
                        // Failed/unsealed prior Hello is valid: its original reservation
                        // id, not a Sealed receipt, names the explicit next candidate.
                        if predecessor.is_some()
                            && metadata.message == predecessor
                            && [1, 4].contains(&metadata.role)
                        {
                            expected_role = Some(metadata.role + 3);
                        }
                    }
                }
            }
            ConfirmationPurpose::Respond(trigger) => {
                let row = index
                    .rows
                    .iter()
                    .flatten()
                    .find(|row| row.id == trigger)
                    .filter(|row| {
                        row.key
                            .parent
                            .as_ref()
                            .is_some_and(|owner| Arc::ptr_eq(owner, parent))
                    })
                    .ok_or(VoiceError::Unauthorized)?;
                let state = row.state.lock().unwrap_or_else(|e| e.into_inner());
                let metadata = state.child.ok_or(VoiceError::Unauthorized)?;
                let incoming_column = (metadata.role - 1) % 3 + 1;
                // High Finish has no response. A mere role+1 bound would wrongly adopt
                // another candidate as the response to that terminal incoming flight.
                if !matches!(
                    (binding.local_is_low, incoming_column),
                    (true, 2) | (false, 1)
                ) {
                    return Err(VoiceError::Unauthorized);
                }
                expected_role = Some(metadata.role + 1);
            }
            ConfirmationPurpose::Incoming(expected) => {
                if expected_operation != CryptoOperation::DecryptConfirmation {
                    return Err(VoiceError::Unauthorized);
                }
                expected_role = Some(expected);
            }
        }
        if expected_role != Some(role) {
            return Err(VoiceError::RetryMismatch);
        }
        for other in index.rows.iter().flatten().filter(|row| {
            !Arc::ptr_eq(row, &self.row)
                && row
                    .key
                    .parent
                    .as_ref()
                    .is_some_and(|owner| Arc::ptr_eq(owner, parent))
        }) {
            let state = other.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.child.is_some_and(|metadata| metadata.role == role) {
                return Err(VoiceError::RetryMismatch);
            }
        }
        let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
        let original_metadata = state.child;
        if original_metadata.is_some_and(|original| original.role != role)
            || (!claim && original_metadata.is_none())
        {
            return Err(VoiceError::RetryMismatch);
        }
        let Body::Pending(pending) = &mut state.body else {
            return Err(VoiceError::Unauthorized);
        };
        if !pending.selected.lease().is_live()
            || !action.belongs_to(pending.selected.binding().scope, pending.selected.lease())
        {
            return Err(VoiceError::Unauthorized);
        }
        if let Some(original) = &pending.published {
            if !Arc::ptr_eq(original, action) {
                return Err(VoiceError::RetryMismatch);
            }
        } else if claim {
            pending.published = Some(action.clone());
        } else {
            return Err(VoiceError::Unauthorized);
        }
        if original_metadata.is_none() {
            state.child = Some(ChildMetadata {
                role,
                message: None,
            });
        }
        Ok(())
    }

    pub(super) fn require_child_action(
        &self,
        action: &Arc<CryptoAction>,
    ) -> Result<(), VoiceError> {
        self.child_action_fence(action, false)
    }

    /// Publication and role already belong to Pending before this fallible payload borrow.
    /// An End/settlement refusal retains that exact Action, never another Actor request.
    pub(super) fn describe_child(&self, action: &Arc<CryptoAction>) -> Result<(), VoiceError> {
        if action.operation() != CryptoOperation::EncryptConfirmation {
            return Ok(());
        }
        {
            let state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            if state
                .child
                .is_some_and(|metadata| metadata.message.is_some())
            {
                return Ok(());
            }
        }
        let input = action.input()?;
        let catcoms_sync::voice_v2::crypto_actions::CryptoInput::EncryptConfirmation(flight) =
            input.as_input()
        else {
            return Err(VoiceError::Unauthorized);
        };
        let message = flight.reservation.id;
        {
            let mut state = self.row.state.lock().unwrap_or_else(|e| e.into_inner());
            let Body::Pending(pending) = &state.body else {
                return Err(VoiceError::Unauthorized);
            };
            if !pending
                .published
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, action))
            {
                return Err(VoiceError::Unauthorized);
            }
            let metadata = state.child.as_mut().ok_or(VoiceError::Unauthorized)?;
            if metadata.message.is_some_and(|original| original != message) {
                return Err(VoiceError::RetryMismatch);
            }
            metadata.message = Some(message);
        }
        drop(input); // actual Root input custody leaves only after all native guards
        Ok(())
    }

    pub(crate) fn attach_child(
        self,
        action: Arc<CryptoAction>,
        bytes: usize,
    ) -> Result<RowId, VoiceError> {
        self.mark_handoff()?;
        self.child_action_fence(&action, true)?;
        self.attach_published(bytes)
    }
}
