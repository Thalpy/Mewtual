import { mount } from "svelte";
import "./app.css";

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

export default start();
