//! One explicitly harness-owned retained row, never a new producer authority.
use super::*;
use crate::voice_handoff::factory::native::fixture::{ActionIdentity, Observation};
use crate::voice_handoff::ledger::{ResolvedRow, RowId};

fn retain_action(original: &mut Option<ActionIdentity>, actual: Option<ActionIdentity>) {
    if let Some(original) = original {
        assert_eq!(
            actual,
            Some(*original),
            "known Root publication cannot disappear or change"
        );
    } else {
        *original = actual;
    }
}

pub(super) struct Recovery {
    row: ResolvedRow,
    pub selector: PairSelector,
    address: usize,
    action: Option<ActionIdentity>,
}
impl Recovery {
    pub(super) async fn capture(
        context: &Context,
        route: &Route,
        observed: Observation,
        clock: &dyn Clock,
        pending: bool,
    ) -> Self {
        let selector = observed
            .selector
            .expect("post-selection refusal retains actual selector");
        let (id, address) = observed
            .parent
            .expect("recovery requires actual Begin, not guessed Root absence");
        let live = context.upgrade().expect("original mounted context");
        let row = live
            .ledger
            .resolve(route.selected(), Purpose::Derive(selector), id)
            .expect("same selected grant/executor/key/purpose resolves original row");
        assert_eq!(
            row.fixture_address(),
            address,
            "same registration allocation"
        );
        let mut recovery = Self {
            row,
            selector,
            address,
            action: observed.action,
        };
        assert_eq!(
            recovery.observe(route, clock).await,
            !pending,
            "Capacity retains Pending; a positive must actually attach, not return metadata"
        );
        recovery
    }
    pub(super) fn id(&self) -> RowId {
        self.row.id()
    }
    pub(super) fn worker(&self, observed: Observation) {
        if let Some(selector) = observed.selector {
            assert_eq!(
                selector, self.selector,
                "recovery cannot choose another peer/epoch/key"
            );
        }
        if let Some((id, address)) = observed.parent {
            assert_eq!(
                (id, address),
                (self.id(), self.address),
                "same row slot AND serial/allocation"
            );
        }
        if let (Some(original), Some(actual)) = (self.action, observed.action) {
            assert_eq!(
                actual, original,
                "same actual Root action/serial/version/input/key tag"
            );
        }
    }
    pub(super) async fn observe(&mut self, route: &Route, clock: &dyn Clock) -> bool {
        loop {
            match self
                .row
                .fixture_recovery_snapshot(route.selected(), self.selector)
            {
                Ok(snapshot) => {
                    retain_action(&mut self.action, snapshot.action);
                    if snapshot.attached {
                        assert!(
                            snapshot.action.is_some(),
                            "NotPending requires actual attached action"
                        );
                    } else {
                        assert!(
                            snapshot.prepared,
                            "same original preparation survives every refusal"
                        );
                    }
                    return snapshot.attached;
                }
                Err(catcoms_sync::voice_v2::VoiceError::Capacity) => {
                    clock.sleep(Duration::from_millis(10)).await
                }
                Err(error) => panic!("original recovery row closed/replaced/missing: {error:?}"),
            }
        }
    }
}

#[test]
fn recovery_action_observation_cannot_forget_or_replace_a_known_publication() {
    // Scalar harness identities are not native authority. This pins monotonic
    // observation: an initial pair/derive refusal may lack an Action; once seen,
    // that SAME publication must remain present until actual attachment.
    let action = ActionIdentity {
        address: 1,
        serial: 2,
        version: 1,
        input: [3; 32],
        keys: [[4; 32], [5; 32]],
        handshake: [6; 32],
        generation: 1,
        deadline: 100,
    };
    let mut known = None;
    retain_action(&mut known, None);
    retain_action(&mut known, Some(action));
    retain_action(&mut known, Some(action));
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        retain_action(&mut known, None);
    }))
    .is_err());
    let mut changed = action;
    changed.input[0] ^= 1;
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        retain_action(&mut known, Some(changed));
    }))
    .is_err());
}
