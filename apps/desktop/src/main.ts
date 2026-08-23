import { mount } from "svelte";
import "./app.css";
import {
  beginStartupCapture,
  diagnosticId,
  drainStartupLog,
  renderStartupFailure,
  startupFailureReport,
} from "./startup-log";

// First statement to execute, ahead of every application module. An error thrown while evaluating
// or importing one of them now lands somewhere instead of vanishing into a devtools console nobody
// has open: the blank-window reports that arrived with nothing attached were all from this window.
beginStartupCapture();

async function start() {
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
  const { default: App } = await import("./App.svelte");
  return mount(App, {
    target: document.getElementById("app")!,
  });
}

/**
 * Nothing beyond this point can report through the application, because the application is what
 * failed. The screen is drawn directly and the error is kept where a person can read and copy it.
 */
export default start().catch((error: unknown) => {
  const id = diagnosticId();
  const report = startupFailureReport(id, error, drainStartupLog(), navigator.userAgent);
  // Still worth printing: a developer with devtools open should see this the ordinary way too.
  console.error(report);
  const target = document.getElementById("app");
  if (target) renderStartupFailure(target, id, report);
  return null;
});
