import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_MESSAGE_FRAME,
  defaultMessageFrameLayer,
  encodeMessageFrame,
  messageFrameArrivalStyle,
  messageFrameEffect,
  messageFrameLayerStyle,
  messageFrameMotion,
  messageFramePosition,
  messageFrameScanGeometry,
  messageFrameShape,
  messageFrameStyle,
  parseMessageFrame,
  visibleMessageFrameMotion,
  visibleMessageFrameStyle,
} from "./message-frame.ts";

const frame = (overrides: Partial<typeof DEFAULT_MESSAGE_FRAME> = {}) => ({
  ...DEFAULT_MESSAGE_FRAME,
  effects: [],
  arrival: { ...DEFAULT_MESSAGE_FRAME.arrival },
  ...overrides,
});

test("legacy frame colours stay compatible and gain the softer defaults", () => {
  assert.equal(messageFrameStyle(" #3a3f4b "), "--message-surface:#3a3f4b;--message-opacity:0.56;--message-edge:68%");
  assert.equal(
    messageFrameStyle("linear-gradient(135deg,#1a2980,#26415e)"),
    "--message-surface:linear-gradient(135deg,#1a2980,#26415e);--message-opacity:0.56;--message-edge:68%",
  );
});

test("frame studio values round-trip with bounded catalogued presentation options", () => {
  const saved = encodeMessageFrame(frame({
    surface: "linear-gradient(135deg,#41295a,#5d2a6e)",
    opacity: 57,
    edge: 82,
    motion: "fly",
    shape: "holo",
    effects: [
      { id: "scan", enabled: true, options: { speed: 7, intensity: 82, amount: 4, direction: -1 } },
      { id: "trace", enabled: false, options: { speed: 4, intensity: 70, amount: 45, direction: 1 } },
    ],
    arrival: { duration: 720, distance: 44, fade: 20, direction: -1, easing: "spring" },
  }));
  assert.equal(saved, "mf3|linear-gradient(135deg,#41295a,#5d2a6e)|57|82|fly|holo|720.44.20.-1.spring|scan.1.7.82.4.-1;trace.0.4.70.45.1");
  assert.deepEqual(parseMessageFrame(saved), {
    surface: "linear-gradient(135deg,#41295a,#5d2a6e)",
    opacity: 57,
    edge: 82,
    motion: "fly",
    shape: "holo",
    effects: [
      { id: "scan", enabled: true, options: { speed: 7, intensity: 82, amount: 4, direction: -1 } },
      { id: "trace", enabled: false, options: { speed: 4, intensity: 70, amount: 45, direction: 1 } },
    ],
    arrival: { duration: 720, distance: 44, fade: 20, direction: -1, easing: "spring" },
  });
  assert.equal(messageFrameMotion(saved), "fly");
  assert.equal(messageFrameShape(saved), "holo");
  assert.equal(messageFrameEffect(saved), "scan");

  assert.deepEqual(parseMessageFrame("mf1|#123456|999|-20|pop"), {
    surface: "#123456",
    opacity: 90,
    edge: 0,
    motion: "pop",
    shape: "terminal",
    effects: [],
    arrival: { ...DEFAULT_MESSAGE_FRAME.arrival },
  });
});

test("mf2 single effects migrate into the layer stack", () => {
  const old = parseMessageFrame("mf2|#123456|60|70|glide|packet|trace");
  assert.deepEqual(old.effects, [defaultMessageFrameLayer("trace")]);
  assert.equal(old.arrival.duration, 480);
  assert.equal(messageFrameEffect("mf2|#123456|60|70|glide|packet|trace"), "trace");
});

test("motion can be saved without forcing a coloured frame", () => {
  const saved = encodeMessageFrame(frame({ surface: "", motion: "drift" }));
  assert.equal(saved, "mf3||56|68|drift|terminal|480.30.12.1.soft|");
  assert.equal(messageFrameStyle(saved), "");
  assert.equal(messageFrameMotion(saved), "drift");
});

test("empty and malformed frames are inert", () => {
  assert.equal(messageFrameStyle(""), "");
  assert.equal(messageFrameStyle("   "), "");
  assert.equal(messageFrameStyle(null), "");
  assert.equal(messageFrameStyle("#123456;background:red"), "");
  assert.equal(messageFrameStyle("url(https://example.test/track)"), "");
  assert.equal(messageFrameStyle("image-set(url(x) 1x)"), "");
  assert.equal(messageFrameStyle("var(--panel)"), "");
  assert.equal(messageFrameStyle("mf1|#123456|60|70|spin"), "");
  assert.equal(messageFrameStyle("mf1|#123456|60|70|fly|extra"), "");
  assert.equal(messageFrameStyle("mf2|#123456|60|70|fly|unknown|scan"), "");
  assert.equal(messageFrameStyle("mf2|#123456|60|70|fly|holo|unknown"), "");
  assert.equal(messageFrameStyle("mf3|#123456|60|70|fly|holo|broken|scan.1.5.60.2.1"), "--message-surface:#123456;--message-opacity:0.6;--message-edge:70%");
});

test("layer and arrival options are bounded before becoming CSS variables", () => {
  const hostile = "mf3|#123456|60|70|fly|holo|9999.999.-5.-1.unknown|scan.1.99.999.99.-1;scan.1.2.20.1.1;trace.1.0.0.0.1;unknown.1.5.50.2.1";
  const parsed = parseMessageFrame(hostile);
  assert.deepEqual(parsed.arrival, { duration: 1200, distance: 80, fade: 0, direction: -1, easing: "soft" });
  assert.deepEqual(parsed.effects, [
    { id: "scan", enabled: true, options: { speed: 10, intensity: 100, amount: 8, direction: -1 } },
    { id: "trace", enabled: true, options: { speed: 1, intensity: 20, amount: 10, direction: 1 } },
  ]);
  assert.match(messageFrameLayerStyle(parsed.effects[0]), /--frame-fx-amount:8/);
  assert.match(messageFrameLayerStyle(parsed.effects[0]), /--frame-fx-size:8px/);
  assert.match(messageFrameLayerStyle(parsed.effects[0]), /--frame-fx-half:4px/);
  assert.match(messageFrameArrivalStyle(hostile), /--message-arrival-duration:1200ms/);
});

test("scan layers share the visible message pane coordinate system", () => {
  assert.deepEqual(messageFrameScanGeometry(100, 640, 132), { offset: 32, height: 640 });
  assert.deepEqual(messageFrameScanGeometry(100, 640, 76), { offset: -24, height: 640 });
});

test("disabling peer frames still leaves the operator's own frame visible", () => {
  const surface = "linear-gradient(135deg,#41295a,#5d2a6e)";
  assert.equal(visibleMessageFrameStyle(surface, true), "");
  assert.equal(
    visibleMessageFrameStyle(surface, true, true),
    "--message-surface:linear-gradient(135deg,#41295a,#5d2a6e);--message-opacity:0.56;--message-edge:68%",
  );
  assert.equal(
    visibleMessageFrameStyle(surface, false),
    "--message-surface:linear-gradient(135deg,#41295a,#5d2a6e);--message-opacity:0.56;--message-edge:68%",
  );
});

test("arrival motion comes from the author of that message only", () => {
  const flyAuthor = encodeMessageFrame(frame({
    surface: "#123456", motion: "fly", shape: "packet", effects: [defaultMessageFrameLayer("trace")],
  }));
  const stillAuthor = encodeMessageFrame(frame({ surface: "#654321", shape: "bracket" }));

  assert.equal(visibleMessageFrameMotion(flyAuthor, true, false), "fly");
  assert.equal(visibleMessageFrameMotion(stillAuthor, true, false), "none");
  assert.equal(visibleMessageFrameMotion(flyAuthor, false, false), "none");
  assert.equal(visibleMessageFrameMotion(flyAuthor, true, true), "none");
});

test("same-author runs form one start, middle and end frame", () => {
  const messages = [
    { author: "cat", ts: 1_000 },
    { author: "cat", ts: 2_000 },
    { author: "cat", ts: 3_000 },
  ];
  assert.deepEqual(messages.map((_, i) => messageFramePosition(messages, i)), ["start", "middle", "end"]);
});

test("dividers, replies, authors and the five-minute boundary split frames", () => {
  const messages = [
    { author: "cat", ts: 1_000 },
    { author: "cat", ts: 2_000 },
    { author: "cat", ts: 3_000, reply_to: "one" },
    { author: "cat", ts: 4_000 },
    { author: "dog", ts: 5_000 },
    { author: "dog", ts: 305_000 },
  ];
  const breaks = new Set([1]);
  assert.deepEqual(messages.map((_, i) => messageFramePosition(messages, i, breaks)), ["single", "single", "start", "end", "single", "single"]);
  assert.equal(messageFramePosition(messages, -1), "single");
  assert.equal(messageFramePosition(messages, messages.length), "single");
});
