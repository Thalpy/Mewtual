use super::*;
use catcoms_sync::voice_v2::resources::ResourceKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn config() -> Config {
    serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap()
}

#[test]
fn origins_follow_tauri_mode_and_validate_the_actual_main_configuration() {
    let mut config = config();
    assert!(matches!(
        application_origin(&config, false, true),
        Ok(Origin::PackagedHttp)
    ));
    assert!(matches!(
        application_origin(&config, false, false),
        Ok(Origin::PackagedScheme)
    ));
    #[cfg(debug_assertions)]
    assert!(matches!(
        application_origin(&config, true, true),
        Ok(Origin::Development)
    ));
    #[cfg(not(debug_assertions))]
    assert!(application_origin(&config, true, true).is_err());
    config.app.windows[0].use_https_scheme = true;
    assert!(application_origin(&config, false, true).is_err());
    config.app.windows[0].use_https_scheme = false;
    config.build.dev_url = Some("http://127.0.0.1:1420/".parse().unwrap());
    assert!(application_origin(&config, true, true).is_err());
    config.app.windows[0].url = WebviewUrl::External("https://example.test/".parse().unwrap());
    assert!(application_origin(&config, false, true).is_err());
}

#[test]
fn remote_packaged_assets_and_duplicate_main_windows_are_not_implicitly_supported() {
    let mut config = config();
    config.build.frontend_dist = Some(FrontendDist::Url("https://example.test/".parse().unwrap()));
    assert!(application_origin(&config, false, true).is_err());
    config.build.frontend_dist = None;
    config.app.windows.push(config.app.windows[0].clone());
    assert!(application_origin(&config, false, true).is_err());
}

#[test]
fn csp_adds_one_exact_numeric_port_and_preserves_every_other_source_and_directive() {
    for policy in [config().app.security.csp, config().app.security.dev_csp] {
        let original: HashMap<String, CspDirectiveSources> = policy.clone().unwrap().into();
        let extended: HashMap<String, CspDirectiveSources> =
            extend_policy(policy.as_ref(), 43210).unwrap().into();
        assert_eq!(extended.len(), original.len());
        for (directive, sources) in &original {
            if directive == "connect-src" {
                let mut expected: Vec<String> = sources.clone().into();
                expected.push("http://127.0.0.1:43210".into());
                let actual: Vec<String> = extended[directive].clone().into();
                assert_eq!(actual, expected);
            } else {
                assert_eq!(&extended[directive], sources);
            }
        }
    }
}

#[test]
fn missing_or_deny_all_csp_refuses_without_disabling_any_policy() {
    assert!(extend_policy(None, 43210).is_err());
    assert!(extend_policy(Some(&Csp::Policy("default-src 'self'".into())), 43210).is_err());
    assert!(extend_policy(Some(&Csp::Policy("connect-src 'none'".into())), 43210).is_err());
    assert!(extend_policy(config().app.security.csp.as_ref(), 0).is_err());
    let state = AppState::default();
    let mut config = config();
    config.app.security.dev_csp = Some(Csp::Policy("default-src 'self'".into()));
    let original = config.clone();
    assert!(Host::prepare(&state, &mut config, false).is_err());
    assert_eq!(
        config, original,
        "no partial CSP edit on failed preparation"
    );
    assert_eq!(
        voice_runtime(&state).resources().counts().0,
        4096,
        "only the paid registry index remains"
    );
}

#[test]
fn capacity_refusal_does_not_open_or_advertise_a_replacement_service() {
    let state = AppState::default();
    let resources = voice_runtime(&state).resources();
    let pressure: Vec<_> = (1..=8)
        .map(|id| {
            resources
                .charge([id; 32], 256 * 1024, ResourceKind::Retained)
                .unwrap()
        })
        .collect();
    let mut config = config();
    let original = config.clone();
    assert!(Host::prepare(&state, &mut config, false).is_err());
    assert_eq!(config, original);
    assert_eq!(resources.counts().0, 2 * 1024 * 1024);
    assert!(Host::unavailable(Refused::Capacity).port().is_err());
    drop(pressure);
}

#[test]
fn explicit_tauri_runtime_entry_starts_from_a_thread_without_a_tokio_reactor() {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let state = AppState::default();
    let host = Host::prepare(&state, &mut config(), false).unwrap();
    assert!(
        host.port().is_err(),
        "binding alone never advertises readiness"
    );
    host.start_in_tauri_runtime().unwrap();
    assert_ne!(host.port().unwrap(), 0);
    host.start_in_tauri_runtime().unwrap(); // Idempotent setup cannot replace its port/task.
    tauri::async_runtime::block_on(host.finish()).unwrap();
    assert!(host.port().is_err());
    tauri::async_runtime::block_on(host.finish()).unwrap();
    assert_eq!(voice_runtime(&state).resources().counts().0, 4096);
}

#[test]
fn accidental_start_without_a_reactor_fails_closed_and_cannot_restart() {
    let state = AppState::default();
    let host = Host::prepare(&state, &mut config(), false).unwrap();
    assert_eq!(host.start(), Err(Refused::Io));
    assert!(host.port().is_err());
    assert_eq!(host.start_in_tauri_runtime(), Err(Refused::Io));
    assert_eq!(voice_runtime(&state).resources().counts().0, 4096);
}

#[tokio::test]
async fn actual_socket_and_original_task_are_collected_with_an_incomplete_client() {
    let state = AppState::default();
    let host = Host::prepare(&state, &mut config(), false).unwrap();
    host.start().unwrap();
    let port = host.port().unwrap();
    let mut client = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    client
        .write_all(b"POST /handoff HTTP/1.1\r\n")
        .await
        .unwrap();
    // Wall time bounds a broken harness only, not protocol time or release authority.
    tokio::time::timeout(std::time::Duration::from_secs(10), host.finish())
        .await
        .unwrap()
        .unwrap();
    assert!(host.port().is_err());
    assert!(
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .is_err()
    );
    let mut byte = [0];
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), client.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
    assert_eq!(voice_runtime(&state).resources().counts().0, 4096);
}

#[tokio::test]
async fn another_finish_cannot_claim_success_ahead_of_the_actual_collector() {
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let state = AppState::default();
    let host = Host::prepare(&state, &mut config(), false).unwrap();
    host.start().unwrap();
    // Current-thread runtime: poll the collector once without letting the service task run.
    // Its actual JoinHandle is pending, so a second caller must never report drained success.
    let mut first = Box::pin(host.finish());
    assert!(poll_fn(|cx| Poll::Ready(first.as_mut().poll(cx).is_pending())).await);
    assert_eq!(host.finish().await, Err(Refused::Io));
    first.await.unwrap();
    host.finish().await.unwrap();
    assert_eq!(voice_runtime(&state).resources().counts().0, 4096);
}

#[tokio::test]
async fn canceled_collection_does_not_turn_detached_native_work_into_drain_proof() {
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    let state = AppState::default();
    let host = Host::prepare(&state, &mut config(), false).unwrap();
    host.start().unwrap();
    let mut first = Box::pin(host.finish());
    assert!(poll_fn(|cx| Poll::Ready(first.as_mut().poll(cx).is_pending())).await);
    drop(first);
    assert_eq!(host.finish().await, Err(Refused::Io));
    assert!(host.port().is_err());
    // Real task remains paid until it observes stop and its detached output is dropped.
    let resources = voice_runtime(&state).resources();
    assert!(resources.counts().0 > 4096);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while resources.counts().0 != 4096 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        host.finish().await,
        Err(Refused::Io),
        "no fabricated collected receipt after observer loss"
    );
}
