import assert from "node:assert/strict";
import test from "node:test";
import { studioSoundModel } from "./studio-sound.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { fsource, id32, id64, reg, ME, ROOK } from "./studio-testkit.ts";
const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: StudioMusic } = await import("./StudioMusic.svelte");

const SCORE = id32(10), OTHER = id32(11), PATCH = id64(1);
const readiness = { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: false };
const tracks = [
  { id: SCORE, title: "Moonwalk melody", createdBy: ME, where: "visible" as const },
  { id: OTHER, title: "Recovery sketch", createdBy: ROOK, where: "visible" as const },
];
function model(records: Record<string, unknown>, ready = true) {
  return studioSoundModel({ records: records as never, readiness: { ...readiness, conflicts: !ready } });
}
const who = (id: string) => (id === ME ? "you" : "rook");
const tint = () => "var(--accent)";
function mountMusic(props: Record<string, unknown>) {
  const target = document.createElement("div"); document.body.append(target);
  const links: (string | null)[] = [], opened: string[] = []; let created = 0;
  const app = mount(StudioMusic, { target, props: { frameCount: 3, fps: 8, tracks, who, tint, onlink: (s: string | null) => links.push(s), onopen: (s: string) => opened.push(s), oncreate: () => { created++; }, ...props } as never });
  flushSync();
  const text = () => target.textContent?.replace(/\s+/g, " ") ?? "";
  const click = (sel: string, label: string) => { const b = [...target.querySelectorAll<HTMLButtonElement>(sel)].find((x) => x.textContent?.trim().includes(label)); assert.ok(b, `button ${label}`); b.click(); flushSync(); };
  return { target, links, opened, created: () => created, text, click, done: async () => { await unmount(app); target.remove(); } };
}

test("nothing linked: the picker lists this channel's scores and links one; export reads ready", async () => {
  const m = mountMusic({ model: model({}) });
  try {
    assert.match(m.text(), /no track/);
    assert.match(m.text(), /Moonwalk melody/);
    assert.match(m.text(), /3 at 8 fps · 0\.38 s/);
    assert.match(m.text(), /ready · frames/);
    m.click("button.row", "Recovery sketch");
    assert.deepEqual(m.links, [OTHER]);
    m.click("button.btn", "new track");
    assert.equal(m.created(), 1);
  } finally { await m.done(); }
});

test("a linked, held track shows its title and author, opens, unlinks, and change reveals the picker", async () => {
  const m = mountMusic({ model: model({ "h/score": reg({ op: "set_header", field: "score", value: SCORE }, fsource(1)) }) });
  try {
    assert.match(m.text(), /Moonwalk melody/);
    assert.match(m.text(), /linked/);
    assert.doesNotMatch(m.text(), /pick a track/);
    m.click("button.btn", "open track");
    assert.deepEqual(m.opened, [SCORE]);
    m.click("button.btn", "change");
    assert.match(m.text(), /pick a track/);
    m.click("button.btn", "unlink");
    assert.deepEqual(m.links, [null]);
  } finally { await m.done(); }
});

test("a link to a track this device does not hold is named as unavailable and blocks export", async () => {
  const m = mountMusic({ model: model({ "h/score": reg({ op: "set_header", field: "score", value: id32(99) }, fsource(1)) }) });
  try {
    assert.match(m.text(), /not on this device/);
    assert.match(m.text(), /unavailable/);
    assert.match(m.text(), /waits · linked track not on this device/);
    m.click("button.btn", "read again");
    assert.deepEqual(m.opened, [id32(99)]);
  } finally { await m.done(); }
});

test("two concurrent links show both values and 'use this' relinks on purpose", async () => {
  const records = { "h/score": reg({ op: "set_header", field: "score", value: SCORE }, fsource(1), [{ value: { op: "set_header", field: "score", value: OTHER }, source: fsource(2, ROOK) }]) };
  const m = mountMusic({ model: model(records) });
  try {
    assert.match(m.text(), /another track chosen by rook/);
    assert.match(m.text(), /waits · two tracks chosen/);
    m.click("button.lnk", "use this");
    assert.deepEqual(m.links, [OTHER]);
    m.click("button.lnk", "keep this");
    assert.deepEqual(m.links, [OTHER, SCORE]);
  } finally { await m.done(); }
});

test("recipes count live slots and name the sfx recipes; a read-only reason disables linking", async () => {
  const records = {
    [`p/${PATCH}`]: reg({ op: "set_patch", patch: PATCH, descriptor: legacyJamPatch("sine") }, fsource(1)),
    [`s/${id32(5)}`]: reg({ op: "set_sfx", sfx: id32(5), frame: id32(1), patch: PATCH, note: 60, duration_ms: 250 }, fsource(2)),
  };
  const m = mountMusic({ model: model(records, false), readonlyReason: "Stop playback before editing" });
  try {
    assert.match(m.text(), /recipes · 1 of 64/);
    assert.match(m.text(), /Sine sound/);
    assert.match(m.text(), /waits · sounds need attention/);
    assert.match(m.text(), /Stop playback before editing/);
    const pick = [...m.target.querySelectorAll<HTMLButtonElement>("button.row")].find((b) => /Moonwalk/.test(b.textContent ?? ""));
    assert.ok(pick?.disabled);
  } finally { await m.done(); }
});
