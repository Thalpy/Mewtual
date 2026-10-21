//! Application lifetime for the status-only native/browser channel. Bind and constrain CSP
//! before Tauri creates any windows; enter its IO runtime explicitly during setup. Neither
//! this owner nor its worker holds an AppHandle/AppState, so application teardown cannot form
//! a reference cycle. Its weak native context alone grants no call/action authority; the
//! currently exposed service still handles authenticated Status only.
use super::context::Context;
use super::service::{BoundServer, Origin, Refused, RunningServer};
use crate::{voice_runtime, AppState};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::utils::config::{Config, Csp, CspDirectiveSources, FrontendDist, WebviewUrl};

enum Lifecycle {
    Bound(BoundServer),
    Running(RunningServer),
    Unavailable(Refused),
    Draining,
    Finished(Result<(), Refused>),
}

/// One original port/task per application process. A failed/finished service is never replaced
/// behind a retained browser Session. Its caller gets an unavailable channel and may retain
/// uncertain original work; the rest of the application can still start and operate.
pub(crate) struct Host {
    port: u16,
    lifecycle: Mutex<Lifecycle>,
}

impl Host {
    pub(crate) fn prepare(
        state: &AppState,
        config: &mut Config,
        development: bool,
    ) -> Result<Self, Refused> {
        let origin = application_origin(config, development, cfg!(windows))?;
        let bound = BoundServer::bind_application(
            state.voice_channels.clone(),
            voice_runtime(state),
            origin,
            Context::new(state),
        )?;
        let port = bound.port();
        // Compute BOTH policies before changing either. A missing policy/directive must not
        // silently disable CSP or leave a half-configured context. The bound socket is dropped
        // before its infrastructure credit on every preparation failure.
        let csp = extend_policy(config.app.security.csp.as_ref(), port)?;
        let dev_csp = config
            .app
            .security
            .dev_csp
            .as_ref()
            .map(|policy| extend_policy(Some(policy), port))
            .transpose()?;
        config.app.security.csp = Some(csp);
        config.app.security.dev_csp = dev_csp;
        Ok(Self {
            port,
            lifecycle: Mutex::new(Lifecycle::Bound(bound)),
        })
    }

    pub(crate) fn unavailable(reason: Refused) -> Self {
        Self {
            port: 0,
            lifecycle: Mutex::new(Lifecycle::Unavailable(reason)),
        }
    }

    /// `setup` runs on the UI thread without an assured current Tokio reactor. Merely calling
    /// tokio::spawn there previously caused startup panics. Enter the pinned Tauri IO runtime
    /// only for synchronous conversion/spawn; never block the UI thread on network work.
    pub(crate) fn start_in_tauri_runtime(&self) -> Result<(), Refused> {
        let runtime = tauri::async_runtime::handle();
        let _entered = runtime.inner().enter();
        self.start()
    }

    fn start(&self) -> Result<(), Refused> {
        let mut held = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Lifecycle::Bound(_) = &*held else {
            return match &*held {
                Lifecycle::Running(_) => Ok(()),
                Lifecycle::Unavailable(error) => Err(*error),
                Lifecycle::Draining | Lifecycle::Finished(_) => Err(Refused::Io),
                Lifecycle::Bound(_) => unreachable!(),
            };
        };
        let Lifecycle::Bound(bound) =
            std::mem::replace(&mut *held, Lifecycle::Unavailable(Refused::Io))
        else {
            unreachable!()
        };
        match bound.start() {
            Ok(running) => {
                *held = Lifecycle::Running(running);
                Ok(())
            }
            Err(error) => {
                *held = Lifecycle::Unavailable(error);
                Err(error)
            }
        }
    }

    /// Readiness is about this actual service, not its configured port. IO may still fail
    /// immediately afterward, so the browser must authenticate and retain uncertain flights.
    pub(crate) fn port(&self) -> Result<u16, Refused> {
        let held = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match &*held {
            Lifecycle::Running(running) if !running.is_finished() => Ok(self.port),
            _ => Err(Refused::Io),
        }
    }

    /// Called synchronously in Tauri's Exit callback, BEFORE SDK cleanup/restart, while IO and
    /// logging still exist. Take the actual owner under the mutex, then await outside it.
    /// A concurrent/canceled caller cannot turn an unfinished drain into successful metadata:
    /// only actual completion installs Finished. This is not a renderer-facing retry protocol;
    /// the sole production Exit caller cannot be canceled. RunningServer stops on abnormal Drop.
    pub(crate) async fn finish(&self) -> Result<(), Refused> {
        let previous = {
            let mut held = self
                .lifecycle
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            std::mem::replace(&mut *held, Lifecycle::Draining)
        };
        let result = match previous {
            Lifecycle::Running(running) => running.finish().await.and_then(|completion| {
                if completion.io_failed {
                    Err(Refused::Io)
                } else {
                    Ok(())
                }
            }),
            Lifecycle::Draining => return Err(Refused::Io),
            Lifecycle::Finished(result) => result,
            Lifecycle::Bound(bound) => {
                drop(bound);
                Ok(())
            }
            Lifecycle::Unavailable(_) => Ok(()),
        };
        *self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Lifecycle::Finished(result);
        result
    }
}

/// Mirror the pinned Tauri URL decision, not Rust debug/release mode: custom-protocol debug
/// builds are packaged too. Config changes to remote assets, HTTPS or another dev origin need
/// their own platform acceptance, rather than accidentally widening this service's CORS.
fn application_origin(
    config: &Config,
    development: bool,
    windows: bool,
) -> Result<Origin, Refused> {
    let mut main = config
        .app
        .windows
        .iter()
        .filter(|window| window.label == "main");
    let window = main.next().ok_or(Refused::Binding)?;
    if main.next().is_some()
        || !matches!(&window.url, WebviewUrl::App(_))
        || window.use_https_scheme
    {
        return Err(Refused::Binding);
    }
    if development {
        if config.build.dev_url.as_ref().map(|url| url.as_str()) != Some("http://localhost:1420/") {
            return Err(Refused::Binding);
        }
        #[cfg(debug_assertions)]
        return Ok(Origin::Development);
        #[cfg(not(debug_assertions))]
        return Err(Refused::Binding);
    }
    if matches!(&config.build.frontend_dist, Some(FrontendDist::Url(_))) {
        return Err(Refused::Binding);
    }
    Ok(if windows {
        Origin::PackagedHttp
    } else {
        Origin::PackagedScheme
    })
}

fn extend_policy(policy: Option<&Csp>, port: u16) -> Result<Csp, Refused> {
    if port == 0 {
        return Err(Refused::Binding);
    }
    let mut directives: HashMap<String, CspDirectiveSources> =
        policy.ok_or(Refused::Binding)?.clone().into();
    let connect = directives.get_mut("connect-src").ok_or(Refused::Binding)?;
    let original: Vec<String> = connect.clone().into();
    if original.iter().any(|source| source == "'none'") {
        return Err(Refused::Binding);
    }
    connect.push(format!("http://127.0.0.1:{port}"));
    Ok(directives.into())
}

#[cfg(test)]
mod tests;
