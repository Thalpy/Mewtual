//! Bounded test observations, not authority or progress receipts. Never block a
//! producer or retain an Actor/Row/Grant while reporting a potentially stalled task.
//! Kind: exact/discover/local-confirm/incoming-confirm/ready/consumed =0..5.
//! Outcome: success/capacity/freshness/unauthorized/closed-or-stopped/other =0..5.
//! Stages: start1, peer-snapshot2, preflight3, UI-commit4, publication5,
//! finished6, mount7, parent-slot8, server-map9, unlocked10, selection11, begin12.
//! SDP boundaries (separate field): resolve1, recovery2, preflight3,
//! current/begin4, reserve5, current/attach6, busy-recovery7.
use super::*;
use crate::voice_handoff::factory::Refused as FactoryRefused;
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::VoiceError;

#[derive(Clone, Copy, Default)]
pub(in crate::voice_handoff) struct NativeDiagnostic {
    // The table is harness-owned, never part of the paid Session graph. The exact
    // authenticated logical parent distinguishes a newcomer from an indexed sibling.
    pub sdp_parent: [u8; 16],
    pub sdp_sequence: u64,
    pub sdp_stage: u8,
    pub sdp_refusal: Option<(u8, [u8; 16], u64, Option<VoiceError>)>,
    pub attempts: [u32; 6],
    pub refusals: [u32; 6],
    pub started: [u32; 6],
    pub finished: [u32; 6],
    pub maintenance: [u32; 5],
    pub kind: u8,
    pub stage: u8,
    pub maintenance_cursor: u8,
    pub attempt_cursor: u8,
    pub selector: Option<PairSelector>,
    pub finished_kind: u8,
    pub finished_stage: u8,
    pub finished_selector: Option<PairSelector>,
    pub last_error: Option<VoiceError>,
}
fn classification(error: Option<FactoryRefused>) -> (usize, Option<VoiceError>) {
    match error {
        None => (0, None),
        Some(FactoryRefused::Protocol(VoiceError::Capacity)) => (1, Some(VoiceError::Capacity)),
        Some(FactoryRefused::Protocol(VoiceError::Freshness)) => (2, Some(VoiceError::Freshness)),
        Some(FactoryRefused::Protocol(VoiceError::Unauthorized)) => {
            (3, Some(VoiceError::Unauthorized))
        }
        Some(FactoryRefused::Closed | FactoryRefused::Stopped) => (4, None),
        Some(FactoryRefused::Protocol(error)) => (5, Some(error)),
    }
}
// Explicitly prepared by the test harness before producers start. This fixed table
// belongs to the harness, not the paid application Session. Weak inert identities keep
// neither Session/keys nor actor/ledger/work owners alive; dead slots may be reused.
// Missing observations are acceptable. Contention/full capacity must never delay work.
struct Entry {
    identity: std::sync::Weak<SessionIdentity>,
    value: NativeDiagnostic,
}
type Table = Box<[Option<Entry>]>;
static TABLE: std::sync::OnceLock<Mutex<Table>> = std::sync::OnceLock::new();
static MISSED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub(in crate::voice_handoff) struct NativeObservation {
    flags: Option<(bool, bool, bool)>,
    diagnostic: Option<NativeDiagnostic>,
    missed: bool,
}
impl NativeObservation {
    pub(in crate::voice_handoff) fn sdp_refusal(
        self,
    ) -> Option<(u8, [u8; 16], Option<VoiceError>)> {
        self.diagnostic
            .and_then(|diagnostic| diagnostic.sdp_refusal)
            .map(|(stage, parent, _, error)| (stage, parent, error))
    }
    /// A past refusal, even for this same intent, is not evidence about a later
    /// physical retry. Missing/contended observations remain absence of evidence.
    pub(in crate::voice_handoff) fn sdp_refusal_at(
        self,
        sequence: u64,
    ) -> Option<(u8, [u8; 16], Option<VoiceError>)> {
        self.diagnostic
            .and_then(|diagnostic| diagnostic.sdp_refusal)
            .filter(|(_, _, actual, _)| *actual == sequence)
            .map(|(stage, parent, _, error)| (stage, parent, error))
    }
    pub(in crate::voice_handoff) fn describe_sdp(self) -> String {
        format!("closed/physical/native={:?}; sdp_refusal(stage,parent,physical_sequence,error)={:?}; diagnostic_present={}; global_missed={}; pool counts not sampled (blocking getters)",
            self.flags, self.diagnostic.and_then(|diagnostic| diagnostic.sdp_refusal), self.diagnostic.is_some(), self.missed)
    }
    pub(super) fn is_missing(self) -> bool {
        self.diagnostic.is_none()
    }
    pub(in crate::voice_handoff) fn describe(self, expected: PairSelector) -> String {
        let Some(diagnostic) = self.diagnostic else {
            return format!(
                "session flags={:?}; diagnostic absent/contended; global_missed={}",
                self.flags, self.missed
            );
        };
        let matches = |selector: Option<PairSelector>| {
            selector.map(|actual| {
                [
                    actual.peer == expected.peer,
                    actual.join == expected.join,
                    actual.key_ref == expected.key_ref,
                    actual.epoch == expected.epoch,
                ]
            })
        };
        format!("closed/physical/native={:?}; kind/stage={}/{}; last_maintenance_cursor={}; last_attempt_cursor={}; turns={:?}; spawn_attempts={:?}; refusals={:?}; started={:?}; finished={:?}; last_kind/stage={}/{}; last_error={:?}; selector peer/join/key/epoch={:?}; last_selector={:?}; sdp_stage={}; sdp_last_refusal(stage,parent,physical_sequence,error)={:?}; global_missed={}; refusals before spawn not observed",
            self.flags, diagnostic.kind, diagnostic.stage, diagnostic.maintenance_cursor, diagnostic.attempt_cursor,
            diagnostic.maintenance, diagnostic.attempts, diagnostic.refusals, diagnostic.started, diagnostic.finished,
            diagnostic.finished_kind, diagnostic.finished_stage, diagnostic.last_error,
            matches(diagnostic.selector), matches(diagnostic.finished_selector), diagnostic.sdp_stage, diagnostic.sdp_refusal, self.missed)
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    #[test]
    fn a_previous_physical_refusal_cannot_prove_a_later_ui_attempt() {
        let parent = [9; 16];
        let observation = NativeObservation {
            flags: None,
            diagnostic: Some(NativeDiagnostic {
                sdp_refusal: Some((4, parent, 7, Some(VoiceError::Capacity))),
                ..NativeDiagnostic::default()
            }),
            missed: false,
        };
        assert_eq!(observation.sdp_refusal_at(6), None);
        assert_eq!(observation.sdp_refusal_at(8), None);
        assert_eq!(
            observation.sdp_refusal_at(7),
            Some((4, parent, Some(VoiceError::Capacity)))
        );
    }
}
impl Session {
    pub(in crate::voice_handoff) fn fixture_sdp_stage(
        &self,
        parent: [u8; 16],
        sequence: u64,
        stage: u8,
    ) {
        self.fixture_update_native(|diagnostic| {
            diagnostic.sdp_parent = parent;
            diagnostic.sdp_sequence = sequence;
            diagnostic.sdp_stage = stage;
        });
    }
    pub(in crate::voice_handoff) fn fixture_sdp_finished(&self, error: Option<FactoryRefused>) {
        let Some(error) = error else { return };
        // Even a pool snapshot could wait behind a stalled owner. Keep diagnostic
        // writes nonblocking and do not delay the original authenticated refusal.
        self.fixture_update_native(|diagnostic| {
            diagnostic.sdp_refusal = Some((
                diagnostic.sdp_stage,
                diagnostic.sdp_parent,
                diagnostic.sdp_sequence,
                classification(Some(error)).1,
            ));
        });
    }
    pub(in crate::voice_handoff) fn fixture_prepare_native_diagnostics() {
        TABLE.get_or_init(|| {
            Mutex::new(
                (0..512)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )
        });
    }
    fn fixture_update_native(&self, change: impl FnOnce(&mut NativeDiagnostic)) {
        let identity = Arc::downgrade(&self.identity);
        let Some(table) = TABLE.get() else {
            MISSED.store(true, Ordering::Relaxed);
            return;
        };
        let Ok(mut table) = table.try_lock() else {
            MISSED.store(true, Ordering::Relaxed);
            return;
        };
        let position = table
            .iter()
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.identity.ptr_eq(&identity))
            })
            .or_else(|| {
                table.iter().position(|entry| {
                    entry
                        .as_ref()
                        .is_none_or(|entry| entry.identity.strong_count() == 0)
                })
            });
        let Some(position) = position else {
            MISSED.store(true, Ordering::Relaxed);
            return;
        };
        let entry = table[position].get_or_insert_with(|| Entry {
            identity: identity.clone(),
            value: NativeDiagnostic::default(),
        });
        if !entry.identity.ptr_eq(&identity) {
            *entry = Entry {
                identity,
                value: NativeDiagnostic::default(),
            };
        }
        change(&mut entry.value);
    }
    pub(in crate::voice_handoff) fn fixture_native_snapshot(&self) -> NativeObservation {
        let flags = self
            .counters
            .try_lock()
            .ok()
            .map(|state| (state.closed, state.busy, state.native_busy));
        let identity = Arc::downgrade(&self.identity);
        let diagnostic = TABLE
            .get()
            .and_then(|table| table.try_lock().ok())
            .and_then(|table| {
                table
                    .iter()
                    .flatten()
                    .find(|entry| entry.identity.ptr_eq(&identity))
                    .map(|entry| entry.value)
            });
        NativeObservation {
            flags,
            diagnostic,
            missed: MISSED.load(Ordering::Relaxed),
        }
    }
    pub(super) fn fixture_hold_diagnostic_table() -> impl Drop {
        TABLE
            .get()
            .expect("harness prepared diagnostics")
            .lock()
            .unwrap()
    }
    pub(in crate::voice_handoff) fn fixture_maintenance_turn(&self, stage: u8, cursor: u8) {
        self.fixture_update_native(|diagnostic| {
            let count = &mut diagnostic.maintenance[usize::from(stage)];
            *count = count.saturating_add(1);
            diagnostic.maintenance_cursor = cursor;
        });
    }
    pub(in crate::voice_handoff) fn fixture_native_admission(
        &self,
        kind: u8,
        cursor: u8,
        error: Option<FactoryRefused>,
    ) {
        self.fixture_update_native(|diagnostic| {
            let count = &mut diagnostic.attempts[usize::from(kind)];
            *count = count.saturating_add(1);
            if error.is_some() {
                let count = &mut diagnostic.refusals[classification(error).0];
                *count = count.saturating_add(1);
            }
            diagnostic.attempt_cursor = cursor;
        });
    }
    pub(in crate::voice_handoff) fn fixture_native_started(&self, kind: u8) {
        self.fixture_update_native(|diagnostic| {
            let count = &mut diagnostic.started[usize::from(kind)];
            *count = count.saturating_add(1);
            diagnostic.kind = kind;
            diagnostic.stage = 1;
            diagnostic.selector = None;
        });
    }
    pub(in crate::voice_handoff) fn fixture_native_stage(&self, stage: u8) {
        self.fixture_update_native(|diagnostic| diagnostic.stage = stage);
    }
    pub(in crate::voice_handoff) fn fixture_native_selector(&self, selector: PairSelector) {
        self.fixture_update_native(|diagnostic| diagnostic.selector = Some(selector));
    }
    pub(in crate::voice_handoff) fn fixture_native_finished(&self, error: Option<FactoryRefused>) {
        self.fixture_update_native(|diagnostic| {
            let (class, error) = classification(error);
            let count = &mut diagnostic.finished[class];
            *count = count.saturating_add(1);
            diagnostic.last_error = error;
            diagnostic.finished_kind = diagnostic.kind;
            diagnostic.finished_stage = diagnostic.stage;
            diagnostic.finished_selector = diagnostic.selector;
            diagnostic.stage = 6;
        });
    }
}
