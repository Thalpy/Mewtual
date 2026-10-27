use super::*;

#[test]
fn native_sync_state_keeps_lossless_incarnation_sequence_and_actual_cause() {
    let value = sync_state_payload(
        7,
        u64::MAX,
        app::studio::StudioSyncObservation {
            sequence: u64::MAX,
            paused: true,
            cause: Some(app::studio::StudioSyncPauseCause::Background),
            detail: Some("source unavailable".into()),
        },
    );
    assert_eq!(value["instance"], u64::MAX.to_string());
    assert_eq!(value["sequence"], u64::MAX.to_string());
    assert_eq!(value["paused"], true);
    assert_eq!(value["cause"], "background");
    assert_eq!(value["detail"], "source unavailable");
}
