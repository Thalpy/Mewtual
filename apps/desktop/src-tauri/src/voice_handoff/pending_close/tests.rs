//! Real opened MAC/AEAD requests and native actor/selection fixtures. No browser provider
//! runs here: these tests model its prior drainage and do not enable production dispatch.
use super::*;
use crate::voice_activation_tests::Fixture;
use crate::voice_authority::SelectedHandoffOwner;
use crate::voice_handoff::bootstrap::{
    collect_channel, ensure, SetupOutputRef, Status as SetupStatus,
};
use crate::voice_handoff::factory::tests::{channel_route, selection};
use crate::voice_handoff::ledger::{Begin, PendingStatus, Registration};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::resources::ResourceKind;
use rand_core::SeedableRng;
use std::sync::Arc;

struct Case {
    native: Fixture,
    context: Context,
    route: Route,
    public: SetupOutputRef,
    selected: Arc<SelectedHandoffOwner>,
    selector: PairSelector,
    id: RowId,
}
impl Case {
    async fn new() -> Self {
        let native = Fixture::new().await;
        let (_owner, selected, selector) = selection(&native).await;
        let route = channel_route(&native, &selected).await;
        let SetupStatus::Ready(public) = ensure(
            &native.state,
            selected.id(),
            selected.executor(),
            &mut rand_chacha::ChaCha20Rng::seed_from_u64(83),
        )
        .await
        .unwrap() else {
            panic!("ready original channel")
        };
        let Begin::Factory(registration) = native
            .state
            .voice_handoff
            .begin(selected.clone(), Purpose::Derive(selector))
            .unwrap()
        else {
            panic!("pending original")
        };
        registration.mark_handoff().unwrap();
        let id = registration.id();
        drop(registration); // actual native attempt may have published a Root pair
        let context = Context::new(&native.state);
        Self {
            native,
            context,
            route,
            public,
            selected,
            selector,
            id,
        }
    }
    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![1];
        bytes.extend_from_slice(self.selector.peer.as_bytes());
        bytes.extend_from_slice(&self.selector.join);
        bytes.extend_from_slice(&self.selector.key_ref);
        bytes.extend_from_slice(&self.selector.epoch.to_be_bytes());
        bytes.push(0); // explicit ordinary return, never whole-Workflow retirement
        bytes
    }
    fn frame(&self, sequence: u64, operation: u8, logical: [u8; 16], bytes: &[u8]) -> Incoming {
        crate::voice_handoff::service::tests::opened_frame(
            &self.public,
            &self.route,
            (sequence, operation, 0, logical),
            bytes,
            None,
        )
    }
    fn close(&self, sequence: u64) -> Response {
        close(
            &self.context,
            &self.route,
            self.frame(sequence, 6, self.id.logical(), &self.bytes()),
        )
    }
    fn retry(&self) -> Registration {
        let Begin::Factory(registration) = self
            .native
            .state
            .voice_handoff
            .begin(self.selected.clone(), Purpose::Derive(self.selector))
            .unwrap()
        else {
            panic!("same pending retry")
        };
        registration
    }
    fn status(&self) -> Status {
        self.native
            .state
            .voice_handoff
            .resolve(&self.selected, Purpose::Derive(self.selector), self.id)
            .unwrap()
            .status()
            .unwrap()
    }
}

#[tokio::test]
async fn authenticated_pending_close_is_explicit_local_history_and_never_rearms_the_factory() {
    let case = Case::new().await;
    let response = case.close(1);
    let status = response.result.as_ref().unwrap();
    assert!(matches!(
        status.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(status.action.is_none() && status.engine.receipt.is_none());
    assert!(!status.engine.settled && !status.acknowledged && !status.drained);
    assert!(
        case.route.session().header().is_err(),
        "actual uncollected response still owns the physical position"
    );
    let counts = case.selected.resources().counts();
    for _ in 0..4 {
        let Begin::Existing(id) = case
            .native
            .state
            .voice_handoff
            .begin(case.selected.clone(), Purpose::Derive(case.selector))
            .unwrap()
        else {
            panic!("closed purpose cannot restart")
        };
        assert!(id == case.id);
        assert_eq!(case.selected.resources().counts(), counts);
    }
    let (original, incoming) = response.into_parts();
    drop(incoming);
    let (retry, incoming) = case.close(2).into_parts();
    assert!(retry.unwrap() == original.unwrap());
    assert_eq!(case.selected.resources().counts(), counts);
    drop(incoming);
    let compacted = case
        .native
        .state
        .voice_handoff
        .lookup_original(&case.selected, Purpose::Derive(case.selector))
        .unwrap()
        .wait_compacted()
        .await
        .unwrap();
    assert!(matches!(
        compacted.pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(
        !compacted.drained && !compacted.engine.settled,
        "local closure is not an attached Phase/Root compaction receipt"
    );
    assert!(case
        .native
        .state
        .voice_handoff
        .channel_may_close(&case.selected));
    collect_channel(
        &case.native.state,
        case.selected.id(),
        case.selected.executor(),
    )
    .await
    .unwrap();
    case.native
        .state
        .voice_channels
        .collect_retired(&case.native.state.voice_handoff);
    assert!(
        case.native
            .state
            .voice_handoff
            .lookup_original(&case.selected, Purpose::Derive(case.selector))
            .is_some(),
        "live lease must retain the same closed-purpose fence"
    );
    case.native.close().await;
}

#[tokio::test]
async fn active_registration_wins_pending_close_and_exact_retry_closes_only_after_it_drops() {
    let case = Case::new().await;
    let registration = case.retry();
    let counts = case.selected.resources().counts();
    let (refused, incoming) = case.close(1).into_parts();
    assert!(matches!(
        refused,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    assert!(matches!(case.status().pending, PendingStatus::Open));
    assert_eq!(case.selected.resources().counts(), counts);
    drop(incoming);
    drop(registration);
    let (closed, incoming) = case.close(2).into_parts();
    assert!(matches!(
        closed.unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    drop(incoming);
    case.native.close().await;
}

#[tokio::test]
async fn changed_purpose_header_rowid_opcode_and_noncanonical_body_cannot_close_original_pending() {
    let case = Case::new().await;
    let mut sequence = 0;
    let counts = case.selected.resources().counts();
    for index in [1, 33, 65, 104] {
        let mut bytes = case.bytes();
        bytes[index] ^= 1;
        sequence += 1;
        let response = close(
            &case.context,
            &case.route,
            case.frame(sequence, 6, case.id.logical(), &bytes),
        );
        assert!(matches!(
            response.result,
            Err(Refused::Protocol(VoiceError::Unauthorized))
        ));
    }
    let mut foreign_id = case.id.logical();
    foreign_id[15] ^= 2;
    for (operation, logical, bytes) in [
        (6, foreign_id, case.bytes()),
        (1, case.id.logical(), case.bytes()),
        (6, case.id.logical(), case.bytes()[..104].to_vec()),
        (6, case.id.logical(), [case.bytes(), vec![0]].concat()),
    ] {
        sequence += 1;
        let response = close(
            &case.context,
            &case.route,
            case.frame(sequence, operation, logical, &bytes),
        );
        assert!(response.result.is_err());
    }
    assert!(matches!(case.status().pending, PendingStatus::Open));
    assert_eq!(case.selected.resources().counts(), counts);
    case.native.close().await;
}

#[tokio::test]
async fn authenticated_foreign_session_is_not_the_original_close_capability() {
    let original = Case::new().await;
    let foreign = Case::new().await;
    let incoming = foreign.frame(1, 6, original.id.logical(), &original.bytes());
    let response = close(&original.context, &original.route, incoming);
    assert!(matches!(
        response.result,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    assert!(matches!(original.status().pending, PendingStatus::Open));
    drop(response);
    original.native.close().await;
    foreign.native.close().await;
}

#[tokio::test]
async fn pending_close_recovers_after_end_without_grant_upgrade_and_collects_only_after_last_owner()
{
    let case = Case::new().await;
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    assert!(case.selected.grant_identity().upgrade().is_none());
    let response = case.close(1);
    assert!(matches!(
        response.result.as_ref().unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(!case.status().engine.settled);
    let late = case
        .native
        .state
        .voice_handoff
        .lookup_original(&case.selected, Purpose::Derive(case.selector))
        .unwrap();
    collect_channel(
        &case.native.state,
        case.selected.id(),
        case.selected.executor(),
    )
    .await
    .unwrap();
    let Case {
        native,
        selected,
        selector,
        route,
        public,
        ..
    } = case;
    drop((route, public));
    native
        .state
        .voice_channels
        .collect_retired(&native.state.voice_handoff);
    assert!(
        native
            .state
            .voice_handoff
            .lookup_original(&selected, Purpose::Derive(selector))
            .is_some(),
        "response owns the actual Session independently"
    );
    drop(response);
    native
        .state
        .voice_channels
        .collect_retired(&native.state.voice_handoff);
    assert!(matches!(
        late.status().unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    drop(late);
    native
        .state
        .voice_channels
        .collect_retired(&native.state.voice_handoff);
    assert!(native
        .state
        .voice_handoff
        .lookup_original(&selected, Purpose::Derive(selector))
        .is_none());
    native.close().await;
}

#[tokio::test]
async fn pending_close_uses_original_control_capacity_with_all_process_bytes_occupied() {
    let case = Case::new().await;
    let incoming = case.frame(1, 6, case.id.logical(), &case.bytes());
    let pool = case.selected.resources();
    let group = case.selected.binding().scope.group;
    let mut pressure = Vec::new();
    for shift in (0..=18).rev() {
        if let Ok(credit) = pool.charge(group, 1 << shift, ResourceKind::Retained) {
            pressure.push(credit);
        }
    }
    assert!(pool.charge(group, 1, ResourceKind::Retained).is_err());
    for value in 97..=105 {
        let left = 2 * 1024 * 1024 - pool.counts().0;
        if left == 0 {
            break;
        }
        pressure.push(
            pool.charge(
                [value; 32],
                left.min(catcoms_sync::voice_v2::resources::GROUP_BYTES),
                ResourceKind::Retained,
            )
            .unwrap(),
        );
    }
    assert_eq!(pool.counts().0, 2 * 1024 * 1024);
    let response = close(&case.context, &case.route, incoming);
    assert!(matches!(
        response.result.as_ref().unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(
        pool.counts().0 < 2 * 1024 * 1024,
        "only original local preparation was returned"
    );
    assert!(!response.result.as_ref().unwrap().engine.settled);
    drop(response);
    drop(pressure);
    case.native.close().await;
}

#[tokio::test]
async fn lock_unlock_never_revives_pending_but_original_historical_close_still_works() {
    let case = Case::new().await;
    crate::register_ui_lock_request(&case.native.state);
    crate::lock_session_inner(&case.native.state, None)
        .await
        .unwrap();
    let generation = case
        .native
        .state
        .ui_session_generation
        .load(std::sync::atomic::Ordering::Acquire);
    crate::finalize_unlock_session(&case.native.state, generation, false)
        .await
        .unwrap();
    assert!(!case.selected.lease().is_live());
    assert!(matches!(
        case.native
            .state
            .voice_handoff
            .begin(case.selected.clone(), Purpose::Derive(case.selector)),
        Err(VoiceError::Unauthorized)
    ));
    let response = case.close(1);
    assert!(matches!(
        response.result.as_ref().unwrap().pending,
        PendingStatus::Closed {
            actor_handoff: true
        }
    ));
    assert!(!response.result.as_ref().unwrap().engine.settled);
    drop(response);
    case.native.close().await;
}
