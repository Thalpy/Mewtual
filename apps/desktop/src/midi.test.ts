// Unit tests for the Web MIDI plumbing.
//
// Run with `npm test` (Node's built-in runner + type stripping; no extra dependencies).
//
// Three things here matter more than the rest:
//
//   1. PARSING has to be forgiving in exactly the right places. Real controllers send releases as
//      velocity-0 note-ons, flood the wire with active sensing, and occasionally hand over a
//      truncated packet. Reading any of those literally produces hung notes or invented ones, so
//      each is pinned by a test rather than left to a comment.
//   2. ROUTING owns the sustain pedal, and the pedal must never reach the melody unlock lock. The
//      lock's secret depends on which notes overlapped and how long each group was held, so a
//      pedal that deferred note-offs there would encode a different secret from the same
//      performance and seal a vault with no recovery path. `sustainEnabled: false` is tested as a
//      hard invariant, not a preference.
//   3. DIAGNOSIS is what the user reads when the thing does not work, so the priority order is
//      pinned: the reason shown must be the one that has to be fixed first.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  CC_ALL_NOTES_OFF, CC_ALL_SOUND_OFF, CC_SUSTAIN, MIDI_FIXES, MIDI_MONITOR_LINES,
  MIDI_SETUP_STEPS, deviceRows, describeMidiMessage, isMonitorWorthy, isPortRouted,
  midiChannelLabel, midiInstrumentAction, midiJamPad, midiPortLabel, midiStatus, newMidiRouter,
  parseMidiMessage, pushMonitorLine,
  releaseAllNotes, routeMidi, routedDevices,
  type MidiDeviceRow, type MidiEnvironment, type MidiPortLike,
} from "./midi.ts";

// --- Parsing --------------------------------------------------------------------------------

test("parseMidiMessage decodes note on and note off", () => {
  assert.deepEqual(parseMidiMessage([0x90, 60, 100]), {
    kind: "noteon", channel: 0, note: 60, velocity: 100,
  });
  assert.deepEqual(parseMidiMessage([0x80, 60, 64]), { kind: "noteoff", channel: 0, note: 60 });
});

test("parseMidiMessage treats a velocity-0 note-on as a release", () => {
  // The classic hung-note bug: plenty of keyboards never send 0x80 at all.
  assert.deepEqual(parseMidiMessage([0x90, 72, 0]), { kind: "noteoff", channel: 0, note: 72 });
});

test("parseMidiMessage keeps the channel out of the status nibble", () => {
  assert.deepEqual(parseMidiMessage([0x95, 60, 1]), {
    kind: "noteon", channel: 5, note: 60, velocity: 1,
  });
  assert.deepEqual(parseMidiMessage([0x8f, 60, 0]), { kind: "noteoff", channel: 15, note: 60 });
});

test("parseMidiMessage rejects packets it cannot honestly read", () => {
  assert.equal(parseMidiMessage(null), null);
  assert.equal(parseMidiMessage(undefined), null);
  assert.equal(parseMidiMessage([]), null);
  assert.equal(parseMidiMessage([0x40, 60, 100]), null); // data byte with no status in front
  assert.equal(parseMidiMessage([0x90, 60]), null); // truncated note message
  assert.equal(parseMidiMessage([0xb0, 64]), null); // truncated controller message
});

test("parseMidiMessage masks data bytes to seven bits", () => {
  // A driver that sets the high bit on a data byte must not shift the note an octave-and-a-bit.
  assert.deepEqual(parseMidiMessage([0x90, 0xbc, 0xf1]), {
    kind: "noteon", channel: 0, note: 0x3c, velocity: 0x71,
  });
});

test("parseMidiMessage reads the sustain pedal at the MIDI half-way point", () => {
  assert.deepEqual(parseMidiMessage([0xb0, CC_SUSTAIN, 127]), {
    kind: "sustain", channel: 0, down: true,
  });
  assert.deepEqual(parseMidiMessage([0xb0, CC_SUSTAIN, 64]), {
    kind: "sustain", channel: 0, down: true,
  });
  assert.deepEqual(parseMidiMessage([0xb0, CC_SUSTAIN, 63]), {
    kind: "sustain", channel: 0, down: false,
  });
  assert.deepEqual(parseMidiMessage([0xb0, CC_SUSTAIN, 0]), {
    kind: "sustain", channel: 0, down: false,
  });
});

test("parseMidiMessage folds both stop-everything controllers into one panic", () => {
  assert.deepEqual(parseMidiMessage([0xb2, CC_ALL_SOUND_OFF, 0]), { kind: "panic", channel: 2 });
  assert.deepEqual(parseMidiMessage([0xb2, CC_ALL_NOTES_OFF, 0]), { kind: "panic", channel: 2 });
});

test("parseMidiMessage names traffic it does not act on", () => {
  assert.deepEqual(parseMidiMessage([0xf8]), { kind: "realtime", status: 0xf8 }); // clock
  assert.deepEqual(parseMidiMessage([0xfe]), { kind: "realtime", status: 0xfe }); // active sensing
  assert.deepEqual(parseMidiMessage([0xf0, 0x7e]), { kind: "other", channel: 0, status: 0xf0 });
  assert.deepEqual(parseMidiMessage([0xb0, 1, 90]), { kind: "other", channel: 0, status: 0xb0 });
  assert.deepEqual(parseMidiMessage([0xe0, 0, 64]), { kind: "other", channel: 0, status: 0xe0 });
});

// --- Monitor --------------------------------------------------------------------------------

test("describeMidiMessage spells messages the way the monitor shows them", () => {
  assert.equal(midiChannelLabel(0), "ch1");
  assert.equal(describeMidiMessage({ kind: "noteon", channel: 0, note: 60, velocity: 100 }), "C4 on · vel 100 · ch1");
  assert.equal(describeMidiMessage({ kind: "noteoff", channel: 9, note: 61 }), "C#4 off · ch10");
  assert.equal(describeMidiMessage({ kind: "sustain", channel: 0, down: true }), "sustain down · ch1");
  assert.equal(describeMidiMessage({ kind: "panic", channel: 0 }), "all notes off · ch1");
  assert.equal(describeMidiMessage({ kind: "realtime", status: 0xfe }), "realtime 0xFE");
  assert.equal(describeMidiMessage({ kind: "other", channel: 0, status: 0xe0 }), "status 0xE0");
});

test("the monitor keeps notes and drops the metronome", () => {
  // Active sensing lands roughly three times a second; it would evict everything else.
  assert.equal(isMonitorWorthy({ kind: "realtime", status: 0xf8 }), false);
  assert.equal(isMonitorWorthy({ kind: "noteon", channel: 0, note: 60, velocity: 1 }), true);
});

test("pushMonitorLine is newest-first and bounded", () => {
  let lines = [] as ReturnType<typeof pushMonitorLine>;
  for (let i = 0; i < MIDI_MONITOR_LINES + 4; i++) {
    lines = pushMonitorLine(lines, { seq: i, port: "K", text: `n${i}`, routed: true });
  }
  assert.equal(lines.length, MIDI_MONITOR_LINES);
  assert.equal(lines[0].seq, MIDI_MONITOR_LINES + 3);
  assert.equal(lines.at(-1)?.seq, 4);
});

test("a filtered-out port still earns a monitor line", () => {
  // Deliberate: watching the port you did NOT pin light up is how you find the one carrying keys.
  const lines = pushMonitorLine([], { seq: 1, port: "MIDIIN2 (Keys)", text: "C4 on", routed: false });
  assert.equal(lines[0].routed, false);
});

// --- Routing --------------------------------------------------------------------------------

test("General MIDI drum notes map exactly onto the fixed jam kit", () => {
  assert.deepEqual(
    [36, 38, 37, 39, 42, 46, 45, 50, 51, 49].map((note) => midiJamPad(note)),
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
  );
  assert.equal(midiJamPad(35), null);
  assert.equal(midiJamPad(52), null);
  assert.equal(midiJamPad(36.5), null);
});

test("instrument MIDI dispatch follows Keys or one-shot Pads mode", () => {
  assert.deepEqual(midiInstrumentAction(60, true, "keys"), { kind: "note", note: 60, on: true });
  assert.deepEqual(midiInstrumentAction(60, false, "keys"), { kind: "note", note: 60, on: false });
  assert.deepEqual(midiInstrumentAction(42, true, "pads"), { kind: "pad", pad: 4 });
  assert.equal(midiInstrumentAction(42, false, "pads"), null, "a one-shot has no sound to release");
  assert.equal(midiInstrumentAction(60, true, "pads"), null, "unmapped MIDI keys stay silent in Pads mode");
});

const on = (note: number, velocity = 90) => ({ kind: "noteon" as const, channel: 0, note, velocity });
const off = (note: number) => ({ kind: "noteoff" as const, channel: 0, note });
const pedal = (down: boolean) => ({ kind: "sustain" as const, channel: 0, down });

test("routeMidi plays and lifts a plain note", () => {
  const state = newMidiRouter();
  assert.deepEqual(routeMidi(state, on(60), true), [{ note: 60, on: true }]);
  assert.deepEqual(routeMidi(state, off(60), true), [{ note: 60, on: false }]);
  assert.equal(state.held.size, 0);
});

test("routeMidi swallows duplicate note-ons and orphan note-offs", () => {
  // Two ports of one controller, key repeat, and a driver echoing its own output all do this.
  const state = newMidiRouter();
  assert.deepEqual(routeMidi(state, on(60), true), [{ note: 60, on: true }]);
  assert.deepEqual(routeMidi(state, on(60), true), []);
  assert.deepEqual(routeMidi(state, off(60), true), [{ note: 60, on: false }]);
  assert.deepEqual(routeMidi(state, off(60), true), []);
});

test("routeMidi lets the pedal hold notes and lifts them all on release", () => {
  const state = newMidiRouter();
  routeMidi(state, on(64), true);
  routeMidi(state, on(60), true);
  assert.deepEqual(routeMidi(state, pedal(true), true), []);
  assert.deepEqual(routeMidi(state, off(64), true), []); // deferred: pedal is down
  assert.deepEqual(routeMidi(state, off(60), true), []);
  assert.deepEqual(routeMidi(state, pedal(false), true), [
    { note: 60, on: false },
    { note: 64, on: false },
  ]);
});

test("routeMidi ignores a pedal position that has not changed", () => {
  const state = newMidiRouter();
  routeMidi(state, on(60), true);
  routeMidi(state, pedal(true), true);
  routeMidi(state, off(60), true);
  assert.deepEqual(routeMidi(state, pedal(true), true), []); // resent, not a new edge
  assert.equal(state.sustained.size, 1);
});

test("routeMidi re-articulates a note struck again under the pedal", () => {
  const state = newMidiRouter();
  routeMidi(state, on(60), true);
  routeMidi(state, pedal(true), true);
  routeMidi(state, off(60), true);
  assert.deepEqual(routeMidi(state, on(60), true), [
    { note: 60, on: false },
    { note: 60, on: true },
  ]);
  assert.equal(state.sustained.size, 0);
  assert.equal(state.held.has(60), true);
});

test("the pedal never defers a note-off when sustain is disabled", () => {
  // The melody unlock lock passes sustainEnabled: false. Deferring there would change which
  // notes count as overlapping and how long the group was held, and therefore change the secret
  // derived from an identical performance. There is no recovery from a vault sealed that way.
  const state = newMidiRouter();
  routeMidi(state, pedal(true), false);
  assert.deepEqual(routeMidi(state, on(60), false), [{ note: 60, on: true }]);
  assert.deepEqual(routeMidi(state, off(60), false), [{ note: 60, on: false }]);
  assert.equal(state.sustained.size, 0);
});

test("lifting the pedal drains notes even after sustain was disabled", () => {
  // Leaving a call mid-pedal switches the target surface; the notes it was holding still have to
  // come back up rather than hanging until the next panic.
  const state = newMidiRouter();
  routeMidi(state, on(60), true);
  routeMidi(state, pedal(true), true);
  routeMidi(state, off(60), true);
  assert.deepEqual(routeMidi(state, pedal(false), false), [{ note: 60, on: false }]);
});

test("a panic message lifts everything and forgets the pedal", () => {
  const state = newMidiRouter();
  routeMidi(state, on(60), true);
  routeMidi(state, pedal(true), true);
  routeMidi(state, off(60), true);
  routeMidi(state, on(67), true);
  assert.deepEqual(routeMidi(state, { kind: "panic", channel: 0 }, true), [
    { note: 60, on: false },
    { note: 67, on: false },
  ]);
  assert.equal(state.sustain, false);
});

test("releaseAllNotes is ascending, deduplicated and idempotent", () => {
  const state = newMidiRouter();
  routeMidi(state, on(67), true);
  routeMidi(state, on(60), true);
  assert.deepEqual(releaseAllNotes(state), [
    { note: 60, on: false },
    { note: 67, on: false },
  ]);
  assert.deepEqual(releaseAllNotes(state), []);
});

test("routeMidi does nothing with traffic it does not act on", () => {
  const state = newMidiRouter();
  assert.deepEqual(routeMidi(state, { kind: "realtime", status: 0xf8 }, true), []);
  assert.deepEqual(routeMidi(state, { kind: "other", channel: 0, status: 0xe0 }, true), []);
});

// --- Devices --------------------------------------------------------------------------------

const port = (p: Partial<MidiPortLike> & { id: string }): MidiPortLike => ({
  state: "connected",
  connection: "open",
  ...p,
});

test("midiPortLabel falls back rather than showing an empty row", () => {
  assert.equal(midiPortLabel({ id: "a", name: "  Keystation 49  " }), "Keystation 49");
  assert.equal(midiPortLabel({ id: "a", name: "   " }), "Unnamed MIDI input");
  assert.equal(midiPortLabel({ id: "a" }), "Unnamed MIDI input");
});

test("isPortRouted matches by id, then by name, and lets everything through by default", () => {
  const p = port({ id: "abc123", name: "Keystation 49" });
  assert.equal(isPortRouted(p, ""), true);
  assert.equal(isPortRouted(p, "abc123"), true);
  // A reinstalled driver renumbers the id; the saved name is what keeps this working.
  assert.equal(isPortRouted(p, "Keystation 49"), true);
  assert.equal(isPortRouted(p, "other-id"), false);
});

test("deviceRows sorts connected first and reports open ports as listening", () => {
  const rows = deviceRows([
    port({ id: "z", name: "Zeta", state: "disconnected", connection: "pending" }),
    port({ id: "b", name: "Alpha", connection: "closed" }),
    port({ id: "a", name: "Alpha" }),
  ]);
  assert.deepEqual(rows.map((r) => r.id), ["a", "b", "z"]); // label ties broken by id, stably
  assert.deepEqual(rows.map((r) => r.connected), [true, true, false]);
  assert.deepEqual(rows.map((r) => r.listening), [true, false, false]);
});

test("deviceRows marks which ports the input filter allows", () => {
  const rows = deviceRows(
    [port({ id: "a", name: "Keys", manufacturer: "  Maker  " }), port({ id: "b", name: "Control" })],
    "Keys",
  );
  const keys = rows.find((r) => r.id === "a");
  assert.equal(keys?.routed, true);
  assert.equal(keys?.maker, "Maker");
  assert.equal(rows.find((r) => r.id === "b")?.routed, false);
  assert.deepEqual(routedDevices(rows).map((r) => r.id), ["a"]);
});

test("routedDevices ignores a routed port that is unplugged", () => {
  const rows = deviceRows([port({ id: "a", name: "Keys", state: "disconnected" })], "");
  assert.deepEqual(routedDevices(rows), []);
});

// --- Diagnosis ------------------------------------------------------------------------------

const live: MidiDeviceRow[] = [
  { id: "a", label: "Keystation 49", maker: "M-Audio", connected: true, listening: true, routed: true },
];
const env = (over: Partial<MidiEnvironment> = {}): MidiEnvironment => ({
  supported: true,
  requested: true,
  busy: false,
  failure: "",
  permission: "granted",
  devices: live,
  ...over,
});

test("midiStatus reports a working controller", () => {
  const status = midiStatus(env());
  assert.equal(status.code, "ready");
  assert.equal(status.level, "ok");
  assert.equal(status.title, "Listening");
  assert.match(status.detail, /Keystation 49/);
});

test("midiStatus counts multiple routed inputs", () => {
  const status = midiStatus(env({ devices: [...live, { ...live[0], id: "b", label: "Pad" }] }));
  assert.equal(status.title, "Listening to 2 inputs");
});

test("midiStatus reports the most fundamental problem first", () => {
  // Each of these also satisfies the conditions below it; the earlier reason is the one to show.
  assert.equal(midiStatus(env({ supported: false, permission: "denied", devices: [] })).code, "unsupported");
  assert.equal(midiStatus(env({ busy: true, devices: [] })).code, "scanning");
  assert.equal(midiStatus(env({ requested: false, permission: "denied" })).code, "off");
  assert.equal(midiStatus(env({ permission: "denied", failure: "boom" })).code, "denied");
  assert.equal(midiStatus(env({ failure: "boom", devices: [] })).code, "error");
  assert.equal(midiStatus(env({ devices: [] })).code, "no-devices");
});

test("midiStatus surfaces the real failure text", () => {
  assert.equal(midiStatus(env({ failure: "NotAllowedError" })).detail, "NotAllowedError");
});

test("midiStatus calls out hardware the routing filter is throwing away", () => {
  const status = midiStatus(env({ devices: [{ ...live[0], routed: false }] }));
  assert.equal(status.code, "not-routed");
  assert.equal(status.level, "warn");
});

test("an unplugged device is not a working one", () => {
  const status = midiStatus(env({ devices: [{ ...live[0], connected: false, listening: false }] }));
  assert.equal(status.code, "no-devices");
});

test("the help text stays populated and non-empty", () => {
  // The panel renders these directly; an empty entry would render as a blank bullet.
  for (const step of [...MIDI_SETUP_STEPS, ...MIDI_FIXES]) {
    assert.ok(step.title.length > 0, "help step needs a title");
    assert.ok(step.detail.length > 20, `help step "${step.title}" needs real guidance`);
  }
  assert.ok(MIDI_SETUP_STEPS.length >= 4);
  assert.ok(MIDI_FIXES.length >= 4);
});
