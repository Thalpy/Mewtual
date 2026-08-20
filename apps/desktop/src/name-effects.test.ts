import { test } from "node:test";
import assert from "node:assert/strict";

import {
  decodeNameEffects, defaultNameEffect, effectConfigured, effectEnabled, encodeNameEffects,
  nameEffectClasses, nameEffectStyle,
} from "./name-effects.ts";
import type { NameEffectId } from "./name-effects.ts";

test("legacy effects still decode", () => {
  assert.equal(decodeNameEffects("wave")[0]?.id, "wave");
  const gradient = decodeNameEffects("grad2-e879c0-977df2-45-a7r")[0];
  assert.equal(gradient?.id, "gradient");
  assert.deepEqual(gradient?.options.stops, ["#e879c0", "#977df2"]);
  assert.equal(gradient?.options.direction, -1);
});

test("effect stacks round-trip and retain independent options", () => {
  const wave = defaultNameEffect("mexican");
  wave.options.height = 9;
  const shadow = defaultNameEffect("shadow");
  shadow.options.color = "#123456";
  const decoded = decodeNameEffects(encodeNameEffects([wave, shadow]));
  assert.deepEqual(decoded.map((effect) => effect.id), ["mexican", "shadow"]);
  assert.equal(decoded[0].options.height, 9);
  assert.equal(decoded[1].options.color, "#123456");
});

test("disabled effects retain settings but do not render", () => {
  const shadow = defaultNameEffect("shadow");
  shadow.enabled = false;
  shadow.options.blur = 13;
  const decoded = decodeNameEffects(encodeNameEffects([shadow]));
  assert.equal(effectConfigured(decoded, "shadow"), true);
  assert.equal(effectEnabled(decoded, "shadow"), false);
  assert.equal(decoded[0].options.blur, 13);
  assert.equal(nameEffectClasses(decoded), "");
  assert.equal(nameEffectStyle(decoded), "");
});

test("untrusted stack values are clamped, deduplicated, and unknown ids ignored", () => {
  const raw = "fxs1:" + encodeURIComponent(JSON.stringify([
    { id: "shadow", options: { x: 999, opacity: -4, color: "red" } },
    { id: "shadow", options: { x: 1 } },
    { id: "made-up", options: {} },
  ]));
  const decoded = decodeNameEffects(raw);
  assert.equal(decoded.length, 1);
  assert.equal(decoded[0].options.x, 8);
  assert.equal(decoded[0].options.opacity, 10);
  assert.equal(decoded[0].options.color, "#000000");
});

test("composed styling keeps animations and shadows in shared declarations", () => {
  const effects = [
    defaultNameEffect("rainbow"), defaultNameEffect("wave"),
    defaultNameEffect("neon"), defaultNameEffect("shadow"),
  ];
  assert.match(nameEffectClasses(effects), /fx-rainbow/);
  const style = nameEffectStyle(effects);
  assert.match(style, /animation:fx-rainbow[^;]+,fx-stack-wave/);
  assert.match(style, /text-shadow:[^;]+rgba\(0,0,0,0\.7\)/);
});

test("every studio effect and global control survives one complete recipe", () => {
  const ids: NameEffectId[] = [
    "gradient", "rainbow", "neon", "wave", "mexican", "pulse", "outline", "shadow", "retro", "glitch",
    "shimmer", "sparkle", "wobble", "candy", "ghost", "fire", "extrude", "typography", "master",
  ];
  const encoded = encodeNameEffects(ids.map(defaultNameEffect));
  assert.ok(encoded.length < 4096);
  assert.deepEqual(decodeNameEffects(encoded).map((effect) => effect.id), ids);
});

test("new effect options are validated and produce composable CSS", () => {
  const raw = "fxs1:" + encodeURIComponent(JSON.stringify([
    { id: "sparkle", options: { speed: 99, intensity: -1 } },
    { id: "wobble", options: { speed: 5, amount: 99 } },
    { id: "fire", options: { height: 6, intensity: 80, speed: 7 } },
    { id: "extrude", options: { depth: 4, direction: -1, color: "#123456", opacity: 75 } },
    { id: "typography", options: { weight: 2000, italic: true, uppercase: true, tracking: 99, bubble: 2 } },
    { id: "master", options: { intensity: 125, speed: 150 } },
  ]));
  const effects = decodeNameEffects(raw);
  assert.equal(effects[0].options.speed, 10);
  assert.equal(effects[0].options.intensity, 20);
  assert.equal(effects[1].options.amount, 8);
  assert.equal(effects[4].options.weight, 900);
  assert.equal(effects[4].options.tracking, 6);

  const classes = nameEffectClasses(effects);
  assert.match(classes, /fx-sparkle/);
  assert.match(classes, /fx-fire/);
  assert.doesNotMatch(classes, /fx-typography|fx-master/);

  const style = nameEffectStyle(effects);
  assert.match(style, /--fx-sparkle-dur:/);
  assert.match(style, /--fx-fire-dur:/);
  assert.match(style, /font-weight:900/);
  assert.match(style, /text-transform:uppercase/);
  assert.match(style, /text-shadow:/);
});
