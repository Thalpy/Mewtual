import { test } from "node:test";
import assert from "node:assert/strict";

import { NEWS_CHIME_NOTES, scheduleNewsChime } from "./news-chime.ts";

test("the news cue is three matching, evenly spaced reference-shaped pips", () => {
  assert.equal(NEWS_CHIME_NOTES.length, 3);
  assert.deepEqual(NEWS_CHIME_NOTES.map((note) => note.frequency), [932.33, 932.33, 932.33]);
  assert.deepEqual(NEWS_CHIME_NOTES.map((note) => note.offset), [0, 0.1, 0.2]);
  assert.ok(Math.max(...NEWS_CHIME_NOTES.map((note) => note.offset + note.duration)) < 0.3);
});

test("scheduleNewsChime creates one short, shaped voice per note", () => {
  const starts: number[] = [];
  const stops: number[] = [];
  const oscillatorTypes: OscillatorType[] = [];
  const frequencyStarts: [number, number][] = [];
  const frequencyEnds: [number, number][] = [];
  const gainEvents: ["set" | "ramp", number, number][] = [];

  const ctx = {
    currentTime: 12,
    destination: {},
    createOscillator() {
      let oscillatorType: OscillatorType = "sine";
      return {
        get type() { return oscillatorType; },
        set type(value: OscillatorType) { oscillatorType = value; oscillatorTypes.push(value); },
        frequency: {
          setValueAtTime(value: number, time: number) { frequencyStarts.push([value, time]); },
          exponentialRampToValueAtTime(value: number, time: number) { frequencyEnds.push([value, time]); },
        },
        connect(node: unknown) { return node; },
        start(time: number) { starts.push(time); },
        stop(time: number) { stops.push(time); },
      };
    },
    createGain() {
      return {
        gain: {
          setValueAtTime(value: number, time: number) { gainEvents.push(["set", value, time]); },
          exponentialRampToValueAtTime(value: number, time: number) { gainEvents.push(["ramp", value, time]); },
        },
        connect(node: unknown) { return node; },
      };
    },
  } as unknown as AudioContext;

  scheduleNewsChime(ctx);

  assert.deepEqual(oscillatorTypes, ["square", "square", "square"]);
  assert.deepEqual(starts, NEWS_CHIME_NOTES.map((note) => 12 + note.offset));
  assert.deepEqual(stops, NEWS_CHIME_NOTES.map((note) => 12 + note.offset + note.duration + 0.01));
  assert.deepEqual(frequencyEnds, NEWS_CHIME_NOTES.map((note) => [note.frequency, 12 + note.offset + 0.038]));
  assert.equal(frequencyStarts.length, 3);
  assert.equal(gainEvents.length, 9);
});
