use super::super::derivation_result::tests::Case;
use super::super::service::tests::opened_frame;
use super::*;

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
