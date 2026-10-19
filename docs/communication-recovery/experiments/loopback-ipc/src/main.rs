//! Manual compatibility experiment, not production voice IPC or an application authority.
//! It opens no vault and transports no SDP, identities or user files. Stock framework response
//! copies and browser/network-service allocations are intentionally NOT claimed as bounded.
mod server;

use catcoms_rt::{OsCryptoRng, RngCore};
use serde::Serialize;
use std::sync::{atomic::Ordering, Arc, Mutex};
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[cfg(not(feature = "custom-protocol"))]
compile_error!("Build this experiment with --features custom-protocol to test embedded assets");

struct BootstrapState {
    attempts: u8,
    // Fixture material only. This is not a vault/controller/call or production engine permit.
    secret: [u8; 32],
}
struct State {
    server: Arc<server::Service>,
    bootstrap: Mutex<BootstrapState>,
}
#[derive(Serialize)]
struct Bootstrap {
    origin: String,
    blocked_origin: String,
    secret: [u8; 32],
}

/// Two successful serialization attempts for the SAME fixture key. Their modeled 4KiB reply
/// allowance is held until process exit; a lost reply never resets it. This does not measure
/// internal COM/SDK copies, and rejection replies remain a stock framework limitation.
#[tauri::command]
fn bootstrap(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<Bootstrap, &'static str> {
    state
        .server
        .metrics
        .bootstrap_entries
        .fetch_add(1, Ordering::SeqCst);
    if window.label() != "main" {
        return Err("unexpected bootstrap surface");
    }
    let mut bootstrap = state.bootstrap.lock().map_err(|_| "probe state poisoned")?;
    if bootstrap.attempts == 2 {
        return Err("fixture bootstrap allowance exhausted; restart the probe");
    }
    bootstrap.attempts += 1;
    state
        .server
        .metrics
        .bootstrap_grants
        .fetch_add(1, Ordering::SeqCst);
    Ok(Bootstrap {
        origin: state.server.origin(),
        blocked_origin: state.server.blocked_origin(),
        secret: bootstrap.secret,
    })
}

#[tauri::command]
fn inspect(state: tauri::State<'_, State>) -> server::Snapshot {
    state.server.snapshot()
}

#[tauri::command]
fn finish_probe(app: tauri::AppHandle, state: tauri::State<'_, State>) {
    state.server.stop();
    app.exit(0);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    // Bind before constructing Context/windows and retain both listeners through process shutdown.
    let (service, tasks) = runtime.block_on(server::Service::start())?;
    let mut secret = [0; 32];
    OsCryptoRng.fill_bytes(&mut secret);
    let state = State {
        server: service.clone(),
        bootstrap: Mutex::new(BootstrapState {
            attempts: 0,
            secret,
        }),
    };
    let mut context = tauri::generate_context!();
    let csp = context
        .config_mut()
        .app
        .security
        .csp
        .as_mut()
        .ok_or("missing fixture CSP")?;
    let mut directives: std::collections::HashMap<
        String,
        tauri::utils::config::CspDirectiveSources,
    > = csp.clone().into();
    let sources = directives
        .get_mut("connect-src")
        .ok_or("missing connect-src")?;
    sources.push(service.origin());
    *csp = directives.into();
    let remote = format!("{}/remote", service.origin()).parse()?;
    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![bootstrap, inspect, finish_probe])
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Mewtual IPC feasibility (no vault)")
                .inner_size(900.0, 640.0)
                .build()?;
            WebviewWindowBuilder::new(app, "restricted", WebviewUrl::App("index.html".into()))
                .initialization_script("window.__PROBE_SURFACE__='restricted';")
                .visible(false)
                .build()?;
            WebviewWindowBuilder::new(app, "remote", WebviewUrl::External(remote))
                .visible(false)
                .build()?;
            Ok(())
        })
        .build(context)?;
    let stopping = service.clone();
    let exit_code = app.run_return(move |handle, event| {
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { .. },
            ..
        } = event
        {
            if label == "main" {
                stopping.stop();
                handle.exit(0);
            }
        }
    });
    service.stop();
    runtime.block_on(async {
        for task in tasks {
            let _ = task.await;
        }
    });
    assert!(
        service.drained(),
        "probe workers retain actual custody through shutdown"
    );
    println!("PROBE.SHUTDOWN.DRAINED");
    if exit_code != 0 {
        return Err("probe event loop failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tauri::ipc::Origin;

    #[test]
    fn generated_acl_grants_bootstrap_only_to_main_local_origin() {
        let mut context: tauri::Context = tauri::generate_context!();
        let authority = context.runtime_authority_mut();
        let allowed = |label: &str, origin: &Origin| {
            authority
                .resolve_access("bootstrap", label, label, origin)
                .is_some_and(|value| !value.is_empty())
        };
        assert!(allowed("main", &Origin::Local));
        assert!(!allowed("restricted", &Origin::Local));
        assert!(!allowed(
            "main",
            &Origin::Remote {
                url: "http://127.0.0.1:1".parse().unwrap()
            }
        ));
    }
}
