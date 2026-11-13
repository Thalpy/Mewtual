//! Test-only, bounded, non-creating observation. No Root/Actor query or owned row
//! survives this synchronous snapshot; contention is reported rather than waited on.
use super::*;

impl Ledger {
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
        let mut original_parent = None;
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
            original_parent = Some(row.id);
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
        // Copy a fixed bounded metadata snapshot. Never retain rows, payloads,
        // receipts, action owners or a producer lock while formatting diagnostics.
        let mut children = [None; ROWS];
        for (slot, row) in index.rows.iter().enumerate() {
            let Some(row) = row else { continue };
            let Purpose::Confirmation { parent, request } = row.key.purpose else {
                continue;
            };
            if Some(parent) != original_parent || !row.key.matches(selected, row.key.purpose) {
                continue;
            }
            let role = match request {
                ConfirmationPurpose::Start(_) => 0_u16,
                ConfirmationPurpose::Respond(_) => 1_u16,
                ConfirmationPurpose::Incoming(kind) => 2 + u16::from(kind),
            };
            children[slot] = Some(match row.state.try_lock() {
                Err(_) => (role, "contended", false, false, false, None),
                Ok(state) => {
                    let outcome = state.status.engine.receipt.as_ref().map(|receipt| {
                        use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
                        match receipt.outcome {
                            CryptoOutcome::Derived => "derived",
                            CryptoOutcome::Opened => "opened",
                            CryptoOutcome::Sealed { .. } => "sealed",
                            CryptoOutcome::Rejected(_) => "rejected",
                        }
                    });
                    (
                        role,
                        if state.status.action.is_some() {
                            "attached"
                        } else {
                            "unattached"
                        },
                        state.status.acknowledged,
                        state.status.drained,
                        state.status.engine.settled,
                        outcome,
                    )
                }
            });
        }
        drop(index); // Formatting/allocator work cannot hold the producer's index.
        format!("parents={parents}; matching peer/join/key/epoch={matches:?}; exact state/action/worker/handoff/prepared/publication={exact:?}; children role/attachment/ack/drained/settled/outcome={children:?}")
    }
}
