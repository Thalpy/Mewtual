// `./startup-log` is the only static import in this file, and it has to stay that way.
//
// A static import is evaluated before the body of the module that asks for it, so `svelte`,
// `app.css` and `./startup-log` itself all ran to completion before `beginStartupCapture()` did.
// A module that throws while being evaluated therefore threw into a window with no handler on it:
// blank window, nothing written, nothing to send. Being the first *statement* was never early
// enough. `./startup-log` imports nothing at runtime for the same reason, so the one module ahead
// of the capture cannot be the module that fails, and everything else is loaded below by a dynamic
// import that lands in the handler installed here.
import {
  beginStartupCapture,
  diagnosticId,
  drainStartupLog,
  renderStartupFailure,
  startupFailureDetail,
  startupFailureSummary,
} from "./startup-log";

beginStartupCapture();

async function start() {
  // Loaded here rather than at the top of the file so that a stylesheet which fails to arrive is a
  // caught failure with a screen rather than a silent one. The window shows nothing until the mount
  // below, so nothing renders unstyled in the meantime.
  await import("./app.css");

  const fixture = new URLSearchParams(window.location.search).get("fixture");

  // The visual fixture is deliberately a development-only door. It lets browser automation
  // render the real Svelte application with deterministic data, but production builds always
  // retain the native Tauri IPC boundary even if somebody supplies the same query parameter.
  if (import.meta.env.DEV && window.location.hostname === "localhost" && fixture) {
    const { installVisualFixture } = await import("./visual-fixture");
    installVisualFixture(fixture);
  }

  // App.svelte imports Tauri's window APIs at module evaluation time, so it must be loaded only
  // after the development fixture has installed Tauri's supported frontend mocks.
  const { mount } = await import("svelte");
  const { default: App } = await import("./App.svelte");
  return mount(App, {
    target: document.getElementById("app")!,
  });
}

/**
 * Nothing beyond this point can report through the application, because the application is what
 * failed. The screen is drawn directly and the error is kept where a person can read and copy it.
 *
 * Two reports: the redacted summary the screen offers, and the raw detail behind a disclosure. The
 * console gets the raw one, because a developer with devtools open is already looking at their own
 * machine and wants the frames.
 */
export default start().catch((error: unknown) => {
  const id = diagnosticId();
  const captured = drainStartupLog();
  const summary = startupFailureSummary(id, error, captured, navigator.userAgent);
  const detail = startupFailureDetail(id, error, captured, navigator.userAgent);
  console.error(detail);
  const target = document.getElementById("app");
  if (target) renderStartupFailure(target, id, summary, detail);
  return null;
});
