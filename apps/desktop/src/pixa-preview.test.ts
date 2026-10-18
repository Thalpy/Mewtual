import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { PixaPreviewManager, type PixaPreviewFile, type PixaPreviewOptions, type PixaPreviewScope } from "./pixa-preview.ts";
import { decodePixa, encodePixa, PIXA_MAX_BYTES } from "./pixa.ts";
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

// The transport strip. Its buttons carry their names in a visually hidden span, so the same
// `button(host, "Pause")` lookups above keep working; these tests pin the readouts around them.
test("the strip's frame counter, progress and elapsed readouts follow the clock and hold with Pause", async () => {
  const h = harness(), host = h.create(), canvas = host.querySelector("canvas")!;
  const strip = host.querySelector<HTMLElement>(".pixa-tr")!, frame = host.querySelector(".pixa-frame")!, time = host.querySelector(".pixa-time")!;
  // The style engine serialises "50.0%" as "50%", so read the bar back as a number.
  const fill = host.querySelector<HTMLElement>(".pixa-prog > i")!, width = () => Number.parseFloat(fill.style.width);
  try {
    assert.equal(strip.hidden, true, "no transport before a package arrives");
    h.visible(host); await wait(() => canvas.dataset.pixaFrame === "0");
    assert.equal(strip.hidden, false);
    assert.equal(frame.textContent, "1 / 3"); assert.equal(time.textContent, "0.0 s"); assert.equal(width(), 0);
    h.setNow(150); await wait(() => canvas.dataset.pixaFrame === "1");
    assert.equal(frame.textContent, "2 / 3"); assert.equal(time.textContent, "0.1 s"); assert.equal(width(), 50);
    h.setNow(290); await wait(() => time.textContent === "0.2 s");
    assert.equal(frame.textContent, "3 / 3"); assert.equal(width(), 96.7);
    // The icon is decorative: the button's whole text is the action name, and the figure records
    // the pause so the stylesheet keeps the strip on screen.
    const pause = button(host, "Pause"); assert.equal(pause.querySelector("svg")!.textContent, ""); assert.equal(pause.title, "Pause");
    pause.click(); assert.equal(host.dataset.paused, "true"); assert.equal(pause, button(host, "Play"));
    h.setNow(1500); await sleep(60);
    assert.equal(canvas.dataset.pixaFrame, "2"); assert.equal(frame.textContent, "3 / 3"); assert.equal(time.textContent, "0.2 s"); assert.equal(width(), 96.7);
    button(host, "Play").click(); assert.equal(host.dataset.paused, undefined); assert.equal(button(host, "Pause"), pause);
    // Leaving the viewport releases the package and the strip with it.
    h.visible(host, false); assert.equal(strip.hidden, true); assert.equal(host.dataset.paused, undefined);
  } finally { h.destroy(); }
});

test("Open hands the host this card's file and current scope from the strip and the facts link; without a handler neither exists", async () => {
  const opened: { file: PixaPreviewFile; scope: PixaPreviewScope }[] = []; let membership = 1;
  const h = harness({ open: (file, target) => { opened.push({ file, scope: target }); },
    current: target => target.membership === membership, renewScope: previous => ({ ...previous, membership }) });
  const host = h.create();
  try {
    h.visible(host); await wait(() => !host.querySelector("canvas")!.hidden);
    button(host, "Open").click(); button(host, "open").click();
    assert.equal(opened.length, 2);
    for (const call of opened) { assert.equal(call.file.cid, "0".repeat(64)); assert.equal(call.file.name, "loop.pixa"); assert.deepEqual(call.scope, scope); }
    assert.equal(host.querySelector(".pixa-facts button.pixa-link"), button(host, "open"), "the text link sits in the facts line");
    // After a membership edge an explicit Play renews the card's scope; Open must pass the renewed
    // one so the host compares against what the card actually observes now.
    membership = 2; h.manager.reset(); button(host, "Play flipnote").click(); await wait(() => !host.querySelector("canvas")!.hidden);
    button(host, "Open").click(); assert.equal(opened.length, 3); assert.equal(opened[2].scope.membership, 2);
  } finally { h.destroy(); }
  const bare = harness(), plain = bare.create();
  try {
    bare.visible(plain); await wait(() => !plain.querySelector("canvas")!.hidden);
    assert.equal(button(plain, "Open"), undefined); assert.equal(button(plain, "open"), undefined);
  } finally { bare.destroy(); }
});

test("the facts line names the file and its size up front and adds frames, fps, sound and expiry once known", async () => {
  const h = harness({ expiryText: file => `expires 30d (${file.name})` }), host = h.create();
  const facts = () => host.querySelector(".pixa-facts")!.textContent!;
  try {
    assert.equal(host.querySelector(".pixa-facts .pixa-name")!.textContent, "loop.pixa");
    assert.match(facts(), /4\.9 kib/); assert.doesNotMatch(facts(), /frames/); assert.match(facts(), /expires 30d \(loop\.pixa\)/);
    assert.equal(host.querySelector<HTMLElement>(".pixa-sounds")!.hidden, true);
    h.visible(host); await wait(() => !host.querySelector("canvas")!.hidden);
    assert.match(facts(), /3 frames · 10 fps · 4\.9 kib/); assert.match(facts(), /♪ 1 sound(?!s)/, "one marker is one sound, not one sounds");
    assert.equal(facts().includes("♫ music"), value.take.events.length > 0, "music is claimed only when the take carries events");
    assert.equal(host.querySelector<HTMLElement>(".pixa-sounds")!.hidden, false);
    // Content behind a cid is immutable, so the facts survive the package's release.
    h.visible(host, false); assert.match(facts(), /3 frames · 10 fps/);
  } finally { h.destroy(); }
  const bare = harness({ expiryText: () => "" }), plain = bare.create();
  try { assert.equal(plain.querySelectorAll(".pixa-facts .micro").length, 2, "an empty expiry adds no fact"); } finally { bare.destroy(); }
});

test("data-state follows idle, loading, ready, error and blocked so the veil can be styled truthfully", async () => {
  const pending = deferred<typeof value>(); let reads = 0;
  const h = harness({ reader: { read: () => ++reads === 1 ? pending.promise : Promise.reject(new Error("gone")), cancel() {} } }), host = h.create();
  try {
    assert.equal(host.dataset.state, "idle"); assert.equal(button(host, "Play flipnote").hidden, false);
    h.visible(host); assert.equal(host.dataset.state, "loading"); assert.match(host.textContent!, /fetching 4\.9 kib/);
    assert.equal(button(host, "Play flipnote").hidden, true);
    pending.resolve(value); await wait(() => host.dataset.state === "ready"); assert.doesNotMatch(host.textContent!, /fetching/);
    h.visible(host, false); assert.equal(host.dataset.state, "idle");
    h.visible(host); await wait(() => host.dataset.state === "error");
    assert.match(host.textContent!, /Couldn’t load this flipnote/); assert.equal(button(host, "Play flipnote").hidden, false);
    assert.equal(host.querySelector("canvas")!.hidden, true);
    const big = h.manager.create({ cid: "f".repeat(64), name: "huge.pixa", size: PIXA_MAX_BYTES + 1 }, scope); document.body.append(big);
    try {
      assert.equal(big.dataset.state, "blocked"); assert.match(big.textContent!, /too large to preview/); assert.equal(button(big, "Play flipnote").disabled, true);
      h.visible(big); await settle(2); assert.equal(big.dataset.state, "blocked"); assert.equal(reads, 2, "an oversized card never reads");
    } finally { big.remove(); }
  } finally { h.destroy(); }
});
