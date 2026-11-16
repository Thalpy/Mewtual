//! Native scheduling hints from actual retained rows. These observations create neither
//! an action nor a new attempt: publication still re-enters the original parent/child
//! fences under the UI commit, and Root checks the exact live call and protocol state.
use super::*;
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;

/// Retain the actual indexed parent and, when needed, the actual known child. Public
/// RowIds and renderer role hints cannot construct this original-allocation observation.
pub(in crate::voice_handoff) struct Continuation {
    pub parent: ResolvedRow,
    pub request: ConfirmationPurpose,
    pub known: Option<ResolvedRow>,
    pub incoming: Option<Arc<CryptoAction>>,
}

#[cfg(test)]
impl ResolvedRow {
    /// Passive native oracle: no issuance, ACK, grant, side effect or renewed authority.
    pub(in crate::voice_handoff) fn retired_plan_test_observation(
        &self,
        selected: &SelectedHandoffOwner,
    ) -> Result<bool, VoiceError> {
        let index = self.indexed()?;
        Ledger::certified_retired_plan(&index, selected, &self.row)
    }
}

impl Ledger {
    /// A complete original Plan certificate closes this subtree permanently. This is
    /// only an outstanding-work/admission filter, never browser advancement authority or
    /// permission to remove history. Call under Index; direct Control recovery remains
    /// available. A local Pending close, ordinary Return or Phase final cannot qualify.
    pub(super) fn certified_retired_plan(
        index: &Index,
        selected: &SelectedHandoffOwner,
        row: &Arc<Row>,
    ) -> Result<bool, VoiceError> {
        if row.key.parent.is_some()
            || !matches!(row.key.purpose, Purpose::Derive(_))
            || !row.key.matches(selected, row.key.purpose)
            || !index
                .rows
                .iter()
                .flatten()
                .any(|actual| Arc::ptr_eq(actual, row))
        {
            return Ok(false);
        }
        let state = super::transition::try_lock(&row.state)?;
        let retired = matches!(state.body, Body::Drained(..))
            && state.status.action.is_some()
            && state.status.drained
            && state.status.engine.settled
            && state.final_phase.is_none()
            && state.final_delivery().is_some_and(|delivery| {
                delivery.kind == FinalKind::Plan
                    && delivery.issued.is_some()
                    && delivery.acknowledged
            })
            && state
                .plan()
                .is_some_and(|plan| plan.control.is_none() && !plan.can_adopt_child());
        // Issued Plan proof is the irreversible full-retirement witness established by
        // issue_final, not a renderer claim. No later native admission may reopen it.
        // Do NOT re-sample boolean Root Phase observers here: transient try-lock pressure
        // maps to false there and would expose an old terminal descriptor to a fresh pump,
        // permanently pinning it. Historical proof/ACK stays valid under that pressure.
        if !retired {
            return Ok(false);
        }
        // Same Index -> parent -> child order and closure contract as Plan final. This
        // automatic scheduling query must refuse Capacity rather than wait on a child.
        // At most32 children are visited; no allocation, descriptor queue or destruction.
        for child in index.rows.iter().flatten().filter(|child| {
            child
                .key
                .parent
                .as_ref()
                .is_some_and(|parent| Arc::ptr_eq(parent, row))
        }) {
            let child_state = super::transition::try_lock(&child.state)?;
            if !sdp_rows::locally_drained(&child_state).unwrap_or_else(|| {
                matches!(child_state.body, Body::ClosedPending)
                    || (matches!(child_state.body, Body::Drained(..)) && child_state.status.drained)
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The current browser coordinator can advance only one original negotiation at a
    /// time. Do not automatically prepay another parent/Root pair while an uncertified
    /// subtree is registered: its unused preparation can consume the current Ready budget.
    /// Exact Plan-certified retired rows may remain paid until End without blocking a
    /// different selector. This neither forgets them nor renews their old authority.
    /// This is only a scheduling observation. Final new-parent admission repeats it under
    /// the same UI commit as begin; exact known Pending/history recovery bypasses this gate.
    pub(in crate::voice_handoff) fn automatic_parent_available(
        &self,
        selected: &SelectedHandoffOwner,
    ) -> Result<bool, VoiceError> {
        let index = super::transition::try_lock(&self.index)?;
        // A collector unlinks before dropping real owners outside Index. Absence during
        // that ticket is not actual history collection and cannot admit a successor early.
        if self.collecting.load(Ordering::Acquire) {
            return Err(VoiceError::Capacity);
        }
        for row in index.rows.iter().flatten() {
            if matches!(row.key.purpose, Purpose::Derive(_))
                && row.key.matches(selected, row.key.purpose)
                && !Self::certified_retired_plan(&index, selected, row)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Rotate through the finite original table. Pending work wins at its own slot,
    /// including after proof expiry; an uncertain Incoming without an actual publication
    /// must instead be rediscovered from Root's paid snapshot, never from its role alone.
    pub(in crate::voice_handoff) fn confirmation_continuation(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        cursor: &mut u8,
    ) -> Option<Continuation> {
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        for offset in 0..ROWS {
            let slot = (usize::from(*cursor) + offset) % ROWS;
            let Some(row) = &index.rows[slot] else {
                continue;
            };
            if !row.key.matches(selected, row.key.purpose) {
                continue;
            }
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            let candidate = match row.key.purpose {
                Purpose::Sdp { .. } => continue, // Private v8 work never re-enters confirmation factories.
                Purpose::Confirmation { request, .. } => {
                    let Body::Pending(pending) = &state.body else {
                        continue;
                    };
                    if pending.worker
                        || (matches!(request, ConfirmationPurpose::Incoming(_))
                            && pending.published.is_none())
                    {
                        continue;
                    }
                    let parent = row.key.parent.as_ref()?;
                    // Existing publication can resume without a rotating snapshot. Retain
                    // the exact child too, so equal labels from another Index cannot win.
                    Some(Continuation {
                        parent: ResolvedRow {
                            ledger: self.clone(),
                            row: parent.clone(),
                        },
                        request,
                        known: matches!(request, ConfirmationPurpose::Incoming(_)).then(|| {
                            ResolvedRow {
                                ledger: self.clone(),
                                row: row.clone(),
                            }
                        }),
                        incoming: None,
                    })
                }
                Purpose::Derive(_) => {
                    if !state.status.drained
                        || !state.status.acknowledged
                        || !state.status.engine.settled
                        || !state
                            .status
                            .engine
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| receipt.outcome == CryptoOutcome::Derived)
                        || state.final_phase.is_some()
                        || state.final_delivery().is_some()
                    {
                        continue;
                    }
                    let Some(plan) = state.plan().filter(|plan| plan.can_adopt_child()) else {
                        continue;
                    };
                    let mut request = None;
                    // At most32 rows are scanned for each parent. No Root/Clock callback,
                    // payload borrow, resource charge or affine destruction occurs here.
                    for child in index.rows.iter().flatten().filter(|child| {
                        child
                            .key
                            .parent
                            .as_ref()
                            .is_some_and(|parent| Arc::ptr_eq(parent, row))
                    }) {
                        let child_state = child
                            .state
                            .lock()
                            .unwrap_or_else(|error| error.into_inner());
                        let Some(metadata) = child_state.child else {
                            continue;
                        };
                        if !child_state.status.drained
                            || !child_state.status.acknowledged
                            || !child_state.status.engine.settled
                            || !child_state
                                .status
                                .engine
                                .receipt
                                .as_ref()
                                .is_some_and(|receipt| receipt.outcome == CryptoOutcome::Opened)
                            || !matches!(
                                (plan.local_is_low, metadata.role),
                                (false, 1 | 4 | 7) | (true, 2 | 5 | 8)
                            )
                        {
                            continue;
                        }
                        let response = ConfirmationPurpose::Respond(child.id);
                        if !index.rows.iter().flatten().any(|other| {
                            other.key.matches(
                                selected,
                                Purpose::Confirmation {
                                    parent: row.id,
                                    request: response,
                                },
                            )
                        }) {
                            request = Some(response);
                            break;
                        }
                    }
                    if request.is_none() && plan.local_is_low {
                        let start = ConfirmationPurpose::Start(None);
                        if !index.rows.iter().flatten().any(|other| {
                            other.key.matches(
                                selected,
                                Purpose::Confirmation {
                                    parent: row.id,
                                    request: start,
                                },
                            )
                        }) {
                            request = Some(start);
                        }
                    }
                    request.map(|request| Continuation {
                        parent: ResolvedRow {
                            ledger: self.clone(),
                            row: row.clone(),
                        },
                        request,
                        known: None,
                        incoming: None,
                    })
                }
            };
            if candidate.is_some() {
                *cursor = ((slot + 1) % ROWS) as u8;
                return candidate;
            }
        }
        None
    }

    /// Select only a real authenticated Root action contained by an original plan. A
    /// snapshot is non-dequeueing; skip already attached/closed children and leave its
    /// native bytes and attempt budgets untouched. The caller drops the paid snapshot
    /// before final publication; this one action retains its own original Root payment.
    pub(in crate::voice_handoff) fn incoming_confirmation(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        action: &Arc<CryptoAction>,
    ) -> Option<Continuation> {
        if action.operation() != CryptoOperation::DecryptConfirmation {
            return None;
        }
        let role = action.protected_row().ok()?;
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        for row in
            index.rows.iter().flatten().filter(|row| {
                row.key.parent.is_none() && row.key.matches(selected, row.key.purpose)
            })
        {
            let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
            if !state.status.drained
                || state.final_phase.is_some()
                || state.final_delivery().is_some()
                || !state
                    .plan()
                    .is_some_and(|plan| plan.can_adopt_child() && plan.identity.contains(action))
            {
                continue;
            }
            let request = ConfirmationPurpose::Incoming(role);
            if let Some(child) = index.rows.iter().flatten().find(|child| {
                child.key.matches(
                    selected,
                    Purpose::Confirmation {
                        parent: row.id,
                        request,
                    },
                )
            }) {
                let child_state = child
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if !matches!(&child_state.body, Body::Pending(pending) if !pending.worker) {
                    continue;
                }
            }
            return Some(Continuation {
                parent: ResolvedRow {
                    ledger: self.clone(),
                    row: row.clone(),
                },
                request,
                known: None,
                incoming: Some(action.clone()),
            });
        }
        None
    }
}
