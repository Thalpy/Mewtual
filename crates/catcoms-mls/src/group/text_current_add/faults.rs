//! Test-only failures at real owning cache/state transitions. No fake SDK output,
//! success flag, operation permit or Source/model setter is supplied.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TestCut {
    AddEntered,
    OutputsCached,
    CommitSerialized,
    WelcomeSerialized,
    GroupInfoSerialized,
    Serialized,
    MergeEntered,
    Merged,
    ProviderClosed,
    ObservationCached,
    SnapshotCached,
    Complete,
}
#[derive(Default)]
pub(super) struct Faults {
    pub(super) fail_at: Option<TestCut>,
    pub(super) sdk_dispatches: usize,
    pub(super) merge_dispatches: usize,
}
impl PreparedTextCurrentAdd {
    pub(super) fn after_cut_for_test(&mut self, cut: TestCut) -> Result<(), MlsError> {
        if self.faults.fail_at == Some(cut) {
            self.faults.fail_at = None;
            Err(MlsError::Internal("injected private Add cache return loss"))
        } else {
            Ok(())
        }
    }
}
