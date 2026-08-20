import assert from "node:assert/strict";
import test from "node:test";
import {
  TEXT_EFFECTS,
  formatTextEffect,
  insertTextEffect,
  parseTextEffect,
  stripTextEffects,
  textEffectGradient,
  textEffectHtml,
} from "./message-effects.ts";

test("the catalog includes expressive, terminal, utility and broad pride choices", () => {
  const ids = new Set(TEXT_EFFECTS.map((effect) => effect.id));
  for (const id of ["shake", "wave", "sparkle", "animalese", "flame", "gloom", "cyber", "crt", "censor"]) {
    assert.equal(ids.has(id), true, id);
  }
  assert.equal(TEXT_EFFECTS.filter((effect) => effect.group === "Pride").length >= 16, true);
  assert.match(textEffectGradient("pride/trans"), /#5bcefa/);
});

test("effect tokens are bounded, catalogued and preserve literal text", () => {
  assert.deepEqual(parseTextEffect("[fx:shake]hello[/fx] after"), {
    raw: "[fx:shake]hello[/fx]", id: "shake", text: "hello",
  });
  assert.equal(parseTextEffect("[fx:unknown]hello[/fx]"), null);
  assert.equal(parseTextEffect(`[fx:shake]${"x".repeat(321)}[/fx]`), null);
  assert.equal(formatTextEffect("cyber", "online"), "[fx:cyber]online[/fx]");
});

test("rendered effect HTML escapes hostile text and animalese emits deterministic voice units", () => {
  assert.equal(textEffectHtml("shake", "<img>"), '<span class="text-fx text-fx-shake" data-text-fx="shake">&lt;img&gt;</span>');
  const animalese = textEffectHtml("animalese", "mew");
  assert.match(animalese, /aria-label="mew"/);
  assert.match(animalese, /data-fx-tone="/);
  assert.doesNotMatch(animalese, /<img>/);
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
