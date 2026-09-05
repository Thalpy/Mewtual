import { test } from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_PUSH_TO_TALK,
  bindableKey,
  keyLabel,
  micTransmitting,
  parsePushToTalk,
  pushToTalkEvent,
} from "./push-to-talk.ts";

const ptt = { mode: "ptt" as const, key: "KeyV" };

test("an open microphone transmits until it is muted", () => {
  assert.equal(micTransmitting(DEFAULT_PUSH_TO_TALK, false, false), true);
  assert.equal(micTransmitting(DEFAULT_PUSH_TO_TALK, true, false), false);
  // Holding a key means nothing when push to talk is off.
  assert.equal(micTransmitting(DEFAULT_PUSH_TO_TALK, true, true), false);
});

test("push to talk transmits only while the key is held", () => {
  assert.equal(micTransmitting(ptt, false, true), true);
  assert.equal(micTransmitting(ptt, false, false), false);
});

test("an explicit mute outranks the talk key", () => {
  // Mute is a statement about the whole call; push to talk only decides when an unmuted
  // microphone is live. Pressing the key while muted must not put you on air.
  assert.equal(micTransmitting(ptt, true, true), false);
});

test("push to talk with no key bound never silences the microphone", () => {
  // The failure this prevents: a mode with no key would mute the whole call with no way to open
  // it, which reads as a broken app rather than as a setting.
  const unbound = { mode: "ptt" as const, key: "" };
  assert.equal(micTransmitting(unbound, false, false), true);
  assert.equal(parsePushToTalk(unbound).mode, "open", "and it does not survive being stored");
});

test("a stored setting is only honoured when it is complete and sane", () => {
  assert.deepEqual(parsePushToTalk(null), DEFAULT_PUSH_TO_TALK);
  assert.deepEqual(parsePushToTalk({}), DEFAULT_PUSH_TO_TALK);
  assert.deepEqual(parsePushToTalk({ mode: "ptt", key: "KeyV" }), ptt);
  assert.deepEqual(parsePushToTalk({ mode: "nonsense", key: "KeyV" }), { mode: "open", key: "KeyV" });
  assert.equal(parsePushToTalk({ mode: "ptt", key: "x".repeat(64) }).key, "", "a bounded key");
  assert.equal(parsePushToTalk({ mode: "ptt", key: 7 }).mode, "open");
});

test("the keys that get you out of the app cannot be bound to the microphone", () => {
  assert.ok(bindableKey("KeyV"));
  assert.ok(bindableKey("ControlLeft"));
  assert.ok(bindableKey("Backquote"));
  for (const reserved of ["Escape", "Tab", "Enter", "NumpadEnter", "MetaLeft", "MetaRight"]) {
    assert.equal(bindableKey(reserved), false, `${reserved} must stay itself`);
  }
  assert.equal(bindableKey(""), false);
});

test("a bound key that lands in a text field is a keystroke, not a talk button", () => {
  const inCall = { code: "KeyV", typing: false, inCall: true };
  assert.equal(pushToTalkEvent(ptt, inCall), true);
  // Binding a letter and then writing a message must not open the microphone on every V.
  assert.equal(pushToTalkEvent(ptt, { ...inCall, typing: true }), false);
  assert.equal(pushToTalkEvent(ptt, { ...inCall, inCall: false }), false, "not in a call");
  assert.equal(pushToTalkEvent(ptt, { ...inCall, code: "KeyB" }), false, "a different key");
  assert.equal(pushToTalkEvent(DEFAULT_PUSH_TO_TALK, inCall), false, "push to talk is off");
});

test("a binding is shown as something a person would recognise", () => {
  assert.equal(keyLabel(""), "not set");
  assert.equal(keyLabel("KeyV"), "V");
  assert.equal(keyLabel("Digit4"), "4");
  assert.equal(keyLabel("ControlLeft"), "Control Left");
  assert.equal(keyLabel("Numpad0"), "Numpad 0");
  assert.equal(keyLabel("ArrowUp"), "Arrow Up");
  assert.equal(keyLabel("Backquote"), "Backquote");
});
