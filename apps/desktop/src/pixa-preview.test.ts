import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { PixaPreviewManager, type PixaPreviewOptions, type PixaPreviewScope } from "./pixa-preview.ts";
import { decodePixa, encodePixa } from "./pixa.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { deferred, settle } from "./studio-testkit.ts";

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
class Observer {
  static current: Observer;
  readonly observed = new Set<Element>();
  private readonly callback: IntersectionObserverCallback;
  constructor(callback: IntersectionObserverCallback) { this.callback = callback; Observer.current = this; }
  observe(element: Element) { this.observed.add(element); }
  unobserve(element: Element) { this.observed.delete(element); }
  disconnect() { this.observed.clear(); }
  emit(element: Element, visible: boolean) { this.callback([{target: element, isIntersecting: visible} as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
}
class Pixels {
  readonly data: Uint8ClampedArray; readonly width: number; readonly height: number;
  constructor(data: Uint8ClampedArray, width: number, height: number) { this.data = data; this.width = width; this.height = height; }
}
Object.assign(globalThis, { IntersectionObserver: Observer, MutationObserver: w.MutationObserver, ImageData: Pixels });
Object.defineProperty(document, "hidden", { configurable: true, value: false });
Object.defineProperty(w.HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ({ putImageData() {} }) });
const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const original = decodePixa(Uint8Array.from(Buffer.from(vector.pixaHex, "hex")));
const value = decodePixa(encodePixa({ ...original, fps: 10, frames: [original.frames[0], original.frames[0], original.frames[0]],
  take: { ...original.take, patches: [legacyJamPatch("sine")] }, sfx: [{frame: 0, patch: 0, note: 60, durationMs: 333}] }));
const scope: PixaPreviewScope = { server: 7, channel: "42", generation: 1, membership: 1, surface: "chat" };
const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
async function wait(predicate: () => boolean) { for (let i = 0; i < 80; i++) { if (predicate()) return; await sleep(5); } assert.fail("preview did not reach expected state"); }
function harness(overrides: Partial<PixaPreviewOptions> = {}) {
  let now = 0, allowed = true, current = true, owner: object | null = null, audioCalls = 0, reads = 0, cancels = 0;
  const manager = new PixaPreviewManager({ reader: {read: async () => { reads++; return value; }, cancel: () => { cancels++; }},
    current: () => current, autoLoad: () => allowed, now: () => now,
    startAudio: async token => { owner = token; audioCalls++; return true; },
    stopAudio: token => { if (owner === token) owner = null; }, audioState: token => owner === token ? {state: "playing", elapsedMs: now} : null, ...overrides });
  const hosts: HTMLElement[] = [];
  const create = (id = hosts.length) => { const host = manager.create({cid: String(id).padStart(64, "0"), name: "loop.pixa", size: 5000}, scope); document.body.append(host); hosts.push(host); return host; };
  return { manager, create, visible: (host: HTMLElement, on = true) => Observer.current.emit(host, on),
    setNow: (value: number) => { now = value; }, setAllowed: (value: boolean) => { allowed = value; }, setCurrent: (value: boolean) => { current = value; },
    audioCalls: () => audioCalls, owner: () => owner, reads: () => reads, cancels: () => cancels,
    destroy: () => { manager.destroy(); for (const host of hosts) host.remove(); } };
}
const button = (host: HTMLElement, text: string) => [...host.querySelectorAll("button")].find(button => button.textContent === text)!;

test("inline pictures loop silently by default and Pause holds the actual displayed frame", async () => {
  const h = harness(), host = h.create(), canvas = host.querySelector("canvas")!;
  let loads = 0; host.addEventListener("load", () => { loads++; });
  try {
    h.visible(host); await wait(() => canvas.dataset.pixaFrame === "0");
    h.setNow(150); await wait(() => canvas.dataset.pixaFrame === "1");
    h.setNow(350); await wait(() => canvas.dataset.pixaFrame === "0");
    button(host, "Pause").click(); h.setNow(1500); await sleep(60);
    assert.equal(canvas.dataset.pixaFrame, "0"); assert.equal(h.audioCalls(), 0); assert.equal(loads, 1, "only package arrival announces layout growth, never frame paints");
  } finally { h.destroy(); }
});
test("many embeds retain two packages, serialize reads and bound observer registrations", async () => {
  const pending = deferred<typeof value>(); let running = 0, peak = 0, reads = 0;
  const h = harness({reader: {read: async () => { reads++; running++; peak = Math.max(peak, running); const result = reads === 1 ? await pending.promise : value; running--; return result; }, cancel() {}}});
  try {
    const hosts = [h.create(), h.create(), h.create()]; for (const host of hosts) h.visible(host);
    assert.equal(reads, 1); pending.resolve(value); await wait(() => reads === 3);
    await wait(() => !hosts[2].querySelector("canvas")!.hidden);
    assert.equal(peak, 1); assert.equal(hosts[0].querySelector("canvas")!.hidden, true);
    for (let i = 0; i < 150; i++) h.create(i + 3);
    assert.equal(Observer.current.observed.size, 128);
    assert.equal(hosts.filter(host => !host.querySelector("canvas")!.hidden).length, 2);
  } finally { h.destroy(); }
});
test("an offscreen or removed card cannot attach a late read", async () => {
  const pending = deferred<typeof value>(); let cancelled = 0;
  const h = harness({reader: {read: () => pending.promise, cancel: () => { cancelled++; }}}), host = h.create();
  try {
    h.visible(host); h.visible(host, false); host.remove(); await settle(2); pending.resolve(value); await settle(4);
    assert.equal(host.querySelector("canvas")!.hidden, true); assert.ok(cancelled > 0);
  } finally { h.destroy(); }
});
test("old-card cleanup cannot stop a newer audio owner and revocation never auto-resumes sound", async () => {
  const h = harness(), first = h.create(), second = h.create();
  try {
    h.visible(first); h.visible(second); await wait(() => !second.querySelector("canvas")!.hidden);
    button(first, "Play sound").click(); await settle(2); const firstOwner = h.owner();
    button(second, "Play sound").click(); await settle(2); const secondOwner = h.owner(); assert.notEqual(firstOwner, secondOwner);
    first.remove(); await settle(3); assert.equal(h.owner(), secondOwner);
    h.setCurrent(false); h.manager.reset(); assert.equal(h.owner(), null); assert.equal(h.audioCalls(), 2);
    h.setCurrent(true); await sleep(60); assert.equal(h.audioCalls(), 2);
  } finally { h.destroy(); }
});
test("trust blocks passive reads while an explicit play grants only this card and reset revokes it", async () => {
  const h = harness(), host = h.create();
  try {
    h.setAllowed(false); h.visible(host); await settle(2); assert.equal(h.reads(), 0);
    button(host, "Play flipnote").click(); await wait(() => !host.querySelector("canvas")!.hidden); assert.equal(h.reads(), 1);
    h.manager.reset(); h.visible(host); await settle(2); assert.equal(h.reads(), 1); assert.equal(h.audioCalls(), 0);
  } finally { h.destroy(); }
});

test("an explicit Play renews a retained host after membership changes without reviving its old read or audio", async () => {
  const pending = deferred<typeof value>(); let membership = 1, reads = 0;
  const h = harness({ current: target => target.membership === membership,
    renewScope: previous => ({...previous, membership}),
    reader: {read: async () => ++reads === 1 ? pending.promise : value, cancel() {}} });
  const host = h.create();
  try {
    h.visible(host); membership = 2; h.manager.reset();
    button(host, "Play flipnote").click();
    assert.equal(host.querySelector("canvas")!.hidden, true); assert.equal(reads, 1, "old read must drain before new work");
    pending.resolve(value); await wait(() => reads === 2 && !host.querySelector("canvas")!.hidden);
    assert.equal(h.audioCalls(), 0);
    h.setNow(150); await wait(() => host.querySelector("canvas")!.dataset.pixaFrame === "1");
    h.setNow(350); await wait(() => host.querySelector("canvas")!.dataset.pixaFrame === "0");
  } finally { h.destroy(); }
});

test("an explicit Play cannot renew a retained host into another server", async () => {
  const h = harness({renewScope: previous => ({...previous, server: previous.server + 1})}), host = h.create();
  try { button(host, "Play flipnote").click(); await settle(2); assert.equal(h.reads(), 0); assert.match(host.textContent!, /Open this file’s channel/); }
  finally { h.destroy(); }
});

test("Pause cancels a pending sound start without allowing its late refusal to overwrite the card", async () => {
  const pending = deferred<boolean>(); let preparing = false;
  const h = harness({startAudio: async () => { preparing = true; return pending.promise; },
    stopAudio: () => { preparing = false; }, audioState: () => preparing ? {state: "preparing", elapsedMs: 0} : null});
  const host = h.create();
  try {
    h.visible(host); await wait(() => !host.querySelector("canvas")!.hidden);
    button(host, "Play sound").click(); button(host, "Pause").click(); pending.resolve(false); await settle(3);
    assert.doesNotMatch(host.textContent!, /Sound unavailable/); assert.equal(preparing, false);
  } finally { h.destroy(); }
});

test("a refused sound start remains retryable and a successful retry clears the old failure", async () => {
  let attempts = 0, playing = false;
  const h = harness({startAudio: async () => { if (++attempts === 1) return false; playing = true; return true; },
    stopAudio: () => { playing = false; }, audioState: () => playing ? {state: "playing", elapsedMs: 0} : null});
  const host = h.create();
  try {
    h.visible(host); await wait(() => !host.querySelector("canvas")!.hidden);
    button(host, "Play sound").click(); await wait(() => host.textContent!.includes("Sound unavailable"));
    button(host, "Play sound").click(); await wait(() => !!button(host, "Mute"));
    assert.equal(attempts, 2); assert.doesNotMatch(host.textContent!, /Sound unavailable/);
  } finally { h.destroy(); }
});
