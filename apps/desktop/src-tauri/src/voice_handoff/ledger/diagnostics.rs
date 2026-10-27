//! Test-only, bounded, non-creating observation. No Root/Actor query or owned row
//! survives this synchronous snapshot; contention is reported rather than waited on.
use super::*;

impl Ledger {
    /// Bounded failure oracle for unscripted multi-peer composition. Only fixed
    /// selector/state metadata is copied; never ask a possibly parked Actor or keep
    /// Row/Action/Grant owners alive while formatting the failure.
    pub(in crate::voice_handoff) fn fixture_coordinator_diagnostic(
        &self,
        selected: &SelectedHandoffOwner,
        peers: &[(catcoms_crypto::DeviceId, u8)],
    ) -> String {
        let observed = (|| {
            let index = self.index.try_lock().map_err(|_| "ledger contended")?;
            let mut rows = [None; ROWS];
            let mut used = 0;
            for row in index.rows.iter().flatten() {
                let Purpose::Derive(selector) = row.key.purpose else {
                    continue;
                };
                if !row.key.matches(selected, row.key.purpose) {
                    continue;
                }
                let peer = peers
                    .iter()
                    .find(|(id, _)| *id == selector.peer)
                    .map(|(_, label)| *label);
                let state = row.state.try_lock().ok().map(|state| {
                    let kind = match &state.body {
                        Body::Pending(_) => "pending",
                        Body::Live(_) => "live",
                        Body::Drained(..) => "drained",
                        Body::ClosingPending => "closing",
                        Body::ClosedPending => "closed",
                        _ => "other",
                    };
                    (
                        kind,
                        state.status.action.is_some(),
                        state.status.engine.settled,
                        state.status.acknowledged,
                        state.status.drained,
                        state.final_phase.is_some(),
                        state.final_delivery().map(|delivery| {
                            (
                                delivery.kind,
                                delivery.issued.is_some(),
                                delivery.acknowledged,
                            )
                        }),
                    )
                });
                rows[used] = Some((peer, selector.epoch, state));
                used += 1;
            }
            let mut media = [None; ROWS];
            for (slot, entry) in index.media.iter().enumerate() {
                if let Some((peer, collected)) = entry
                    .as_ref()
                    .and_then(|entry| entry.fixture_history_diagnostic(selected))
                {
                    let label = peers
                        .iter()
                        .find(|(id, _)| *id == peer)
                        .map(|(_, label)| *label);
                    media[slot] = Some((label, collected));
                }
            }
            Ok::<_, &'static str>((rows, media))
        })();
        format!("native (parent peer/epoch/cached_state/action/settled/ack/drained/final_phase/final_delivery, media peer/actual_history_collected)={observed:?}")
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
