import assert from "node:assert/strict";
import test from "node:test";
import { checkPalette, decodePix, encodePix } from "./pix.ts";
import { DEFAULT_PALETTE } from "./studio-store.ts";
import { paletteGroups } from "./studio-palette.ts";
import { instrumentName, noteName, soundLengthMs } from "./studio-sound-ui.ts";
import { legacyJamPatch } from "./jam-patch.ts";

test("new palettes offer white-to-black and a fixed purple within the existing 16 slots", () => {
  assert.equal(DEFAULT_PALETTE.length, 16); checkPalette(DEFAULT_PALETTE);
  assert.deepEqual(paletteGroups(DEFAULT_PALETTE).grayscale, [0, 6, 7, 1]);
  assert.deepEqual(DEFAULT_PALETTE[9], { role: 0, r: 0xa6, g: 0x74, b: 0xe8 });
});
test("palette display order never remaps pixels or rewrites older authored colours", () => {
  const palette = [{role: 0, r: 0, g: 0, b: 0}, {role: 0, r: 255, g: 255, b: 255},
    {role: 0, r: 22, g: 88, b: 44}, {role: 3, r: 90, g: 30, b: 120}];
  const bytes = encodePix({ w: 2, h: 2, palette, pixels: new Uint8Array([0, 1, 2, 3]) });
  const decoded = decodePix(bytes), before = decoded.palette.map(entry => ({...entry}));
  assert.deepEqual(paletteGroups(decoded.palette), { grayscale: [1, 0], colors: [2], themed: [3] });
  assert.deepEqual(decoded.palette, before); assert.deepEqual(encodePix(decoded), bytes);
});
test("sound labels are readable while lengths retain exact millisecond bounds", () => {
  const patch = legacyJamPatch("sine");
  assert.equal(instrumentName(patch, [{name: "Soft bell", patch}]), "Soft bell");
  assert.equal(instrumentName(patch, []), "Sine sound"); assert.equal(instrumentName(undefined, []), "Unavailable sound");
  assert.equal(noteName(60), "C4"); assert.equal(noteName(0), "C-1"); assert.equal(noteName(127), "G9");
  assert.equal(soundLengthMs(.001), 1); assert.equal(soundLengthMs(95), 95000); assert.equal(soundLengthMs(600), 600000);
  for (const value of [0, -.1, NaN, Infinity, 600.001]) assert.equal(soundLengthMs(value), null);
});
