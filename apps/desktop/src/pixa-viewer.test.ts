import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { decodePixa, encodePixa, type Pixa } from "./pixa.ts";
import { pixaFrameAt } from "./pixa-media.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import type { StudioAudioState } from "./studio-audio.ts";
import { FLIPNOTE_FPS_MAX, FLIPNOTE_FPS_MIN } from "./studio-contract.ts";
import {
  PIXA_VIEWER_THUMB_LIMIT, clampFrame, formatKib, formatSeconds, frameOffsetMs, frameProbeMs, isSilentPixa,
  sfxCountByFrame, viewerFacts, viewerProvenance, visualElapsedAt,
} from "./pixa-viewer.ts";
import { deferred } from "./studio-testkit.ts";

// --- Pure helpers ------------------------------------------------------------------------------------

test("frameProbeMs lands inside its frame for every legal fps where the exact start can round under it", () => {
  for (let fps = FLIPNOTE_FPS_MIN; fps <= FLIPNOTE_FPS_MAX; fps++) {
    const frames = new Array(fps * 3).fill(new Uint8Array(1));
    for (let frame = 0; frame < frames.length; frame++) {
      assert.equal(pixaFrameAt({ fps, frames } as unknown as Pixa, frameProbeMs(frame, fps)), frame, `fps ${fps} frame ${frame}`);
    }
  }
  assert.equal(frameOffsetMs(1, 10), 100);
  assert.equal(frameOffsetMs(0, 3), 0);
});

test("visualElapsedAt wraps with loop, ends at the track length without it, and ignores a backwards clock", () => {
  assert.deepEqual(visualElapsedAt({ baseMs: 100, originMs: 1000, loop: true }, 1250, 300), { elapsedMs: 50, ended: false });
  assert.deepEqual(visualElapsedAt({ baseMs: 100, originMs: 1000, loop: false }, 1150, 300), { elapsedMs: 250, ended: false });
  assert.deepEqual(visualElapsedAt({ baseMs: 100, originMs: 1000, loop: false }, 1200, 300), { elapsedMs: 300, ended: true });
  assert.deepEqual(visualElapsedAt({ baseMs: 100, originMs: 1000, loop: false }, 500, 300), { elapsedMs: 100, ended: false });
  assert.deepEqual(visualElapsedAt({ baseMs: 0, originMs: 0, loop: true }, 50, 0), { elapsedMs: 0, ended: true });
});

test("clampFrame, counts, facts and provenance formatters", () => {
  assert.equal(clampFrame(-1, 3), 0); assert.equal(clampFrame(7, 3), 2); assert.equal(clampFrame(1.9, 3), 1); assert.equal(clampFrame(2, 0), 0);
  const sfx = [{ frame: 1, patch: 0, note: 60, durationMs: 1 }, { frame: 1, patch: 0, note: 62, durationMs: 1 }, { frame: 0, patch: 0, note: 60, durationMs: 1 }];
  assert.deepEqual([...sfxCountByFrame({ sfx })], [[1, 2], [0, 1]]);
  assert.equal(sfxCountByFrame(null).size, 0);
  assert.equal(isSilentPixa({ sfx: [], take: { events: [] } as unknown as Pixa["take"] }), true);
  assert.equal(isSilentPixa({ sfx, take: { events: [] } as unknown as Pixa["take"] }), false);
  assert.equal(formatSeconds(250), "0.25"); assert.equal(formatSeconds(NaN), "0.00"); assert.equal(formatSeconds(-5), "0.00");
  assert.equal(formatKib(4198), "4.1 kib"); assert.equal(formatKib(2 * 1024 * 1024), "2.0 mib");
  assert.equal(viewerFacts({ fps: 8, frames: [new Uint8Array(1), new Uint8Array(1), new Uint8Array(1)] }, 4198), "flipnote · 192×144 · 3 frames · 8 fps · 4.1 kib");
  assert.equal(viewerFacts({ fps: 8, frames: [new Uint8Array(1)] }, 0), "flipnote · 192×144 · 1 frame · 8 fps");
  assert.equal(viewerFacts(null, 512), "flipnote · 0.5 kib");
  assert.equal(viewerProvenance("rowan", "#general", "expires 30d"), "by rowan · in #general · expires 30d");
  assert.equal(viewerProvenance("", "#general", ""), "in #general");
  assert.equal(viewerProvenance("", "", ""), "");
});

// --- Component -----------------------------------------------------------------------------------------

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
class Pixels {
  readonly data: Uint8ClampedArray; readonly width: number; readonly height: number;
  constructor(data: Uint8ClampedArray, width: number, height: number) { this.data = data; this.width = width; this.height = height; }
}
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, Event: w.Event, KeyboardEvent: w.KeyboardEvent, ImageData: Pixels });
Object.defineProperty(w.HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ({ putImageData() {}, drawImage() {}, clearRect() {}, imageSmoothingEnabled: true }) });
const { mount, unmount, flushSync } = await import("svelte");
const { default: Harness } = await import("../scripts/fixtures/PixaViewerHarness.svelte");

const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const original = decodePixa(Uint8Array.from(Buffer.from(vector.pixaHex, "hex")));
/** Three frames at 10 fps (100 ms each, 300 ms track) with one sound marker on the middle frame. */
const value = decodePixa(encodePixa({ ...original, fps: 10, frames: [original.frames[0], original.frames[0], original.frames[0]],
  take: { ...original.take, patches: [legacyJamPatch("sine")] }, sfx: [{ frame: 1, patch: 0, note: 60, durationMs: 333 }] }));
const silentValue = decodePixa(encodePixa({ ...value, take: { ...value.take, events: [] }, sfx: [] }));
const withFrames = (count: number) => decodePixa(encodePixa({ ...value, frames: new Array(count).fill(original.frames[0]), sfx: [] }));
const stopped: StudioAudioState = { state: "stopped", elapsedMs: 0 };

const sleep = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
async function wait(predicate: () => boolean, what = "expected state") {
  for (let i = 0; i < 100; i++) { if (predicate()) return; await sleep(5); }
  assert.fail(`viewer did not reach ${what}`);
}
type HarnessProps = Parameters<InstanceType<typeof Harness>["set"]>[0];
function harness(overrides: HarnessProps = {}) {
  const target = document.createElement("div"); document.body.append(target);
  let now = 0, nowReads = 0;
  const plays: Array<[number, boolean]> = [], pending: Array<ReturnType<typeof deferred<boolean>>> = [];
  const counts = { stop: 0, close: 0, download: 0 };
  const app = mount(Harness, { target, props: {
    name: "moon cat.pixa", pixa: value, bytes: 4198, author: "rowan", place: "#general", expiry: "expires 30d", audioState: stopped,
    onplay: (offset: number, loop: boolean) => { plays.push([offset, loop]); const d = deferred<boolean>(); pending.push(d); return d.promise; },
    onstop: () => { counts.stop++; }, ondownload: () => { counts.download++; }, onclose: () => { counts.close++; },
    now: () => { nowReads++; return now; }, ...overrides,
  } });
  flushSync();
  const button = (text: string) => [...target.querySelectorAll("button")].find(b => b.textContent === text || b.getAttribute("aria-label") === text)!;
  return {
    app, target, plays, pending, counts,
    canvas: () => target.querySelector<HTMLCanvasElement>('[aria-label="Flipnote viewer"] .stage canvas')!,
    frame: () => target.querySelector<HTMLCanvasElement>('[aria-label="Flipnote viewer"] .stage canvas')?.dataset.pixaFrame,
    button, click: (text: string) => { button(text).click(); flushSync(); },
    key: (key: string, on: Element | null = null) => { (on ?? target.querySelector('[role="dialog"]')!).dispatchEvent(new w.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true })); flushSync(); },
    status: () => target.querySelector(".transport .chip .micro")!.textContent,
    setNow: (ms: number) => { now = ms; }, nowReads: () => nowReads,
    set: (next: HarnessProps) => { app.set(next); flushSync(); },
    destroy: async () => { await unmount(app); target.remove(); },
  };
}

test("stepping changes the shown frame, clamps at both ends and the readout follows", async () => {
  const h = harness();
  try {
    assert.equal(h.frame(), "0"); assert.match(h.target.textContent!, /1 \/ 3/); assert.match(h.target.textContent!, /0\.00 s of 0\.30 s/);
    h.click("Next frame"); assert.equal(h.frame(), "1"); assert.match(h.target.textContent!, /2 \/ 3/); assert.match(h.target.textContent!, /0\.10 s of 0\.30 s/);
    h.click("Next frame"); assert.equal(h.frame(), "2");
    h.click("Next frame"); assert.equal(h.frame(), "2", "next at the last frame holds rather than wrapping");
    h.click("Previous frame"); assert.equal(h.frame(), "1");
    h.click("First frame"); assert.equal(h.frame(), "0");
    h.click("Previous frame"); assert.equal(h.frame(), "0");
    h.click("Last frame"); assert.equal(h.frame(), "2");
    h.key("Home"); assert.equal(h.frame(), "0"); h.key("ArrowRight"); assert.equal(h.frame(), "1"); h.key("End"); assert.equal(h.frame(), "2"); h.key("ArrowLeft"); assert.equal(h.frame(), "1");
    assert.equal(h.counts.stop, 0, "stepping while nothing runs never asks the host to stop");
    assert.equal(h.plays.length, 0);
  } finally { await h.destroy(); }
});

test("Play with sound on asks the host from the held frame's time and the picture follows the host clock while playing", async () => {
  const h = harness();
  try {
    h.click("Next frame");
    assert.equal(h.button("Play").textContent, "Play");
    h.click("Play");
    assert.deepEqual(h.plays, [[100, true]]);
    assert.equal(h.button("Pause").textContent, "Pause"); assert.equal(h.status(), "preparing");
    assert.equal(h.frame(), "1", "the held frame stays on screen while the host prepares");
    h.set({ audioState: { state: "preparing", elapsedMs: 0 } });
    assert.equal(h.frame(), "1", "a preparing host reports elapsed 0; following it would flash the first frame");
    h.set({ audioState: { state: "playing", elapsedMs: 250 } }); h.pending[0].resolve(true); await sleep(0); flushSync();
    assert.equal(h.frame(), "2"); assert.equal(h.status(), "playing"); assert.match(h.target.textContent!, /0\.25 s of 0\.30 s/);
    h.set({ audioState: { state: "playing", elapsedMs: 50 } }); assert.equal(h.frame(), "0");
    // A looping run keeps wrapping the pictures while the host's audio runs on through a sound tail.
    h.set({ audioState: { state: "playing", elapsedMs: 950 } }); assert.equal(h.frame(), "0"); assert.match(h.target.textContent!, /0\.05 s of 0\.30 s/, "readout wraps with the pictures");
    assert.equal(h.target.querySelector('[aria-current="true"]')!.getAttribute("aria-label"), "Frame 1", "the strip agrees with the wrapped frame");
    assert.ok(h.target.querySelector(".sfx-chip") === null, "no marker chip on a frame without sound");
    h.set({ audioState: { state: "playing", elapsedMs: 150 } }); assert.equal(h.target.querySelector(".sfx-chip")!.textContent, "♪ 1");
    // The host stopping on its own (loop off reaching the end) holds the last shown frame.
    h.set({ audioState: stopped });
    assert.equal(h.button("Play").textContent, "Play"); assert.equal(h.frame(), "1"); assert.equal(h.status(), "paused");
    assert.equal(h.counts.stop, 0, "a host-initiated stop is not echoed back");
    // A one-shot run (loop off) is passed to the host as such, holds the last frame through the
    // tail and clamps the readout; the host's natural stop then arms "play again" from the start.
    // This host never resolves the second `onplay`: a stop that lands while the promise is still
    // pending must still end the run on the host's evidence of having played.
    h.click("Loop"); h.click("Play"); assert.deepEqual(h.plays[1], [100, false]);
    h.set({ audioState: { state: "playing", elapsedMs: 900 } }); assert.equal(h.frame(), "2"); assert.match(h.target.textContent!, /0\.30 s of 0\.30 s/, "a one-shot readout never exceeds the picture track");
    h.set({ audioState: stopped }); assert.equal(h.frame(), "2");
    h.click("Play"); assert.deepEqual(h.plays[2], [0, false], "Play after a finished one-shot run starts again");
  } finally { await h.destroy(); }
});

test("sound off: Play advances frames from the injected clock, loops, and Pause holds the frame", async () => {
  const h = harness();
  try {
    h.click("Sound"); assert.equal(h.button("Sound").getAttribute("aria-pressed"), "false");
    h.click("Play"); assert.equal(h.plays.length, 0, "the visual source never touches the audio host");
    assert.equal(h.button("Pause").textContent, "Pause"); assert.equal(h.status(), "playing");
    h.setNow(150); await wait(() => h.frame() === "1", "frame 1 at 150 ms");
    h.setNow(350); await wait(() => h.frame() === "0", "wrap at 300 ms");
    h.setNow(520); await wait(() => h.frame() === "2", "frame 2 at 220 ms into the second lap");
    h.click("Pause"); assert.equal(h.frame(), "2"); assert.equal(h.status(), "paused");
    const reads = h.nowReads(); h.setNow(2000); await sleep(100);
    assert.equal(h.frame(), "2"); assert.equal(h.nowReads(), reads, "the timer chain stops with the run");
    assert.equal(h.counts.stop, 0);
  } finally { await h.destroy(); }
});

test("loop off: the visual run stops at the last frame and the next Play restarts from the first", async () => {
  const h = harness();
  try {
    h.click("Sound"); h.click("Loop"); assert.equal(h.button("Loop").getAttribute("aria-pressed"), "false");
    h.click("Play"); assert.equal(h.button("Loop").disabled, true, "loop is fixed for the length of a run");
    h.setNow(1000); await wait(() => h.frame() === "2" && !!h.button("Play"), "end of track");
    assert.equal(h.status(), "paused");
    const reads = h.nowReads(); await sleep(100); assert.equal(h.nowReads(), reads);
    h.click("Play"); assert.equal(h.frame(), "0", "Play after a finished run starts again");
    h.click("Pause");
  } finally { await h.destroy(); }
});

test("stepping or switching sound while the sound source runs stops the host once and a late answer is ignored", async () => {
  const h = harness();
  try {
    h.click("Play"); h.set({ audioState: { state: "playing", elapsedMs: 150 } }); assert.equal(h.frame(), "1");
    h.click("Next frame");
    assert.equal(h.counts.stop, 1); assert.equal(h.frame(), "2"); assert.equal(h.button("Play").textContent, "Play");
    h.set({ audioState: stopped }); assert.equal(h.frame(), "2", "the host's stop must not pull the picture back to the audio frame");
    h.pending[0].resolve(false); await sleep(0); flushSync();
    assert.doesNotMatch(h.target.textContent!, /could not start/, "a refusal for a run that was already stopped says nothing");
    assert.equal(h.frame(), "2");
    // Switching to picture-only mid-run stops the host and holds the frame the audio was on.
    h.click("Play"); assert.deepEqual(h.plays[1], [200, true]);
    h.set({ audioState: { state: "playing", elapsedMs: 50 } }); assert.equal(h.frame(), "0");
    h.click("Sound"); assert.equal(h.counts.stop, 2); assert.equal(h.frame(), "0"); assert.equal(h.button("Play").textContent, "Play");
    h.set({ audioState: stopped });
    h.click("Play"); assert.equal(h.plays.length, 2); h.setNow(150); await wait(() => h.frame() === "1", "visual run from the held frame");
    h.key("ArrowLeft"); assert.equal(h.frame(), "0"); assert.equal(h.counts.stop, 2, "stopping the visual source never calls the host");
    h.key(" "); assert.equal(h.button("Pause").textContent, "Pause"); h.key(" "); assert.equal(h.button("Play").textContent, "Play");
  } finally { await h.destroy(); }
});

test("a refused sound start explains itself and stays retryable; a thrown start is treated as refused", async () => {
  const h = harness();
  try {
    h.click("Play"); h.pending[0].resolve(false); await sleep(0); flushSync();
    assert.match(h.target.textContent!, /Sound could not start\. Press Play to try again\./);
    assert.equal(h.button("Play").textContent, "Play"); assert.equal(h.frame(), "0");
    h.click("Play"); assert.equal(h.plays.length, 2); assert.doesNotMatch(h.target.textContent!, /could not start/);
    h.pending[1].reject(new Error("engine")); await sleep(0); flushSync();
    assert.match(h.target.textContent!, /could not start/); assert.equal(h.status(), "paused");
  } finally { await h.destroy(); }
});

test("Escape, the close buttons and the scrim call onclose; the card itself does not", async () => {
  const h = harness();
  try {
    h.key("Escape"); assert.equal(h.counts.close, 1);
    h.click("Close flipnote viewer"); assert.equal(h.counts.close, 2);
    h.click("close"); assert.equal(h.counts.close, 3);
    h.target.querySelector<HTMLElement>('[role="dialog"]')!.click(); flushSync(); assert.equal(h.counts.close, 3);
    h.target.querySelector<HTMLElement>(".pixa-viewer")!.click(); flushSync(); assert.equal(h.counts.close, 4);
    h.click("download .pixa"); assert.equal(h.counts.download, 1);
    assert.equal(document.activeElement, h.target.querySelector('[role="dialog"]'), "the dialog takes focus on mount");
  } finally { await h.destroy(); }
});

test("deafened forces the visual source, explains it, and keeps picture playback working", async () => {
  const h = harness({ deafened: true });
  try {
    assert.match(h.target.textContent!, /Turn off Deafen to hear it\./);
    const sound = h.button("Sound"); assert.equal(sound.disabled, true); assert.equal(sound.getAttribute("aria-pressed"), "false");
    h.click("Play"); assert.equal(h.plays.length, 0);
    h.setNow(150); await wait(() => h.frame() === "1", "visual playback while deafened");
    // Deafen flipping on under a sound run stops the host and holds the frame.
    h.click("Pause"); h.set({ deafened: false }); assert.doesNotMatch(h.target.textContent!, /Turn off Deafen/);
    h.click("Play"); assert.equal(h.plays.length, 1); h.set({ audioState: { state: "playing", elapsedMs: 250 } }); assert.equal(h.frame(), "2");
    h.set({ deafened: true });
    assert.equal(h.counts.stop, 1); assert.equal(h.frame(), "2"); assert.equal(h.button("Play").textContent, "Play");
  } finally { await h.destroy(); }
});

test("a package with nothing to hear never asks the audio host", async () => {
  const h = harness({ pixa: silentValue });
  try {
    assert.equal(h.button("Sound").disabled, true); assert.equal(h.button("Sound").getAttribute("aria-pressed"), "false");
    h.click("Play"); assert.equal(h.plays.length, 0); assert.equal(h.status(), "playing");
    h.setNow(150); await wait(() => h.frame() === "1", "visual playback of a silent package");
  } finally { await h.destroy(); }
});

test("a null package shows the loading status, an error shows the alert, and arrival starts validated at frame 0", async () => {
  const h = harness({ pixa: null, bytes: 0 });
  try {
    assert.equal(h.target.querySelector('[role="status"].note')!.textContent, "Loading and validating animation…");
    assert.equal(h.frame(), undefined); assert.equal(h.button("Play").disabled, true);
    assert.doesNotMatch(h.target.textContent!, /validated/); assert.match(h.target.textContent!, /flipnote(?! · 192)/);
    h.set({ error: "pixa:v1: magic" });
    assert.equal(h.target.querySelector('[role="alert"]')!.textContent, "pixa:v1: magic"); assert.equal(h.target.querySelector(".stage"), null);
    h.set({ error: "", pixa: value, bytes: 4198 });
    assert.equal(h.frame(), "0"); assert.match(h.target.textContent!, /validated/);
    assert.match(h.target.textContent!, /flipnote · 192×144 · 3 frames · 10 fps · 4\.1 kib/);
    assert.match(h.target.textContent!, /by rowan · in #general · expires 30d/);
    assert.equal(h.button("Play").disabled, false);
  } finally { await h.destroy(); }
});

test("thumbnails mark the current frame and sound markers, select on click, and vanish above the cap", async () => {
  const h = harness();
  try {
    const thumbs = () => [...h.target.querySelectorAll<HTMLButtonElement>(".thumb")];
    assert.equal(thumbs().length, 3);
    assert.deepEqual(thumbs().map(t => t.classList.contains("cur")), [true, false, false]);
    assert.deepEqual(thumbs().map(t => !!t.querySelector(".pill")), [false, true, false]);
    thumbs()[2].click(); flushSync(); assert.equal(h.frame(), "2"); assert.equal(thumbs()[2].getAttribute("aria-current"), "true");
    h.set({ pixa: withFrames(PIXA_VIEWER_THUMB_LIMIT) }); assert.equal(thumbs().length, PIXA_VIEWER_THUMB_LIMIT); assert.equal(h.frame(), "0", "a new package starts over");
    h.set({ pixa: withFrames(PIXA_VIEWER_THUMB_LIMIT + 1) }); assert.equal(thumbs().length, 0); assert.equal(h.target.querySelector(".strip"), null);
    assert.equal(h.frame(), "0"); assert.match(h.target.textContent!, /49 frames/);
  } finally { await h.destroy(); }
});

test("unmounting mid-run drops the visual timer and disowns a pending sound start", async () => {
  const h = harness();
  h.click("Sound"); h.click("Play"); h.setNow(150); await wait(() => h.frame() === "1");
  await h.destroy();
  const reads = h.nowReads(); h.setNow(400); await sleep(100); assert.equal(h.nowReads(), reads, "no tick after unmount");
  const g = harness();
  g.click("Play"); await g.destroy(); g.pending[0].resolve(false); await sleep(0);
  assert.equal(g.counts.stop, 0, "unmount leaves host audio to the host; it stops it when it closes the viewer");
});
