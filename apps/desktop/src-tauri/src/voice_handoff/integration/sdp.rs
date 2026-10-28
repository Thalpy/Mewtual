//! Real authenticated storage control, after actual TCP/native Finish and KeyClaim.
//! Hold original preparation/copy owners at the real admission boundary; no sleeps or
//! literal unlocked/confirmed booleans stand in for the process publication fence.
use super::*;
use catcoms_sync::voice_v2::{
    resources::{Credit, ResourceKind, Resources, GROUP_BYTES},
    VoiceError,
};
pub(super) fn body(
    parent: &ledger::ResolvedRow,
    key: [u8; 32],
    release: Option<[u8; 32]>,
) -> Vec<u8> {
    let (serial, version, tag) = parent.status().unwrap().action.unwrap();
    let mut bytes = vec![5];
    bytes.extend_from_slice(&serial.to_be_bytes());
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.extend_from_slice(&tag);
    bytes.extend_from_slice(&parent.confirmation_handshake().unwrap());
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(&1u64.to_be_bytes());
    bytes.push(u8::from(release.is_some()));
    if let Some(proof) = release {
        bytes.extend_from_slice(&proof);
    }
    bytes
}
pub(super) async fn exchange(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> Vec<u8> {
    exchange_observed(state, public, route, parent, body)
        .await
        .0
}
async fn exchange_observed(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
) -> (Vec<u8>, u64) {
    let header = (
        route.session().fixture_counts().1 + 1,
        12,
        0,
        parent.id().logical(),
    );
    // Exercise the real authenticated adapter after framed native channel admission.
    let reply = crate::voice_handoff::sdp::apply(
        &context::Context::new(state),
        route,
        opened_frame(public, route, header, body, None),
    )
    .await
    .unwrap();
    (opened_reply(public, header, body, &reply), header.0)
}

/// Preparing fixture pressure is not protocol work. A concurrent paid owner may win
/// between the snapshot and charge, so retry that local refusal a bounded number of
/// times. The hook is test-only and runs outside all pool/native guards.
async fn hold_pressure(
    pool: &Arc<Resources>,
    group: [u8; 32],
    mut before_charge: impl FnMut(),
) -> Result<Credit, VoiceError> {
    for _ in 0..8 {
        let used = pool.group_counts(group).map_or(0, |counts| counts.0);
        let available = GROUP_BYTES - used;
        before_charge();
        if available != 0 {
            match pool.charge(group, available, ResourceKind::Retained) {
                Ok(credit) => return Ok(credit),
                Err(VoiceError::Capacity) => {}
                Err(error) => return Err(error),
            }
        }
        tokio::task::yield_now().await;
    }
    Err(VoiceError::Capacity)
}

/// Same logical intent and original Claim deadline throughout. Only an authenticated
/// Capacity reply is retryable. New physical nonces are required by the real codec;
/// neither the SDP sequence/proof nor a Source attempt/deadline is reset.
async fn retry_capacity(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    body: &[u8],
    deadline: u64,
    mut refused: impl FnMut(u64),
) -> Vec<u8> {
    let clock = crate::voice_runtime(state).clock();
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    loop {
        let now = clock.monotonic_ms();
        assert!(
            now < deadline,
            "original storage observation deadline expired; {}",
            route.session().fixture_native_snapshot().describe_sdp()
        );
        let (reply, sequence) = tokio::select! {
            result = exchange_observed(state, public, route, parent, body) => result,
            _ = clock.sleep(std::time::Duration::from_millis(deadline - now)) => {
                panic!("original storage exchange exceeded its remaining deadline");
            }
        };
        assert!(
            clock.monotonic_ms() < deadline,
            "storage observation returned after its original deadline"
        );
        if reply != capacity {
            return reply; // Unauthorized/closed/malformed never become retries.
        }
        refused(sequence);
        let remaining = deadline.saturating_sub(clock.monotonic_ms());
        clock
            .sleep(std::time::Duration::from_millis(remaining.min(10)))
            .await;
    }
}

#[cfg(test)]
mod pressure_tests {
    use super::*;

    #[tokio::test]
    async fn pressure_setup_retries_a_real_charge_race_without_losing_either_owner() {
        let pool = Arc::new(Resources::default());
        let group = [81; 32];
        let mut other = None;
        let mut samples = 0;
        let pressure = hold_pressure(&pool, group, || {
            samples += 1;
            if other.is_none() {
                // A genuine paid owner wins precisely between snapshot and charge.
                other = Some(pool.charge(group, 1, ResourceKind::Retained).unwrap());
            }
        })
        .await
        .unwrap();
        assert_eq!(samples, 2);
        assert_eq!(pool.counts(), (GROUP_BYTES, 0));
        drop(pressure);
        assert_eq!(pool.counts(), (1, 0));
        drop(other);
        assert_eq!(pool.counts(), (0, 0));
    }

    #[tokio::test]
    async fn pressure_setup_stops_after_eight_actual_losses_and_refunds_only_its_owners() {
        let pool = Arc::new(Resources::default());
        let group = [82; 32];
        let mut winners = Vec::new();
        let result = hold_pressure(&pool, group, || {
            winners.push(pool.charge(group, 1, ResourceKind::Retained).unwrap());
        })
        .await;
        assert!(matches!(result, Err(VoiceError::Capacity)));
        assert_eq!(winners.len(), 8);
        assert_eq!(pool.counts(), (8, 0));
        drop(winners);
        assert_eq!(pool.counts(), (0, 0));
    }
}
// Keep original endpoint/parent/proof owners explicit across these independent race modes.
#[allow(clippy::too_many_arguments)]
pub(super) async fn exercise(
    state: &AppState,
    public: &bootstrap::SetupOutput,
    route: &bootstrap::Route,
    parent: &ledger::ResolvedRow,
    key: [u8; 32],
    deadline: u64,
    attach: bool,
    sdk_race: bool,
    pc_case: Option<u8>,
) -> (
    Option<super::sdk::Token>,
    Option<super::sdp_http::RetiredProbe>,
) {
    channel::Session::fixture_prepare_native_diagnostics();
    let prepare = body(parent, key, None);
    let request = requests::sdp::Request::decode(parent.id().logical(), &prepare).unwrap();
    let capacity = replies::refused(factory::Refused::Protocol(VoiceError::Capacity));
    let unauthorized = replies::refused(factory::Refused::Protocol(VoiceError::Unauthorized));
    let commit = state.ui_session_commit.clone().lock_owned().await;
    assert_eq!(
        exchange(state, public, route, parent, &prepare).await,
        capacity
    );
    drop(commit);
    for offset in [1, 9, 17, 49, 81, 113] {
        let mut wrong = prepare.clone();
        wrong[offset] ^= 1;
        assert_eq!(
            exchange(state, public, route, parent, &wrong).await,
            unauthorized,
            "changed SDP native identity {offset}"
        );
    }
    let mut zero = prepare.clone();
    zero[113..121].fill(0);
    assert_eq!(
        exchange(state, public, route, parent, &zero).await,
        replies::refused(factory::Refused::Protocol(VoiceError::Malformed))
    );
    if attach {
        // Traverse the ACTUAL authenticated opcode12 path, not just Phase::prepare.
        // Hold ordinary-pool pressure and require the actual failure at RESERVATION,
        // not preflight/UI/attachment contention. Pool snapshots can change while the
        // actor preflight awaits; they are not an atomic one-byte boundary oracle.
        // Only this unrelated pressure is released before retrying the same intent.
        let pool = route.selected().resources();
        let group = route.selected().binding().scope.group;
        let pressure = hold_pressure(&pool, group, || {}).await.unwrap();
        assert_eq!(
            exchange(state, public, route, parent, &prepare).await,
            capacity,
            "actual storage admission must refuse under ordinary-pool pressure"
        );
        assert_eq!(
            route.session().fixture_native_snapshot().sdp_refusal(),
            Some((5, parent.id().logical(), Some(VoiceError::Capacity))),
            "only the actual reserve refusal exercises this recovery contract"
        );
        let failed = parent
            .sdp_test_preparation()
            .expect("original fixed intent survives refusal");
        assert_eq!(failed.0, 1);
        assert!(
            !failed.2 && !failed.3,
            "refused worker drained without installing a Phase"
        );
        assert!(parent.sdp_test_drained());
        drop(pressure);
        // Capacity release does not make native UI/Index fences immediately idle.
        // Force genuine UI contention AFTER the reserve refusal, and release only on
        // this physical request's stage4 evidence. A preflight refusal cannot satisfy it.
        let mut commit = Some(state.ui_session_commit.clone().lock_owned().await);
        let first = retry_capacity(
            state,
            public,
            route,
            parent,
            &prepare,
            deadline,
            |sequence| {
                if commit.is_some()
                    && route
                        .session()
                        .fixture_native_snapshot()
                        .sdp_refusal_at(sequence)
                        == Some((4, parent.id().logical(), Some(VoiceError::Capacity)))
                {
                    drop(commit.take());
                }
            },
        )
        .await;
        assert!(
            commit.is_none(),
            "actual forced UI admission refusal must have returned"
        );
        assert_eq!(
            first.len(),
            crate::voice_handoff::sdp::REPLY_BYTES,
            "unexpected authenticated storage reply prefix {:?}; {}",
            &first[..first.len().min(3)],
            route.session().fixture_native_snapshot().describe_sdp()
        );
        assert_eq!(&first[..2], &[5, 1]);
        assert_eq!(
            &first[138..170],
            &failed.1,
            "retry funds the SAME parent/action/key/sequence intent, not a replacement"
        );
        assert_eq!(
            retry_capacity(state, public, route, parent, &prepare, deadline, |_| {}).await,
            first,
            "lost reply recovers same original storage intent"
        );
        let proof = first[138..170].try_into().unwrap();
        let phase = parent.sdp_test_phase().unwrap();
        if let Some(case) = pc_case {
            let token = super::sdk::start(state, public, route, parent, &prepare, proof).await;
            let probe = super::pc::exercise(state, public, route, parent, token, case).await;
            assert!(parent.sdp_test_whole_drained());
            return (None, probe);
        }
        let mut copy = Some(phase.copy(8192).unwrap().adopt(vec![17; 8192]).unwrap());
        let sdk = if sdk_race {
            // Isolate SDK preparation custody: no native copy may mask a missing
            // sdk_preparing term in whole_drained during the candidate unwind test.
            drop(copy.take());
            super::sdk::close_preparation_then_unwind(
                state, public, route, parent, &prepare, proof,
            )
            .await;
            None
        } else {
            Some(super::sdk::start(state, public, route, parent, &prepare, proof).await)
        };
        let observer = phase.effect();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || parent.sdp_test_release_unwind(route.session().identity(), &request, proof)
        ))
        .is_err());
        // Quiet production collection must also resume a recorded closed intent while
        // the KeyClaim/call are still current. The held real buffer still blocks drainage.
        state.voice_handoff.collect_retired();
        let closing = exchange(
            state,
            public,
            route,
            parent,
            &body(parent, key, Some(proof)),
        )
        .await;
        assert_eq!(&closing[..2], &[5, if copy.is_some() { 2 } else { 3 }]);
        assert!(!observer.is_live());
        if let Some(copy) = &copy {
            assert!(!parent.sdp_test_drained());
            assert_eq!(
                copy.bytes()[0],
                17,
                "release cannot destroy a live native owner"
            );
        } else {
            assert!(parent.sdp_test_drained());
        }
        drop(copy);
        assert!(parent.sdp_test_drained());
        let closed = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&closed[..2], &[5, 3]);
        assert_eq!(&closed[138..170], &proof);
        assert!(phase.arm().is_err());
        assert_eq!(parent.sdp_test_whole_drained(), sdk.is_none());
        (sdk, None)
    } else {
        let context = context::Context::new(state);
        factory::preflight_binding(&context, route).await.unwrap();
        let mut pending = {
            let live = context.upgrade().unwrap();
            let commit = live.commit.clone().try_lock_owned().unwrap();
            let authority = live.authority.try_lock().unwrap();
            let admission =
                crate::voice_handoff::transition::Admission::new(&live, &authority, route, &commit);
            route
                .try_with_open(|| parent.begin_sdp(&admission, &request))
                .unwrap()
                .unwrap()
        };
        assert!(
            !parent.sdp_test_drained(),
            "intent is custody BEFORE64KiB allocation"
        );
        let preparing = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&preparing[..2], &[5, 0]);
        let proof = preparing[138..170].try_into().unwrap();
        let closing = exchange(
            state,
            public,
            route,
            parent,
            &body(parent, key, Some(proof)),
        )
        .await;
        assert_eq!(&closing[..2], &[5, 2]);
        // Simulate actual allocation returning AFTER release won; only final attachment
        // could arm it, and that original native fence must refuse irreversibly.
        pending.prepare().unwrap();
        {
            let live = context.upgrade().unwrap();
            let commit = live.commit.clone().try_lock_owned().unwrap();
            let authority = live.authority.try_lock().unwrap();
            let admission =
                crate::voice_handoff::transition::Admission::new(&live, &authority, route, &commit);
            assert!(matches!(
                route.try_with_open(|| parent.attach_sdp(&admission, &request, &mut pending)),
                Err(VoiceError::Unauthorized)
            ));
        }
        assert!(!parent.sdp_test_drained());
        drop(pending);
        assert!(parent.sdp_test_drained());
        let closed = exchange(state, public, route, parent, &prepare).await;
        assert_eq!(&closed[..2], &[5, 3]);
        assert_eq!(&closed[138..170], &proof);
        (None, None)
    }
}
