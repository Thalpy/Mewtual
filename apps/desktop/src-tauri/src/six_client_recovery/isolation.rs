//! Private admission seams: intentional failure cannot poison the real acceptance gate.
use super::*;

async fn admitted(capacity: &Arc<tokio::sync::Semaphore>) -> Scenario {
    Scenario::new(
        false,
        FixtureAdmission {
            capacity: capacity.clone(),
            _permit: capacity.clone().acquire_owned().await.unwrap(),
        },
    )
}
fn panic_text(error: tokio::task::JoinError) -> String {
    let error = error.into_panic();
    if let Some(text) = error.downcast_ref::<String>() {
        text.clone()
    } else {
        error.downcast_ref::<&str>().unwrap().to_string()
    }
}

#[tokio::test]
async fn isolation_completed_owner_returns_its_original_private_turn() {
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let scenario = admitted(&capacity).await;
    assert_eq!(capacity.available_permits(), 0);
    drop(scenario);
    assert!(!capacity.is_closed());
    assert_eq!(capacity.available_permits(), 1);
    drop(admitted(&capacity).await);
}

#[tokio::test]
async fn isolation_cancelled_driver_construction_cannot_release_an_untracked_turn() {
    let parent = tempfile::tempdir().unwrap();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let mut scenario = admitted(&capacity).await;
    scenario.clients[0].root = tempfile::tempdir_in(parent.path()).unwrap();
    let vault = scenario.clients[0].root.path().to_path_buf();
    // Exactly the marker interval around BOTH real Transport::new callers: no Running or
    // driver receiver exists yet. Drop the parked constructor before receiver installation.
    scenario.clients[0].driver_starting = true;
    let mut construction = Box::pin(async move {
        let _original = scenario;
        std::future::pending::<()>().await;
    });
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(construction.as_mut().poll(&mut context).is_pending());
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(construction)));
    assert!(failure.is_err());
    assert!(capacity.is_closed());
    assert!(capacity.clone().acquire_owned().await.is_err());
    assert!(vault.exists());
}

#[tokio::test]
async fn isolation_unfinished_driver_closes_admission_and_preserves_failed_vault() {
    let parent = tempfile::tempdir().unwrap();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let mut scenario = admitted(&capacity).await;
    scenario.clients[0].root = tempfile::tempdir_in(parent.path()).unwrap();
    let vault = scenario.clients[0].root.path().to_path_buf();
    let (sender, receiver) = watch::channel(MeshObservationSnapshot::default());
    scenario.clients[0].driver = Some(receiver);
    let error = tokio::spawn(async move {
        drop(scenario);
    })
    .await
    .unwrap_err();
    assert!(panic_text(error).contains("must await all actual actors/drivers"));
    assert!(capacity.is_closed());
    assert!(capacity.clone().acquire_owned().await.is_err());
    assert!(
        vault.exists(),
        "failed storage remains available for inspection"
    );
    drop(sender);
    // The outer owned TempDir cleans this synthetic test artifact; no production vault is used.
}

#[tokio::test]
async fn isolation_cancelled_driver_shutdown_keeps_marker_until_actual_closure() {
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let mut scenario = admitted(&capacity).await;
    let (sender, receiver) = watch::channel(MeshObservationSnapshot::default());
    scenario.clients[0].driver = Some(receiver);
    let vault = scenario.clients[0].root.path().to_path_buf();
    let mut waiting = Box::pin(close_driver(&mut scenario.clients[0].driver));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(waiting.as_mut().poll(&mut context).is_pending());
    drop(waiting);
    assert!(scenario.clients[0].driver.is_some());
    assert!(vault.exists());
    assert_eq!(capacity.available_permits(), 0);
    drop(sender);
    close_driver(&mut scenario.clients[0].driver).await;
    assert!(scenario.clients[0].driver.is_none());
    drop(scenario);
    assert!(!capacity.is_closed());
    assert_eq!(capacity.available_permits(), 1);
}

#[tokio::test]
async fn isolation_outer_timeout_preserves_original_diagnostic_before_scenario_drop() {
    let parent = tempfile::tempdir().unwrap();
    let capacity = Arc::new(tokio::sync::Semaphore::new(1));
    let mut scenario = admitted(&capacity).await;
    scenario.clients[0].root = tempfile::tempdir_in(parent.path()).unwrap();
    let (sender, receiver) = watch::channel(MeshObservationSnapshot::default());
    scenario.clients[0].driver = Some(receiver);
    let error = tokio::spawn(async move {
        scenario_bounded(
            async move {
                let _original = scenario;
                std::future::pending::<()>().await;
            },
            std::future::ready(()),
            "original phase-specific total bound",
        )
        .await;
    })
    .await
    .unwrap_err();
    assert_eq!(panic_text(error), "original phase-specific total bound");
    assert!(capacity.is_closed());
    drop(sender);
}

#[tokio::test]
async fn isolation_observations_only_owner_cannot_start_real_clients() {
    let mut scenario = Scenario::observations(false);
    let error = tokio::spawn(async move {
        scenario.admit_prefix(3).await;
    })
    .await
    .unwrap_err();
    assert!(panic_text(error).contains("real clients require isolated fixture admission"));
}
