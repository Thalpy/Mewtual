//! Test-only, bounded, non-creating observation. No Root/Actor query or owned row
//! survives this synchronous snapshot; contention is reported rather than waited on.
use super::*;

impl Ledger {
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
