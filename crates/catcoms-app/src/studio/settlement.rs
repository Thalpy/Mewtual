//! Bounded settlement invalidations, not a second durable state machine. Notifications name
//! facts observed under the actual store custody. Consumers reread recovery/source metadata;
//! an Open notification never says that the open epoch's edits have been receipted.
use super::*;
use std::collections::VecDeque;

/// Source phase and recovery-rail observations are independent. For example an Open source
/// can also have RecoveryAvailable; the latter must not be interpreted as an edit lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioSettlementState {
    Open,
    Closing,
    Settled,
    Fault,
    RecoveryAvailable,
    RecoveryEvictionPending,
    /// An attempted transition may have crossed an earlier durable barrier before failing.
    /// No phase or cause is inferred from an error string: reread before displaying a label.
    RefreshRequired,
}
impl From<EpochPhase> for StudioSettlementState {
    fn from(value: EpochPhase) -> Self {
        match value {
            EpochPhase::Open => Self::Open,
            EpochPhase::Closing => Self::Closing,
            EpochPhase::Settled => Self::Settled,
            EpochPhase::Fault => Self::Fault,
        }
    }
}

/// A worker turn produces only a handful of observations. Keep a hard rail anyway so future
/// producer additions cannot turn this diagnostic path into an unbounded queue. Coalescing
/// loses intermediate labels, never content: every event requires reading present state.
#[derive(Default)]
pub(crate) struct SettlementNotices(VecDeque<(StudioTarget, StudioSettlementState)>);
impl SettlementNotices {
    pub(crate) fn note(&mut self, target: StudioTarget, state: StudioSettlementState) {
        if self.0.contains(&(target, state)) {
            return;
        }
        if self.0.len() == 16 {
            self.0.pop_front();
        }
        self.0.push_back((target, state));
    }
    pub(crate) fn take(&mut self) -> Vec<(StudioTarget, StudioSettlementState)> {
        self.0.drain(..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settlement_observations_are_bounded_coalesced_and_not_finality() {
        let mut notices = SettlementNotices::default();
        let target = StudioTarget::Index { channel: [1; 16] };
        notices.note(target, EpochPhase::Open.into());
        notices.note(target, EpochPhase::Open.into());
        notices.note(target, StudioSettlementState::RecoveryAvailable);
        assert_eq!(notices.take().len(), 2);
        assert!(notices.take().is_empty());
        for n in 0..32 {
            notices.note(
                StudioTarget::Index { channel: [n; 16] },
                EpochPhase::Closing.into(),
            );
        }
        let held = notices.take();
        assert_eq!(held.len(), 16);
        assert_eq!(held[0].0.channel(), [16; 16]);
        assert_eq!(
            StudioSettlementState::from(EpochPhase::Fault),
            StudioSettlementState::Fault
        );
    }
}
