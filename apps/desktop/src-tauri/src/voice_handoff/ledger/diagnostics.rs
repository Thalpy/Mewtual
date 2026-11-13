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
            // Missing Claim/key is exactly the boundary a stalled handoff needs
            // to diagnose. Keep the actual original selector and parent/child
            // history instead of discarding them at that absence. Every value
            // below is a fixed flag/role; never print receipts, keys or payloads.
            let plan = state.plan();
            let claim = plan.and_then(|plan| plan.transition.as_ref());
            let key = claim.and_then(|claim| claim.key.as_ref());
            let parent = (
                state.status.action.is_some(),
                state.status.engine.settled,
                state.status.acknowledged,
                state.status.drained,
                state.final_delivery().is_some(),
                state.final_phase.is_some(),
            );
            let custody = (
                plan.map(|plan| plan.local_is_low),
                claim.map(|claim| claim.released),
                key.map(|key| (key.ready as u8, key.released)),
            );
            let storage = key.and_then(|key| key.sdp.as_ref()).map(|sdp| {
                (
                    sdp.closed,
                    sdp.worker(),
                    sdp.phase.is_some(),
                    sdp.sdk_preparing,
                    sdp.sdk.as_ref().map(|sdk| sdk.released()),
                    sdp.phase.as_ref().map(|phase| phase.try_status()),
                )
            });
            drop(state);
            // The finite native index has ROWS slots. No new owners, query,
            // allocation or blocking lock is created by this snapshot.
            let mut children = [None; ROWS];
            for (slot, child) in index.rows.iter().enumerate() {
                let Some(child) = child else { continue };
                let Purpose::Confirmation { parent, request } = child.key.purpose else {
                    continue;
                };
                if parent != id || !child.key.matches(selected, child.key.purpose) {
                    continue;
                }
                let role = match request {
                    ConfirmationPurpose::Start(_) => "start",
                    ConfirmationPurpose::Respond(_) => "respond",
                    ConfirmationPurpose::Incoming(_) => "incoming",
                };
                let flags = child.state.try_lock().ok().map(|state| {
                    (
                        state.child.map(|metadata| metadata.role),
                        state.status.action.is_some(),
                        state.status.engine.settled,
                        state.status.engine.receipt.as_ref().map(|receipt| {
                            !matches!(
                                receipt.outcome,
                                catcoms_sync::voice_v2::crypto_actions::CryptoOutcome::Rejected(_)
                            )
                        }),
                        state.status.acknowledged,
                        state.status.drained,
                    )
                });
                children[slot] = Some((role, flags));
            }
            Ok((selector, parent, custody, storage, children))
        })();
        match observed {
            Ok((selector, parent, custody, storage, children)) => (Some(selector), format!("original parent action/settled/ack/drained/final_delivery/final_phase={parent:?}; custody LOW/claim_released/key_Ready_released={custody:?}; storage closed/worker/phase/sdk_preparing/sdk_released/phase_status={storage:?}; children request/role_action_settled_accepted_ack_drained={children:?}")),
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
