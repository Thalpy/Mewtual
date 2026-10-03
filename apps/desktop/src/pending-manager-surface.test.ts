import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";
import type { PendingSend } from "./pending-sends.ts";

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { HTMLMediaElement: w.HTMLMediaElement, MouseEvent: w.MouseEvent, Event: w.Event });
const { default: Manager } = await import("./PendingSendManager.svelte");
const { mount, unmount, flushSync } = await import("svelte");

function surface(app: ReturnType<typeof pendingManagerApp>) {
  const target = document.createElement("div"); document.body.appendChild(target);
  const state = app.state();
  const mounted = mount(Manager, { target, props: {
    pending: state.pendingSends, recovered: state.recoveredSendDrafts, ready: true,
    conversationName: (server, channel) => `Unavailable conversation ${server} / channel ${channel}`,
    onresolve: app.resolve, onuse: app.use, onremove: app.remove,
  } });
  flushSync();
  return { target,
    async click(token: string, label: string) {
      const card = target.querySelector(`[data-pending-token="${token}"], [data-recovered-token="${token}"]`);
      assert.ok(card);
      const button = [...card.querySelectorAll("button")].find(button => button.textContent?.trim() === label);
      assert.ok(button, label); button.click(); flushSync(); await settle(); flushSync();
    },
    done() { unmount(mounted); target.remove(); },
  };
}

test("compiled vault manager confirms recovery/stop for missing conversations and restores saved copies after hydration", async () => {
  const entries: PendingSend[] = ["invalid", "conflict", "context_changed", undefined].map((retryBlock, n) => ({
    token: (n + 1).toString(16).padStart(32, "0"), server: 99, channel: String(n + 1), expectedContext: "a".repeat(64),
    text: `Keep the whole message ${n}`, replyTo: "original reply", retryBlock: retryBlock as PendingSend["retryBlock"],
  }));
  const app = pendingManagerApp({ pendingSends: Object.fromEntries(entries.map(item => [item.token, item])) });
  await app.hydrate(); app.manager(true);
  let ui = surface(app);
  assert.equal(ui.target.querySelectorAll("[data-pending-token]").length, 4);
  assert.match(ui.target.textContent!, /Unavailable conversation 99/);
  assert.match(ui.target.textContent!, /Request cannot be accepted/);
  assert.match(ui.target.textContent!, /Retry identity conflict/);
  assert.match(ui.target.textContent!, /Conversation changed/);
  assert.match(ui.target.textContent!, /may already have been accepted/);
  assert.ok(![...ui.target.querySelectorAll("button")].some(b => b.textContent?.startsWith("Use in")));
  for (const item of entries.slice(0, 3)) {
    await ui.click(item.token, "Recover to saved draft");
    assert.ok(app.state().pendingSends[item.token], "choosing an action is not confirmation");
    await ui.click(item.token, "Confirm recovery");
    assert.equal(app.state().pendingSends[item.token], undefined);
  }
  const last = entries[3];
  await ui.click(last.token, "Stop retrying");
  assert.match(ui.target.textContent!, /remove the stored request and its text/);
  await ui.click(last.token, "Confirm stop retrying");
  ui.done(); app.lock(); await app.unlock(); ui = surface(app);
  assert.equal(ui.target.querySelectorAll("[data-pending-token]").length, 0);
  assert.equal(ui.target.querySelectorAll("[data-recovered-token]").length, 3);
  assert.deepEqual([...ui.target.querySelectorAll<HTMLTextAreaElement>("textarea")].map(field => field.value), entries.slice(0, 3).map(item => item.text));
  await ui.click(entries[0].token, "Remove saved draft");
  assert.ok(app.state().recoveredSendDrafts[entries[0].token]);
  await ui.click(entries[0].token, "Confirm remove draft");
  assert.equal(app.state().recoveredSendDrafts[entries[0].token], undefined);
  assert.equal(app.state().submissions.length, 0);
  ui.done(); app.done();
});

test("compiled manager reports a failed decision without hiding the original pending text", async () => {
  const token = "1".repeat(32);
  const app = pendingManagerApp({ pendingSends: { [token]: { token, server: 1, channel: "3", expectedContext: "b".repeat(64), text: "retain me", replyTo: "", retryBlock: "invalid" } } });
  await app.hydrate(); app.failSaves(true);
  const ui = surface(app);
  await ui.click(token, "Stop retrying"); await ui.click(token, "Confirm stop retrying");
  assert.match(ui.target.querySelector('[role="alert"]')?.textContent ?? "", /injected disk failure/);
  assert.equal(ui.target.querySelector<HTMLTextAreaElement>("textarea")?.value, "retain me");
  assert.ok(app.state().pendingSends[token]);
  ui.done(); app.done();
});

test("pending manager is a personal settings page with no mounted-server condition", () => {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const pages = source.slice(source.indexOf("const USER_SET_PAGES"), source.indexOf("const SRV_SET_PAGES"));
  assert.match(pages, /id: "pending", label: "Pending messages"/);
  const page = source.slice(source.indexOf('{:else if settingsPage === "pending"}'), source.indexOf('{:else if settingsPage === "vault"}'));
  assert.match(page, /pending=\{pendingSends\} recovered=\{recoveredSendDrafts\}/);
  assert.doesNotMatch(page, /\{#if cur|servers\.length|\.filter\(/);
  assert.match(source, /\{:else if \(servers\.length === 0 \|\| showAdd\) && !showSettings\}/);
  const founding = source.slice(source.indexOf('{:else if (servers.length === 0 || showAdd)'), source.indexOf('<div class="app">'));
  assert.match(founding, /onclick=\{\(\) => openSettings\("pending"\)\}/);
});
