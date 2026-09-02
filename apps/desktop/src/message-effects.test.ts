import assert from "node:assert/strict";
import test from "node:test";
import {
  CONCEALING_EFFECT_IDS,
  DEFAULT_QUICK_TEXT_EFFECT_IDS,
  MAX_TELETYPE_CLICKS,
  ONE_SHOT_EFFECT_IDS,
  QUICK_TEXT_EFFECT_LIMIT,
  rankQuickTextEffects,
  sanitizeTextEffectUsage,
  TELETYPE_STEP_SECONDS,
  TEXT_EFFECTS,
  TEXT_EFFECT_GROUPS,
  PERFECT_CHERRY_BLOSSOM_PETALS,
  cherryBlossomShouldBurst,
  dismissTextEffectPalette,
  formatTextEffect,
  insertTextEffect,
  parseTextEffect,
  redTruthNoiseSample,
  redTruthSoundPlan,
  speakeseSoundPlan,
  stripTextEffects,
  teletypeSoundPlan,
  textEffectGradient,
  textEffectHtml,
} from "./message-effects.ts";

test("the expanded catalog lists every new effect, keeps Utility ahead of Pride, and flags statics", () => {
  const byId = new Map(TEXT_EFFECTS.map((effect) => [effect.id, effect]));
  const motion = ["decrypt", "heartbeat", "jelly"];
  const mood = ["frost", "legendary", "whisper", "void"];
  const signal = ["hologram", "neon", "corrupted", "teletype"];
  const utility = ["blur", "highlight", "key", "tag", "fine-print", "shout", "spaced"];
  for (const [group, ids] of [["Motion", motion], ["Mood", mood], ["Signal", signal], ["Utility", utility]] as const) {
    for (const id of ids) {
      const effect = byId.get(id);
      assert.ok(effect, id);
      assert.equal(effect.group, group, id);
      assert.equal(effect.animated, group !== "Utility", `${id} animated flag`);
      assert.match(textEffectHtml(id, "x"), new RegExp(`data-text-fx="${id}"`));
    }
  }
  assert.ok(TEXT_EFFECT_GROUPS.indexOf("Utility") < TEXT_EFFECT_GROUPS.indexOf("Pride"));
  assert.equal(new Set(TEXT_EFFECTS.map((effect) => effect.id)).size, TEXT_EFFECTS.length, "ids are unique");
  assert.deepEqual([...CONCEALING_EFFECT_IDS], ["censor", "blur"]);
  assert.deepEqual([...ONE_SHOT_EFFECT_IDS], ["speakese", "red-truth", "decrypt", "teletype"]);
});

test("the Aa strip ranks by usage, fills from the seed list, and rejects junk counts", () => {
  assert.deepEqual(rankQuickTextEffects({}), [...DEFAULT_QUICK_TEXT_EFFECT_IDS]);
  const ranked = rankQuickTextEffects({ neon: 5, "pride/trans": 9, wave: 5, decrypt: 1 });
  assert.deepEqual(ranked.slice(0, 4), ["pride/trans", "wave", "neon", "decrypt"], "count desc, then seed order breaks the tie");
  assert.equal(ranked.length, QUICK_TEXT_EFFECT_LIMIT);
  assert.equal(new Set(ranked).size, ranked.length, "no duplicates once seeds fill in");
  assert.deepEqual(rankQuickTextEffects({ shake: 3 }, 3), ["shake", "wave", "sparkle"]);
  assert.deepEqual(
    sanitizeTextEffectUsage({ animalese: 2, wave: 1.9, bogus: 4, shake: -1, crt: Infinity, neon: "7" }),
    { speakese: 2, wave: 1 },
    "legacy ids canonicalise, unknown ids and bad numbers drop",
  );
  assert.deepEqual(sanitizeTextEffectUsage("nope"), {});
});

test("Decrypt and Corrupted carry fixed stand-in glyphs per visible letter and never leak markup", () => {
  const decrypt = textEffectHtml("decrypt", "a <b");
  assert.match(decrypt, /aria-label="a &lt;b"/);
  assert.equal((decrypt.match(/data-fx-glyph="/g) ?? []).length, 3, "three visible letters get glyphs, the space does not");
  assert.equal((decrypt.match(/data-fx-glyph2="/g) ?? []).length, 3);
  assert.doesNotMatch(decrypt, /<b"/);
  assert.equal(decrypt, textEffectHtml("decrypt", "a <b"), "the scramble is deterministic");
  assert.notEqual(
    [...decrypt.matchAll(/data-fx-glyph="([^"]*)"/g)].map((m) => m[1]).join(""),
    [...decrypt.matchAll(/data-fx-glyph2="([^"]*)"/g)].map((m) => m[1]).join(""),
    "the two stand-ins differ so the cycle is visible",
  );
  assert.match(textEffectHtml("corrupted", "ok"), /fx-corrupted-unit fx-i-1" data-fx-glyph="/);
});

test("Teletype types with a cursor and a bounded, alternating click plan", () => {
  const html = textEffectHtml("teletype", "hi");
  assert.equal((html.match(/fx-teletype-unit/g) ?? []).length, 2);
  assert.match(html, /fx-teletype-cursor/);
  const plan = teletypeSoundPlan(3, 1);
  assert.equal(plan.length, 3);
  assert.equal(plan[0].at, 1);
  assert.ok(Math.abs(plan[1].at - plan[0].at - TELETYPE_STEP_SECONDS) < 1e-9);
  assert.notEqual(plan[0].frequency, plan[1].frequency);
  assert.ok(plan.every((click) => click.stop > click.at && click.peak > 0 && click.peak < 0.03));
  assert.equal(teletypeSoundPlan(500, 0).length, MAX_TELETYPE_CLICKS);
  assert.equal(teletypeSoundPlan(-4, 0).length, 0);
});

test("Blur conceals like Censor and Key splits chords into caps", () => {
  const blur = textEffectHtml("blur", "<twist>");
  assert.match(blur, /data-fx-conceal=""/);
  assert.match(blur, /role="button"/);
  assert.doesNotMatch(blur, /<twist>/);
  assert.match(textEffectHtml("censor", "x"), /data-fx-conceal=""/);
  assert.equal(insertTextEffect("", 0, 0, "blur").value, "[fx:blur]spoiler[/fx]");

  const chord = textEffectHtml("key", "Ctrl+Shift+F");
  assert.equal((chord.match(/class="fx-keycap"/g) ?? []).length, 3);
  assert.equal((chord.match(/fx-keycap-join/g) ?? []).length, 2);
  assert.equal((textEffectHtml("key", "+").match(/class="fx-keycap"/g) ?? []).length, 1, "a lone plus is one cap");
  assert.equal((textEffectHtml("key", "Enter").match(/class="fx-keycap"/g) ?? []).length, 1);
  assert.doesNotMatch(textEffectHtml("key", "<img>+x"), /<img>/);
});

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
  assert.equal(plan.strike.length, 3);
  assert.equal(plan.strike[0].at, 2);
  assert.ok(plan.strike.every((note) => note.stop > note.at && note.peak > 0 && note.peak < 0.06));
  assert.equal(new Set(plan.strike.map((note) => note.frequency)).size, 3);
  assert.ok(plan.strike.every((note) => note.stop < plan.sweep.crest), "the ting lands before the wash crests");
  assert.ok(plan.sweep.at > plan.strike[0].at, "the background wash follows the initial ting");
  assert.ok(plan.sweep.crestFrequency > plan.sweep.startFrequency);
  assert.ok(plan.sweep.crestFrequency > plan.sweep.endFrequency);
  assert.ok(plan.sweep.stop - plan.sweep.at > 1.5, "the shaa tail has room to rise and fall");
  assert.equal(redTruthNoiseSample(42), redTruthNoiseSample(42));
  assert.notEqual(redTruthNoiseSample(42), redTruthNoiseSample(43));
  assert.ok(Math.abs(redTruthNoiseSample(42)) <= 1);
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
  assert.ok(plan.every((blip) => blip.peak >= 0.04 && blip.stop > blip.at));
  // Audibility on small speakers: every blip sits in a speech register, and vowels carry the most gain.
  assert.ok(plan.every((blip) => blip.frequency >= 320 && blip.frequency <= 1_100), "speech register");
  assert.ok(plan[0].peak > plan[1].peak, "vowels are louder than consonants");
  assert.notEqual(plan[0].waveform, "sine", "vowels use a harmonic-rich waveform");
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
