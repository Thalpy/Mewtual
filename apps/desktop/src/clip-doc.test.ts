import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_AUDIO_ITEMS, MAX_KEYFRAMES, MAX_OVERLAYS, MAX_VIDEO_LAYERS, MIN_ITEM_SECONDS, PATH_TOLERANCE, addAudio, addImage,
  addLayer, addText, baseLayer, clearPath, clipDuration, editText, isUntouched, layerSourceTime, layerSpan, moveLayer, needsCompositor,
  newClipDoc, overlaysAt, placeAt, placementAt, recordPath, removeAudio, removeLayer, removeOverlay, setOverlayTimes,
  simplifyPath, transformOverlay, updateAudio, updateLayer, withBaseEdit, type ClipDoc, type PathSample,
} from "./clip-doc.ts";
import { initialEdit, setInPoint, setOutPoint, setRemoved, splitAt } from "./video-edit.ts";

function near(actual: number, expected: number, message = "") {
  assert.ok(Math.abs(actual - expected) < 1e-6, `${message} expected ${expected}, got ${actual}`);
}
const doc10 = () => newClipDoc("src:clip", 10);
const only = <T,>(xs: readonly T[]) => { assert.equal(xs.length, 1); return xs[0]; };

// --- The document as a whole --------------------------------------------------------------------

test("a new document is the untouched clip and needs nothing rendered", () => {
  const d = doc10();
  assert.equal(clipDuration(d), 10);
  assert.equal(isUntouched(d), true);
  assert.equal(needsCompositor(d), false);
  assert.equal(baseLayer(d).box.w, 1);
});

test("cuts alone change the clip but keep the light path; anything added needs every frame drawn", () => {
  const cut = withBaseEdit(doc10(), setInPoint(initialEdit(10), 2));
  assert.equal(clipDuration(cut), 8, "the clip is what layer 0 keeps");
  assert.equal(isUntouched(cut), false);
  assert.equal(needsCompositor(cut), false);
  assert.equal(needsCompositor(addText(doc10(), 0, "hi")), true);
  assert.equal(needsCompositor(addLayer(doc10(), "src:b", 5, 0)), true);
  assert.equal(needsCompositor(addAudio(doc10(), "sfx", "kit:snare", 0.4, 1)), true);
  const muted = updateLayer(doc10(), "v1", { muted: true });
  assert.equal(needsCompositor(muted), true, "muting the clip's own sound re-renders its audio");
});

test("only what reaches the posted clip forces a full re-render", () => {
  // Found in review: an item wholly past a trimmed end, or a muted sound, re-rendered the clip.
  let d = addText(doc10(), 8, "late");
  d = addAudio(d, "sfx", "pow", 0.4, 9);
  d = withBaseEdit(d, setOutPoint(initialEdit(10), 5));
  assert.equal(needsCompositor(d), false, "both start after the clip now ends");
  const muted = updateAudio(addAudio(doc10(), "sfx", "pow", 0.4, 1), "a2", { muted: true });
  assert.equal(needsCompositor(muted), false, "a muted sound adds nothing to hear");
  assert.equal(isUntouched(muted), true);
  assert.equal(needsCompositor(updateAudio(muted, "a2", { muted: false })), true);
});

test("replacing the base cut list with the same object is not an edit", () => {
  const d = doc10();
  assert.equal(withBaseEdit(d, baseLayer(d).edit), d);
});

// --- Overlays -----------------------------------------------------------------------------------

test("text and images get a window inside the clip and deterministic ids", () => {
  let d = addText(doc10(), 8.5, "Hello", "wave", "#ff0000");
  d = addImage(d, -3, "img:1", 2);
  const [text, image] = d.overlays;
  assert.equal(text.id, "o2");
  assert.equal(image.id, "o3");
  near(text.start, 8.5);
  near(text.end, 10, "cut at the clip's end");
  near(image.start, 0, "a negative time is held at the start");
  near(image.end, 3);
  assert.deepEqual(text.content, { kind: "text", text: "Hello", effect: "wave", color: "#ff0000" });
  assert.equal(addText(d, 0, "   "), d, "empty text is refused");
  assert.equal(addImage(d, 0, "img:2", 0), d, "a shapeless image is refused");
});

test("the number of overlays is bounded", () => {
  let d = doc10();
  for (let i = 0; i < MAX_OVERLAYS + 3; i++) d = addText(d, i % 9, `t${i}`);
  assert.equal(d.overlays.length, MAX_OVERLAYS);
});

test("editing text keeps what was not changed and refuses empty words or a bad colour", () => {
  const d = addText(doc10(), 0, "Hi");
  const id = d.overlays[0].id;
  const e = editText(d, id, { effect: "neon", color: "not-a-colour" });
  assert.deepEqual(e.overlays[0].content, { kind: "text", text: "Hi", effect: "neon", color: "#ffffff" });
  assert.equal(editText(e, id, { text: "  " }), e);
  assert.equal(editText(e, id, { effect: "neon" }), e, "no change, no step");
});

test("moving an overlay on the timeline carries its path with it", () => {
  let d = addText(doc10(), 1, "Hi");
  const id = d.overlays[0].id;
  d = recordPath(d, id, [{ t: 1, x: 0.1, y: 0.1 }, { t: 2, x: 0.9, y: 0.1 }]);
  d = setOverlayTimes(d, id, 3, 6);
  const o = d.overlays[0];
  near(o.start, 3);
  near(o.end, 6);
  assert.deepEqual(o.keyframes.map((k) => k.t), [3, 4]);
  assert.equal(setOverlayTimes(d, id, 3, 6), d);
  const tiny = setOverlayTimes(d, id, 9.99, 9.99);
  near(tiny.overlays[0].start, 10 - MIN_ITEM_SECONDS);
  near(tiny.overlays[0].end, 10);
});

test("trimming an overlay's ends leaves its path where it was in time", () => {
  let d = addText(doc10(), 1, "Hi");
  const id = d.overlays[0].id;
  d = recordPath(d, id, [{ t: 1, x: 0.1, y: 0.1 }, { t: 3, x: 0.9, y: 0.1 }]);
  const trimmed = setOverlayTimes(d, id, 2, 3.5, false);
  const o = trimmed.overlays[0];
  near(o.start, 2);
  near(o.end, 3.5);
  assert.deepEqual(o.keyframes.map((k) => k.t), [1, 3], "a trim does not drag the motion along");
  near(placementAt(o, 2).x, 0.5, "the overlay now starts half way along its path");
});

test("clearing a path leaves the overlay standing where it is at that moment", () => {
  let d = addImage(doc10(), 0, "img:1", 1);
  const id = d.overlays[0].id;
  assert.equal(clearPath(d, id, 1), d, "a still overlay has no path to clear");
  d = recordPath(d, id, [{ t: 0, x: 0.1, y: 0.2 }, { t: 2, x: 0.9, y: 0.2 }]);
  const still = clearPath(d, id, 1);
  const o = only(still.overlays);
  assert.equal(o.keyframes.length, 1);
  near(o.keyframes[0].x, 0.5);
  near(o.keyframes[0].t, o.start);
});

test("overlaysAt lists only what shows at that moment", () => {
  let d = addText(doc10(), 0, "a");
  d = addText(d, 5, "b");
  assert.deepEqual(overlaysAt(d, 1).map((o) => (o.content as { text: string }).text), ["a"]);
  assert.deepEqual(overlaysAt(d, 3).map((o) => o.id), [], "a ended at 3");
  assert.deepEqual(overlaysAt(d, 5.5).map((o) => (o.content as { text: string }).text), ["b"]);
});

// --- Motion ---------------------------------------------------------------------------------------

test("placement is held before the first keyframe and after the last, and straight in between", () => {
  let d = addImage(doc10(), 0, "img", 1);
  const id = d.overlays[0].id;
  d = recordPath(d, id, [{ t: 1, x: 0, y: 0 }, { t: 3, x: 1, y: 0.5 }]);
  const o = d.overlays[0];
  // The place it had at its start (centre) is kept before the recorded stretch.
  near(placementAt(o, 0).x, 0.5);
  near(placementAt(o, 0.5).x, 0.25, "straight from where it was to where the recording began");
  near(placementAt(o, 2).x, 0.5);
  near(placementAt(o, 2).y, 0.25);
  near(placementAt(o, 9).x, 1, "held after the last keyframe");
});

test("the binary search agrees with a straight scan over a long path", () => {
  let d = addImage(doc10(), 0, "img", 1);
  const id = d.overlays[0].id;
  // A zig-zag that cannot be simplified away.
  const samples: PathSample[] = Array.from({ length: 200 }, (_, i) => ({ t: i * 0.015, x: i % 2 ? 0.8 : 0.2, y: (i % 7) / 7 }));
  d = recordPath(d, id, samples);
  const o = d.overlays[0];
  assert.ok(o.keyframes.length > 150);
  for (let t = 0; t < 3; t += 0.0137) {
    let i = 0;
    while (i + 1 < o.keyframes.length && o.keyframes[i + 1].t <= t) i++;
    const a = o.keyframes[i], b = o.keyframes[Math.min(i + 1, o.keyframes.length - 1)];
    const f = b.t === a.t ? 0 : Math.min(1, Math.max(0, (t - a.t) / (b.t - a.t)));
    near(placementAt(o, t).x, t <= o.keyframes[0].t ? o.keyframes[0].x : a.x + (b.x - a.x) * f, `t=${t}`);
  }
});

test("dragging a still overlay while paused just moves it", () => {
  let d = addText(doc10(), 0, "Hi");
  const id = d.overlays[0].id;
  d = placeAt(d, id, 1.5, { x: 0.2, y: 0.3 });
  const k = only(d.overlays[0].keyframes);
  near(k.x, 0.2);
  near(k.y, 0.3);
  near(k.scale, 0.12, "size is kept");
  assert.equal(placeAt(d, id, 1.5, { x: 0.2, y: 0.3 }), d, "no move, no step");
});

test("dragging an overlay with a path while paused bends the path through a keyframe at that moment", () => {
  let d = addText(doc10(), 0, "Hi");
  const id = d.overlays[0].id;
  d = recordPath(d, id, [{ t: 0, x: 0, y: 0 }, { t: 2, x: 1, y: 0 }]);
  d = placeAt(d, id, 1, { x: 0.5, y: 0.9 });
  assert.deepEqual(d.overlays[0].keyframes.map((k) => k.t), [0, 1, 2]);
  near(placementAt(d.overlays[0], 1).y, 0.9);
  d = placeAt(d, id, 1.005, { x: 0.5, y: 0.1 });
  assert.deepEqual(d.overlays[0].keyframes.map((k) => k.t), [0, 1.005, 2], "a keyframe at the same moment is replaced, not doubled");
});

test("recording replaces only the recorded stretch of the path, keeps size, and stays inside the overlay", () => {
  let d = addImage(doc10(), 0, "img", 1);
  const id = d.overlays[0].id;
  d = transformOverlay(d, id, 2, 0);
  d = recordPath(d, id, [{ t: 0, x: 0.1, y: 0.1 }, { t: 1, x: 0.2, y: 0.2 }, { t: 2, x: 0.3, y: 0.3 }, { t: 2.9, x: 0.4, y: 0.4 }]);
  // Re-record only 1..2: the samples from 0 and 2.9 must survive.
  d = recordPath(d, id, [{ t: 1, x: 0.9, y: 0.9 }, { t: 1.5, x: 0.5, y: 0.1 }, { t: 2, x: 0.9, y: 0.9 }, { t: 7, x: 0, y: 0 }]);
  const o = d.overlays[0];
  const ts = o.keyframes.map((k) => k.t);
  assert.ok(ts.includes(0) && ts.includes(2.9), ts.join(","));
  assert.ok(!ts.includes(7), "samples past the overlay's end are dropped");
  near(placementAt(o, 1.5).x, 0.5);
  for (const k of o.keyframes) near(k.scale, 0.6, "size follows what it had");
});

test("a recording with nothing inside the overlay's time changes nothing", () => {
  const d = addText(doc10(), 0, "Hi");
  const id = d.overlays[0].id;
  assert.equal(recordPath(d, id, [{ t: 8, x: 0, y: 0 }]), d);
  assert.equal(recordPath(d, id, []), d);
  assert.equal(recordPath(d, id, [{ t: Number.NaN, x: 0, y: 0 }]), d);
});

test("simplifying keeps the ends, keeps corners and speed changes, and drops points on a steady line", () => {
  const steady: PathSample[] = Array.from({ length: 50 }, (_, i) => ({ t: i / 50, x: i / 50, y: 0.5 }));
  assert.equal(simplifyPath(steady, PATH_TOLERANCE).length, 2);
  const corner: PathSample[] = [...steady.slice(0, 25), ...Array.from({ length: 25 }, (_, i) => ({ t: 0.5 + i / 50, x: 0.5, y: 0.5 + i / 50 }))];
  assert.ok(simplifyPath(corner, PATH_TOLERANCE).some((s) => Math.abs(s.t - 0.48) < 0.03), "the corner survives");
  // Same straight line, but it pauses halfway: time is the parameter, so the pause must survive.
  const pausing: PathSample[] = [
    ...Array.from({ length: 10 }, (_, i) => ({ t: i / 10, x: i / 20, y: 0 })),
    ...Array.from({ length: 10 }, (_, i) => ({ t: 1 + i / 10, x: 0.5, y: 0 })),
    ...Array.from({ length: 10 }, (_, i) => ({ t: 2 + i / 10, x: 0.5 + i / 20, y: 0 })),
  ];
  assert.ok(simplifyPath(pausing, PATH_TOLERANCE).length > 2, "a stop on a straight line is not 'on the line' in time");
});

test("a very long recording is held to the keyframe cap, ends included", () => {
  let d = addImage(doc10(), 0, "img", 1);
  const id = d.overlays[0].id;
  d = setOverlayTimes(d, id, 0, 10);
  const samples: PathSample[] = Array.from({ length: 3000 }, (_, i) => ({ t: i / 300, x: (i * 7919) % 100 / 100, y: (i * 104729) % 100 / 100 }));
  d = recordPath(d, id, samples);
  const ks = d.overlays[0].keyframes;
  assert.equal(ks.length, MAX_KEYFRAMES);
  near(ks[0].t, 0);
  near(ks[ks.length - 1].t, 2999 / 300);
});

test("scaling and turning apply along the whole path and stay in bounds", () => {
  let d = addText(doc10(), 0, "Hi");
  const id = d.overlays[0].id;
  d = transformOverlay(d, id, 100, 30);
  const k = only(d.overlays[0].keyframes);
  near(k.scale, 2, "held at the maximum size");
  near(k.rotation, 30);
  assert.equal(transformOverlay(d, id, 1, 0), d);
  assert.equal(transformOverlay(d, id, -1, 0), d);
});

test("removing an overlay", () => {
  const d = addText(doc10(), 0, "Hi");
  assert.equal(removeOverlay(d, "nope"), d);
  assert.equal(removeOverlay(d, d.overlays[0].id).overlays.length, 0);
});

// --- Video layers -------------------------------------------------------------------------------

test("up to three video layers, and the clip's own layer cannot be removed or moved", () => {
  let d = doc10();
  d = addLayer(d, "src:b", 4, 2);
  d = addLayer(d, "src:c", 4, 20);
  assert.equal(d.layers.length, MAX_VIDEO_LAYERS);
  assert.equal(addLayer(d, "src:d", 4, 0), d);
  near(d.layers[2].start, 10 - MIN_ITEM_SECONDS, "placed inside the clip");
  assert.equal(removeLayer(d, "v1"), d);
  const moved = updateLayer(d, "v1", { start: 3, box: { x: 0.5, y: 0.5, w: 0.1, h: 0.1 } });
  assert.equal(moved, d, "layer 0 keeps its place and fills the frame");
  assert.equal(removeLayer(d, d.layers[1].id).layers.length, 2);
});

test("added layers swap drawing order between themselves; the clip stays the backdrop", () => {
  let d = addLayer(addLayer(doc10(), "src:b", 4, 0), "src:c", 4, 0);
  const [, b, c] = d.layers.map((l) => l.id);
  d = moveLayer(d, b, 1);
  assert.deepEqual(d.layers.map((l) => l.id), ["v1", c, b], "b now draws over c");
  assert.equal(moveLayer(d, b, 1), d, "already in front");
  assert.equal(moveLayer(d, c, -1), d, "nothing goes behind the clip");
  assert.equal(moveLayer(d, "v1", 1), d, "the clip itself never moves");
});

test("an added layer shows for what it keeps, at its own source times, cut to the clip", () => {
  let d = withBaseEdit(doc10(), setInPoint(initialEdit(10), 4)); // the clip is 6 s
  d = addLayer(d, "src:b", 5, 3);
  let layer = d.layers[1];
  // Cut the added clip: keep [0,1] and [2,5] of its own 5 s.
  layer = { ...layer, edit: setRemoved(splitAt(splitAt(initialEdit(5), 1), 2), 1, true) };
  d = updateLayer(d, layer.id, { edit: layer.edit });
  layer = d.layers[1];
  assert.deepEqual(layerSpan(d, layer), { start: 3, end: 6 }, "4 s of kept clip, cut at the clip's 6 s end");
  assert.equal(layerSourceTime(d, layer, 2.9), null);
  near(layerSourceTime(d, layer, 3.5)!, 0.5);
  near(layerSourceTime(d, layer, 4.5)!, 2.5, "past its own cut");
  assert.equal(layerSourceTime(d, layer, 6), null);
  const base = d.layers[0];
  near(layerSourceTime(d, base, 0)!, 4, "layer 0 maps through its own trim");
});

test("layer volume and box are kept in range", () => {
  let d = addLayer(doc10(), "src:b", 4, 0);
  const id = d.layers[1].id;
  d = updateLayer(d, id, { volume: 3, box: { x: 2, y: -2, w: 9, h: 0.001 } });
  const l = d.layers[1];
  near(l.volume, 1);
  assert.ok(l.box.w <= 1 && l.box.h >= 0.05 && l.box.x <= 0.95 && l.box.y >= -l.box.h + 0.05);
});

// --- Audio --------------------------------------------------------------------------------------

test("audio is placed inside the clip and trimmed to fit it", () => {
  let d = addAudio(doc10(), "music", "song:1", 60, 4);
  const a = d.audio[0];
  near(a.start, 4);
  near(a.length, 6, "a long song plays to the clip's end");
  near(a.volume, 0.8, "music starts a little under the clip's own sound");
  d = addAudio(d, "sfx", "kit:kick", 0.3, 9.95);
  near(d.audio[1].start, 10 - MIN_ITEM_SECONDS, "too close to the end starts early enough to be heard");
  near(d.audio[1].length, MIN_ITEM_SECONDS);
  let full = doc10();
  for (let i = 0; i < MAX_AUDIO_ITEMS + 2; i++) full = addAudio(full, "sfx", "kit:hat", 0.1, i / 10);
  assert.equal(full.audio.length, MAX_AUDIO_ITEMS);
});

test("trimming and moving audio keeps it inside the sound and the clip", () => {
  let d = addAudio(doc10(), "music", "song:1", 8, 0);
  const id = d.audio[0].id;
  d = updateAudio(d, id, { offset: 7.95, length: 5 });
  near(d.audio[0].offset, 8 - MIN_ITEM_SECONDS);
  near(d.audio[0].length, MIN_ITEM_SECONDS, "only what is left of the sound");
  d = updateAudio(d, id, { offset: 2, length: 50, start: 5 });
  near(d.audio[0].length, 5, "and only what fits before the clip ends");
  assert.equal(updateAudio(d, id, { volume: d.audio[0].volume }), d);
  near(updateAudio(d, id, { volume: -1 }).audio[0].volume, 0);
  assert.equal(removeAudio(d, id).audio.length, 0);
});

test("a volume or mute change never cuts a sound to a clip that is shorter for now", () => {
  // Found in review: muting and unmuting after cutting the clip down left the music cut to the
  // short clip for good, and a sound past the new end with a NEGATIVE length that never played.
  let d = addAudio(doc10(), "music", "song", 10, 0);
  d = addAudio(d, "sfx", "pow", 0.5, 8);
  const [song, pow] = d.audio.map((a) => a.id);
  d = withBaseEdit(d, setOutPoint(initialEdit(10), 5));
  d = updateAudio(d, song, { muted: true });
  d = updateAudio(d, song, { muted: false });
  d = updateAudio(d, pow, { volume: 0.5 });
  d = withBaseEdit(d, initialEdit(10));
  near(d.audio[0].length, 10, "the music is whole again once the clip is");
  near(d.audio[1].length, 0.5, "the sound past the old end kept its length");
  assert.ok(d.audio.every((a) => a.length > 0));
  // A real move or trim still fits the sound to the clip as it is.
  const cut = withBaseEdit(d, setOutPoint(initialEdit(10), 5));
  const moved = updateAudio(cut, song, { start: 1 });
  near(moved.audio[0].length, 4);
});

test("documents are plain data that survive a JSON round trip", () => {
  let d: ClipDoc = addText(doc10(), 1, "Hi", "wave");
  d = addAudio(d, "sfx", "kit:clap", 0.2, 2);
  d = addLayer(d, "src:b", 3, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(d)), d);
});
