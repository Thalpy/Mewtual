import assert from "node:assert/strict";
import test from "node:test";
import {
  TEXT_EFFECTS,
  PERFECT_CHERRY_BLOSSOM_PETALS,
  cherryBlossomShouldBurst,
  dismissTextEffectPalette,
  formatTextEffect,
  insertTextEffect,
  parseTextEffect,
  redTruthSoundPlan,
  speakeseSoundPlan,
  stripTextEffects,
  textEffectGradient,
  textEffectHtml,
} from "./message-effects.ts";

test("the catalog includes expressive, terminal, utility and broad pride choices", () => {
  const ids = new Set(TEXT_EFFECTS.map((effect) => effect.id));
  for (const id of ["shake", "wave", "sparkle", "speakese", "red-truth", "perfect-cherry-blossom", "flame", "gloom", "cyber", "crt", "censor"]) {
    assert.equal(ids.has(id), true, id);
  }
  assert.equal(TEXT_EFFECTS.filter((effect) => effect.group === "Pride").length >= 16, true);
  assert.match(textEffectGradient("pride/trans"), /#5bcefa/);
});

test("the Red Truth has a sealed first-letter flourish and a bounded original sting", () => {
  const html = textEffectHtml("red-truth", "<certain>");
  assert.match(html, /data-text-fx="red-truth"/);
  assert.match(html, /fx-red-truth-flourish/);
  assert.equal((html.match(/fx-red-truth-unit/g) ?? []).length, 9);
  assert.doesNotMatch(html, /<certain>/);

  const plan = redTruthSoundPlan(2);
  assert.equal(plan.length, 3);
  assert.equal(plan[0].at, 2);
  assert.ok(plan.every((note) => note.stop > note.at && note.peak > 0 && note.peak < 0.06));
  assert.equal(new Set(plan.map((note) => note.frequency)).size, 3);
  assert.ok(plan.at(-1)!.stop < 2.5, "the opening sting stays brief");
});

test("Perfect Cherry Blossom renders fixed safe petals outside its accessible text", () => {
  const html = textEffectHtml("perfect-cherry-blossom", "petal <signal>");
  assert.match(html, /data-text-fx="perfect-cherry-blossom"/);
  assert.match(html, /aria-label="petal &lt;signal&gt;"/);
  assert.equal((html.match(/class="fx-blossom-petal /g) ?? []).length, PERFECT_CHERRY_BLOSSOM_PETALS);
  assert.doesNotMatch(html, /<signal>/);
});

test("Perfect Cherry Blossom sheds once on entry, not while moving inside", () => {
  assert.equal(cherryBlossomShouldBurst(true, "perfect-cherry-blossom"), true);
  assert.equal(cherryBlossomShouldBurst(false, "perfect-cherry-blossom"), false);
  assert.equal(cherryBlossomShouldBurst(true, "wave"), false);
  assert.equal(cherryBlossomShouldBurst(true, undefined), false);
});

test("effect tokens are bounded, catalogued and preserve literal text", () => {
  assert.deepEqual(parseTextEffect("[fx:shake]hello[/fx] after"), {
    raw: "[fx:shake]hello[/fx]", id: "shake", text: "hello",
  });
  assert.equal(parseTextEffect("[fx:unknown]hello[/fx]"), null);
  assert.equal(parseTextEffect(`[fx:shake]${"x".repeat(321)}[/fx]`), null);
  assert.equal(formatTextEffect("cyber", "online"), "[fx:cyber]online[/fx]");
});

test("rendered effect HTML escapes hostile text and Speakese emits varied deterministic voice units", () => {
  assert.equal(textEffectHtml("shake", "<img>"), '<span class="text-fx text-fx-shake" data-text-fx="shake">&lt;img&gt;</span>');
  const speakese = textEffectHtml("speakese", "mew");
  assert.match(speakese, /aria-label="mew"/);
  assert.match(speakese, /data-fx-tone="/);
  assert.ok(new Set([...speakese.matchAll(/data-fx-tone="(\d+)"/g)].map((match) => match[1])).size > 1);
  assert.equal((speakese.match(/fx-speakese-unit/g) ?? []).length, 3, "each letter gets its own pop unit");
  assert.doesNotMatch(speakese, /<img>/);
});

test("legacy Animalese markup migrates to Speakese without breaking old posts", () => {
  assert.deepEqual(parseTextEffect("[fx:animalese]mew[/fx]"), {
    raw: "[fx:animalese]mew[/fx]", id: "speakese", text: "mew",
  });
  assert.match(textEffectHtml("animalese", "mew"), /data-text-fx="speakese"/);
  assert.equal(formatTextEffect("animalese", "mew"), "[fx:speakese]mew[/fx]");
});

test("Speakese schedules audible, bounded, phoneme-varying blips in letter cadence", () => {
  const plan = speakeseSoundPlan([2, 14, 23], 5);
  assert.equal(plan.length, 3);
  assert.equal(plan[0].at, 5);
  assert.ok(Math.abs(plan[1].at - plan[0].at - 0.072) < 1e-9);
  assert.ok(new Set(plan.map((blip) => Math.round(blip.frequency))).size === 3);
  assert.ok(plan.every((blip) => blip.peak >= 0.03 && blip.stop > blip.at));
  assert.equal(speakeseSoundPlan(Array(100).fill(1), 0).length, 64);
});

test("the selection palette closes when stale textarea selection loses focus", () => {
  assert.equal(dismissTextEffectPalette(false, "outside"), true);
  assert.equal(dismissTextEffectPalette(false, "editor"), false);
  assert.equal(dismissTextEffectPalette(false, "palette"), false);
  assert.equal(dismissTextEffectPalette(true, "outside"), false, "the catalog's overlay owns outside dismissal");
});

test("the picker wrapper preserves selection and plain summaries remove the markers", () => {
  assert.deepEqual(insertTextEffect("say hello now", 4, 9, "wave"), {
    value: "say [fx:wave]hello[/fx] now",
    selectionStart: 13,
    selectionEnd: 18,
  });
  assert.equal(stripTextEffects("A [fx:crt]quiet signal[/fx]."), "A quiet signal.");
});

test("multi-line and long selections become bounded inline tokens without changing the prose", () => {
  const prose = `${"a".repeat(330)}\nsecond line`;
  const wrapped = insertTextEffect(prose, 0, prose.length, "wave");
  assert.equal(stripTextEffects(wrapped.value), prose);
  assert.equal((wrapped.value.match(/\[fx:wave\]/g) ?? []).length, 3);
  assert.equal(wrapped.selectionStart, wrapped.selectionEnd, "complex wrapping leaves a safe caret");
});
