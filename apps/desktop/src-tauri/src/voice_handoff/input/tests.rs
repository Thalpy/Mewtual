use super::super::derivation_result::tests::Case;
use super::super::service::tests::opened_frame;
use super::*;

fn frame(case: &Case, sequence: u64, logical: [u8; 16]) -> Incoming {
    opened_frame(
        &case.public,
        &case.route,
        (sequence, 1, 0, logical),
        &case.bytes()[..105],
        None,
    )
}

#[tokio::test]
async fn input_uses_only_original_prepaid_copy_capacity_under_ordinary_pool_pressure() {
    use catcoms_sync::voice_v2::resources::ResourceKind;
    let case = Case::new().await;
    let pool = case.route.selected().resources();
    let pressure = pool
        .charge(
            case.route.selected().binding().scope.group,
            catcoms_sync::voice_v2::resources::GROUP_BYTES - pool.counts().0,
            ResourceKind::Retained,
        )
        .unwrap();
    let before = pool.counts();
    let reply = pull(
        &case.context,
        &case.route,
        frame(&case, 1, case.id.logical()),
    )
    .await
    .unwrap();
    assert_eq!(
        pool.counts(),
        before,
        "serialization borrows the original Phase, not ordinary bytes"
    );
    drop(reply);
    drop(pressure);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn copy_capacity_refusal_drops_actual_input_before_releasing_its_physical_position() {
    let case = Case::new().await;
    let work = case.row().work().unwrap();
    let phase = work.phase().clone();
    drop(work);
    let held = phase.buffer(16 * 1024).unwrap();
    let result = pull(
        &case.context,
        &case.route,
        frame(&case, 1, case.id.logical()),
    )
    .await;
    assert!(result.is_err());
    assert!(
        !case.route.session().fixture_counts().0,
        "refused serialization has no surviving physical input owner"
    );
    assert!(
        case.row().work().is_ok(),
        "the refused pull released its original Work"
    );
    case.retire();
    assert!(
        !phase.is_drained(),
        "the independent original buffer remains paid"
    );
    drop(held);
    assert!(phase.is_drained());
    case.native.close().await;
}

#[tokio::test]
async fn data_reply_keeps_actual_input_work_and_physical_position_through_collection() {
    let case = Case::new().await;
    let work = case.row().work().unwrap();
    let phase = work.phase().clone();
    drop(work);
    let incoming = frame(&case, 1, case.id.logical());
    let reply = pull(&case.context, &case.route, incoming).await.unwrap();
    assert!(reply.body().len() > FIXED_BYTES + 16);
    assert!(
        case.row().work().is_err(),
        "completed output still owns actual Work"
    );
    assert!(case.route.session().fixture_counts().0);
    case.retire();
    assert!(
        !phase.is_drained(),
        "End/settlement cannot collect an unobserved output"
    );
    drop(reply);
    assert!(!case.route.session().fixture_counts().0);
    assert!(phase.is_drained());
    case.native.close().await;
}

#[tokio::test]
async fn terminal_input_recovers_control_history_without_reconstructing_input() {
    let case = Case::new().await;
    case.action.settle_engine();
    let incoming = frame(&case, 1, case.id.logical());
    let prepared = prepare(&case.context, &case.route, &incoming)
        .await
        .unwrap();
    assert!(matches!(prepared, Prepared::History(_, _)));
    drop(prepared);
    let reply = pull(&case.context, &case.route, incoming).await.unwrap();
    assert_eq!(reply.body().len(), super::super::replies::STATUS_BYTES + 16);
    drop(reply);
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn input_rejects_foreign_session_or_changed_row_before_lending_work() {
    let case = Case::new().await;
    let foreign = Case::new().await;
    let incoming = frame(&foreign, 1, case.id.logical());
    assert!(matches!(
        prepare(&case.context, &case.route, &incoming).await,
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(incoming);
    let mut logical = case.id.logical();
    logical[15] += 1;
    let incoming = frame(&case, 1, logical);
    assert!(prepare(&case.context, &case.route, &incoming)
        .await
        .is_err());
    drop(incoming);
    assert!(case.row().work().is_ok());
    case.retire();
    foreign.retire();
    case.native.close().await;
    foreign.native.close().await;
}
