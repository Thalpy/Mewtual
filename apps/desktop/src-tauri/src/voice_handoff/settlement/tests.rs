use super::super::derivation_result::tests::Case;
use super::super::service::tests::opened_frame;
use super::*;

fn frame(case: &Case, sequence: u64, operation: u8, digest: Option<[u8; 32]>) -> Incoming {
    let mut bytes = case.bytes()[..153].to_vec();
    if let Some(digest) = digest {
        bytes.extend_from_slice(&digest);
    }
    opened_frame(
        &case.public,
        &case.route,
        (sequence, operation, 0, case.id.logical()),
        &bytes,
        None,
    )
}

#[tokio::test]
async fn lost_incomplete_ordinary_return_compacts_on_later_terminal_retry_without_relabelling() {
    let case = Case::new().await;
    case.action.settle_engine();
    let row = case.row();
    let work = row.work().unwrap();
    let phase = work.phase().clone();
    drop(work);
    let copy = phase.buffer(1024).unwrap();
    let mut body = case.bytes()[..105].to_vec();
    body.push(0);
    let incoming = opened_frame(
        &case.public,
        &case.route,
        (1, 6, 0, case.id.logical()),
        &body,
        None,
    );
    let reply = apply(&case.context, &case.route, incoming).seal().unwrap();
    assert!(!row.reconcile().unwrap().drained);
    drop(reply); // non-drained Return reply lost; there is no queued cleanup visitor
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    assert!(!case.selected.lease().is_live());
    drop(copy);
    assert!(phase.is_drained());
    assert!(
        !row.status().unwrap().drained,
        "local Phase completion alone cannot invent row compaction"
    );
    body[105] = 1; // browser terminal cleanup still requests Retire, never relabels Return
    let header = (2, 6, 0, case.id.logical());
    let reply = super::super::finalization::respond(
        &case.context,
        &case.route,
        opened_frame(&case.public, &case.route, header, &body, None),
    )
    .seal()
    .unwrap();
    let refused = super::super::service::tests::opened_reply(&case.public, header, &body, &reply);
    assert_eq!(
        refused,
        super::super::replies::refused(VoiceError::Unauthorized.into())
    );
    drop(reply);
    assert!(
        row.reconcile().unwrap().drained,
        "the bounded visit collects the SAME original ordinary phase"
    );
    assert!(
        row.issue_final(case.route.session().identity()).is_err(),
        "ordinary completion owns no final witness"
    );
    assert!(
        !case
            .native
            .state
            .voice_handoff
            .channel_may_close(&case.selected),
        "coordinator revocation and local Return do not prove Root plan retirement"
    );
    assert!(row.retire_plan().unwrap());
    assert!(
        !case
            .native
            .state
            .voice_handoff
            .channel_may_close(&case.selected),
        "actual Root retirement cannot bypass original final proof delivery"
    );
    let issued = super::super::plan_final::request(&case.context, &case.route, &row)
        .await
        .unwrap();
    assert!(issued.proof.is_some() && !issued.acknowledged);
    row.acknowledge_final(case.route.session().identity(), issued.proof.unwrap())
        .unwrap();
    assert!(case
        .native
        .state
        .voice_handoff
        .channel_may_close(&case.selected));
    case.native.close().await;
}

#[tokio::test]
async fn original_session_can_settle_after_end_but_cannot_readmit() {
    let case = Case::new().await;
    case.native
        .state
        .voice_authority
        .lock()
        .unwrap()
        .end(case.selected.id());
    for sequence in 1..=2 {
        let incoming = frame(&case, sequence, 5, None);
        let (_, status) = original(&case.context, &case.route, &incoming).unwrap();
        assert!(status.engine.settled);
        drop(incoming);
        assert!(case
            .action
            .admit_engine(case.action.engine_bytes())
            .is_err());
    }
    case.retire();
    case.native.close().await;
}

#[tokio::test]
async fn digest_without_actual_terminal_receipt_cannot_acknowledge_anything() {
    let case = Case::new().await;
    let incoming = frame(&case, 1, 4, Some([7; 32]));
    assert!(matches!(
        original(&case.context, &case.route, &incoming),
        Err(Refused::Protocol(VoiceError::Unauthorized))
    ));
    drop(incoming);
    assert!(!case.row().status().unwrap().acknowledged);
    case.retire();
    case.native.close().await;
}
