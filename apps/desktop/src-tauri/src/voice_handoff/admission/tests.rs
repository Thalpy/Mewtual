use super::super::derivation_result::tests::Case;
use super::super::service::tests::opened_frame;
use super::*;
use std::sync::Arc;

fn frame(case: &Case, sequence: u64, engine_bytes: usize) -> Incoming {
    let mut bytes = case.bytes()[..153].to_vec();
    bytes.extend_from_slice(&(engine_bytes as u32).to_be_bytes());
    opened_frame(
        &case.public,
        &case.route,
        (sequence, 2, 0, case.id.logical()),
        &bytes,
        None,
    )
}

/// Holds an ACTUAL native Coordinator/Route mutex on this test thread while a separate
/// synchronous admission thread uses the real final helper. Clock callbacks themselves
/// never block. Always release the guard before joining so a regression fails, not hangs.
fn refuses_held_final_gate<G>(case: &Case, clock: &catcoms_rt::ManualClock, guard: G) {
    use std::sync::mpsc;
    let context = case.context.upgrade().unwrap();
    let before = case.selected.resources().counts();
    std::thread::scope(|scope| {
        let (sampled, observed) = mpsc::channel();
        let (done, result) = mpsc::channel();
        let actual = case.action.clone();
        let route = &case.route;
        let context_ref = &context;
        let worker = scope.spawn(move || {
            let admission = actual.with_engine_admission(actual.engine_bytes(), |step| {
                sampled.send(()).unwrap();
                final_engine_admission(context_ref, route, step)
            });
            done.send(admission).unwrap();
        });
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        clock.advance_ms(2);
        let refusal = result.recv_timeout(std::time::Duration::from_secs(5));
        drop(guard);
        worker.join().unwrap();
        assert_eq!(refusal.unwrap(), Err(VoiceError::Capacity));
    });
    assert_eq!(case.selected.resources().counts(), before);
    assert!(case.native.state.ui_session_commit.try_lock().is_ok());
    assert_eq!(
        case.action
            .with_engine_admission(case.action.engine_bytes(), |step| final_engine_admission(
                &context,
                &case.route,
                step
            )),
        Err(VoiceError::Unauthorized)
    );
}

#[tokio::test]
async fn contended_coordinator_and_route_cannot_grant_work_after_original_deadline() {
    for route in [false, true] {
        let clock = Arc::new(catcoms_rt::ManualClock::new(1000));
        let case = Case::unadmitted_with_clock(clock.clone()).await;
        clock.set_ms(case.action.protected_binding().unwrap().deadline - 1);
        if route {
            let guard = case.route.fixture_lock_route();
            refuses_held_final_gate(&case, clock.as_ref(), guard);
        } else {
            let guard = case.native.state.voice_authority.lock().unwrap();
            refuses_held_final_gate(&case, clock.as_ref(), guard);
        }
        case.retire();
        case.native.close().await;
    }
}

#[tokio::test]
async fn exact_admission_retries_return_original_serial_without_charging_again() {
    let case = Case::unadmitted().await;
    let before = case.selected.resources().counts();
    for sequence in 1..=2 {
        let incoming = frame(&case, sequence, case.action.engine_bytes());
        let Ok(Encoded::Permit(bytes)) = prepare(&case.context, &case.route, &incoming).await
        else {
            panic!("actual engine permit");
        };
        assert_eq!(
            u64::from_be_bytes(bytes[66..74].try_into().unwrap()),
            case.action.serial()
        );
        drop(incoming);
        assert_eq!(case.selected.resources().counts(), before);
    }
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn busy_native_commit_is_retryable_and_settlement_never_reopens_the_engine() {
    let case = Case::new().await;
    let incoming = frame(&case, 1, case.action.engine_bytes());
    let commit = case
        .native
        .state
        .ui_session_commit
        .clone()
        .lock_owned()
        .await;
    assert!(matches!(
        prepare(&case.context, &case.route, &incoming).await,
        Err(Refused::Protocol(VoiceError::Capacity))
    ));
    drop(commit);
    assert!(matches!(
        prepare(&case.context, &case.route, &incoming).await,
        Ok(Encoded::Permit(_))
    ));
    drop(incoming);
    case.action.settle_engine();
    let incoming = frame(&case, 2, case.action.engine_bytes());
    assert!(matches!(
        prepare(&case.context, &case.route, &incoming).await,
        Ok(Encoded::History(_))
    ));
    drop(incoming);
    assert!(case
        .action
        .admit_engine(case.action.engine_bytes())
        .is_err());
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn changed_bytes_or_retired_selection_cannot_admit() {
    let case = Case::new().await;
    let incoming = frame(&case, 1, case.action.engine_bytes() + 1);
    assert!(matches!(
        prepare(&case.context, &case.route, &incoming).await,
        Err(Refused::Protocol(VoiceError::RetryMismatch))
    ));
    drop(incoming);
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    let incoming = frame(&case, 2, case.action.engine_bytes());
    assert!(prepare(&case.context, &case.route, &incoming)
        .await
        .is_err());
    drop(incoming);
    case.retire();
    case.native.close().await;
}
