import assert from "node:assert/strict";
import test from "node:test";
import { deferred, fakeIpc, indexContent, ordinaryView } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLSelectElement: w.HTMLSelectElement, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: ServerStudioSettings } = await import("./ServerStudioSettings.svelte");
const settle = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); flushSync(); };

test("Index repair status and a delayed rejection stay with their original channel", async () => {
  for (const delayed of [false, true]) {
    const ipc = fakeIpc(), response = deferred<unknown>();
    ipc.on("studio_list", () => ordinaryView(indexContent()));
    ipc.on("studio_fault_read", a => ({ v: 1, kind: "fault", channel: a.channel, object: null, scope: { kind: "source" }, source: { phase: "fault" }, candidates: ["a", "b"].map(c => ({ receipt: c.repeat(64), closedEpoch: "3", closeRecord: "c".repeat(64), seedChange: "d".repeat(64), inheritedEpoch: null, locallyInstalled: c === "a" })), mayDecide: true, blockedBy: null, waiting: 0, preservedOperations: 2, repair: null, lastAttempt: null }));
    ipc.on("studio_fault_repair", a => delayed ? response.promise : { v: 1, kind: "faultRepairStarted", channel: a.channel, scope: { kind: "source" }, start: "scheduled", refreshRequired: true });
    const target = document.createElement("div"); document.body.append(target);
    const app = mount(ServerStudioSettings, { target, props: { server: 1, channels: [{ id: "7", name: "A" }, { id: "8", name: "B" }], initialChannel: "7", owner: true, ipc } }); flushSync(); await settle();
    const click = async (text: string) => { const button = [...target.querySelectorAll<HTMLButtonElement>("button")].find(b => b.textContent?.trim() === text); assert.ok(button, text); button.click(); await settle(); };
    await click("Check history");
    const radio = target.querySelector<HTMLInputElement>('input[type="radio"]'); assert.ok(radio); radio.click(); await settle();
    await click("Keep selected version");
    if (!delayed) assert.match(target.textContent!, /Repair started/);
    const channel = target.querySelector<HTMLSelectElement>('select[aria-label="History channel"]')!;
    channel.value = "8"; channel.dispatchEvent(new Event("change", { bubbles: true })); await settle();
    if (delayed) { response.reject(new Error("channel A repair failed")); await settle(); }
    assert.doesNotMatch(target.textContent!, /Repair started|channel A repair failed/);
    assert.equal(target.querySelector('[role="alert"]'), null);
    assert.equal(target.querySelector<HTMLButtonElement>(".fault-controls button")?.disabled, false, "B does not inherit A's busy request");
    await unmount(app); target.remove();
  }
});
