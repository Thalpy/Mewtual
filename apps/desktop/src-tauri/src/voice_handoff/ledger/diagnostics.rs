//! Test-only, bounded, non-creating observation. No Root/Actor query or owned row
//! survives this synchronous snapshot; contention is reported rather than waited on.
use super::*;

/// Harness-owned scalar snapshot. It lends no Action/Row/Grant and never uses a
/// lossy global observation to classify the actual worker being recovered.
pub(in crate::voice_handoff) struct RecoverySnapshot {
    pub attached: bool,
    pub action: Option<crate::voice_handoff::factory::native::fixture::ActionIdentity>,
    pub prepared: bool,
}
impl ResolvedRow {
    pub(in crate::voice_handoff) fn fixture_recovery_snapshot(
        &self,
        selected: &SelectedHandoffOwner,
        selector: PairSelector,
    ) -> Result<RecoverySnapshot, VoiceError> {
        let index = self
            .ledger
            .index
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        if !index.rows[self.row.id.slot].as_ref().is_some_and(|row| {
            Arc::ptr_eq(row, &self.row)
                && row.id == self.row.id
                && row.key.matches(selected, Purpose::Derive(selector))
        }) {
            return Err(VoiceError::Unauthorized);
        }
        let state = self
            .row
            .state
            .try_lock()
            .map_err(|_| VoiceError::Capacity)?;
        let (attached, action, prepared) = match &state.body {
            Body::Pending(pending) if !pending.worker && pending.handed_off => {
                (false, pending.published.clone(), pending.prepared.is_some())
            }
            Body::Live(live) if state.status.pending == PendingStatus::NotPending => {
                (true, Some(live.action.clone()), false)
            }
            _ => return Err(VoiceError::Unauthorized),
        };
        drop(state);
        drop(index);
        // Immutable Action observations and their final Arc drop happen outside
        // Index/State. This temporary belongs to the bounded harness frame.
        let action = action.map(|action| {
            assert!(
                action.is_pending(),
                "recovery must not spend original engine authority"
            );
            crate::voice_handoff::factory::native::fixture::ActionIdentity::of(&action)
        });
        Ok(RecoverySnapshot {
            attached,
            action,
            prepared,
        })
    }
}

impl Ledger {
    /// Map only an independently discovered exact counterpart to its retained
    /// original row for diagnostics. No row is returned to a browser/factory.
    pub(in crate::voice_handoff) fn fixture_discovered_parent(
        &self,
        selected: &SelectedHandoffOwner,
        selector: PairSelector,
    ) -> Option<RowId> {
        let index = self.index.try_lock().ok()?;
        index
            .rows
            .iter()
            .flatten()
            .find(|row| row.key.matches(selected, Purpose::Derive(selector)))
            .map(|row| row.id)
    }

    /// Fixed aggregate confirmation state for the SAME native parent. Never call
    /// ordinary status/engine getters: a blocked callback may hold those locks.
    pub(in crate::voice_handoff) fn fixture_confirmation_diagnostic(
        &self,
        selected: &SelectedHandoffOwner,
        expected: Option<RowId>,
    ) -> String {
        let observed: Result<_, &'static str> = (|| {
            let id = expected.ok_or("no original parent")?;
            let index = self
                .index
                .try_lock()
                .map_err(|_| "ledger index contended")?;
            let parent = index
                .rows
                .iter()
                .flatten()
                .find(|row| row.id == id && row.key.matches(selected, row.key.purpose))
                .ok_or("original parent absent")?;
            let mut totals = [0_u8; 8];
            let mut derive_deadline = None;
            for row in index.rows.iter().flatten() {
                // SDP rows share this SAME parent Arc too. Their independent
                // result/ACK/Return must not inflate confirmation progress.
                let parent_row = Arc::ptr_eq(row, parent);
                let confirmation_child = matches!(row.key.purpose,
                    Purpose::Confirmation { parent: original, .. } if original == id)
                    && row
                        .key
                        .parent
                        .as_ref()
                        .is_some_and(|original| Arc::ptr_eq(original, parent));
                if !row.key.matches(selected, row.key.purpose)
                    || (!parent_row && !confirmation_child)
                {
                    continue;
                }
                totals[0] += 1;
                let Ok(state) = row.state.try_lock() else {
                    totals[1] += 1;
                    continue;
                };
                totals[2] += u8::from(state.status.action.is_some());
                totals[3] += u8::from(state.status.engine.receipt.is_some());
                totals[4] += u8::from(state.status.engine.settled);
                totals[5] += u8::from(state.status.acknowledged);
                totals[6] += u8::from(state.status.drained);
                totals[7] += u8::from(matches!(state.body, Body::Pending(_)));
                if parent_row {
                    if let Body::Live(live) = &state.body {
                        // Immutable scalar only: this accessor takes no Engine/Root
                        // lock and invokes no Clock. In particular, do not diagnose
                        // an expired original by asking is_pending() under State.
                        derive_deadline = live
                            .action
                            .protected_binding()
                            .ok()
                            .map(|binding| binding.deadline);
                    }
                }
            }
            Ok((totals, derive_deadline))
        })();
        match observed {
            Ok((totals, derive_deadline)) => format!("original confirmation rows/contended/actions/receipts/settled/acked/drained/pending={totals:?}; original Derive monotonic deadline={derive_deadline:?}"),
            Err(reason) => reason.into(),
        }
    }

    /// Original selection and full RowId only. Refuse equal-label replacement owners;
    /// diagnostics must not resolve a new parent or wait behind a stalled worker.
    pub(in crate::voice_handoff) fn fixture_sdp_diagnostic(
        &self,
        selected: &SelectedHandoffOwner,
        expected: Option<RowId>,
    ) -> (Option<PairSelector>, String) {
        let observed: Result<_, &'static str> = (|| {
            let id = expected.ok_or("no expected original parent")?;
            let index = self
                .index
                .try_lock()
                .map_err(|_| "ledger index contended")?;
            let row = index
                .rows
                .iter()
                .flatten()
                .find(|row| row.id == id && row.key.matches(selected, row.key.purpose))
                .ok_or("original parent absent")?;
            let Purpose::Derive(selector) = row.key.purpose else {
                return Err("not a parent");
            };
            let state = row.state.try_lock().map_err(|_| "parent row contended")?;
            let plan = state.plan().ok_or("parent has no plan")?;
            let key = plan
                .transition
                .as_ref()
                .and_then(|claim| claim.key.as_ref())
                .ok_or("parent has no key")?;
            let storage = key.sdp.as_ref().map(|sdp| {
                (
                    sdp.closed,
                    sdp.worker(),
                    sdp.phase.is_some(),
                    sdp.sdk_preparing,
                    sdp.sdk.as_ref().map(|sdk| sdk.released()),
                    sdp.phase.as_ref().map(|phase| phase.try_status()),
                )
            });
            Ok((
                selector,
                (
                    plan.local_is_low,
                    key.ready as u8,
                    key.released,
                    state.final_delivery().is_some(),
                    state.final_phase.is_some(),
                ),
                storage,
            ))
        })();
        match observed {
            Ok((selector, flags, storage)) => (Some(selector), format!("original parent LOW/Ready/key_released/final_delivery/final_phase={flags:?}; storage closed/worker/phase/sdk_preparing/sdk_released/phase_status={storage:?}")),
            Err(reason) => (None, reason.into()),
        }
    }
    pub(in crate::voice_handoff) fn fixture_derivation_diagnostic(
        &self,
        selected: &SelectedHandoffOwner,
        expected: PairSelector,
    ) -> String {
        let Ok(index) = self.index.try_lock() else {
            return "ledger contended".into();
        };
        let mut parents = 0_u8;
        let mut matches = [0_u8; 4];
        let mut exact = None;
        for row in index.rows.iter().flatten() {
            let Purpose::Derive(actual) = row.key.purpose else {
                continue;
            };
            if !row.key.matches(selected, row.key.purpose) {
                continue;
            }
            parents += 1;
            for (count, matched) in matches.iter_mut().zip([
                actual.peer == expected.peer,
                actual.join == expected.join,
                actual.key_ref == expected.key_ref,
                actual.epoch == expected.epoch,
            ]) {
                *count += u8::from(matched);
            }
            if actual != expected {
                continue;
            }
            exact = Some(match row.state.try_lock() {
                Err(_) => ("contended", false, false, false, false, false),
                Ok(state) => {
                    let attached = state.status.action.is_some();
                    match &state.body {
                        Body::Pending(pending) => (
                            "pending",
                            attached,
                            pending.worker,
                            pending.handed_off,
                            pending.prepared.is_some(),
                            pending.published.is_some(),
                        ),
                        Body::ClosingPending => ("closing", attached, false, false, false, false),
                        Body::ClosedPending => ("closed", attached, false, false, false, false),
                        Body::Live(live) => ("live", attached, live.worker, true, false, true),
                        Body::Drained(..) => ("drained", attached, false, true, false, false),
                        _ => ("unexpected", attached, false, false, false, false),
                    }
                }
            });
        }
        drop(index); // Formatting/allocator work cannot hold the producer's index.
        format!("parents={parents}; matching peer/join/key/epoch={matches:?}; exact state/action/worker/handoff/prepared/publication={exact:?}")
    }
}
