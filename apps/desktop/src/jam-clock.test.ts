import assert from "node:assert/strict";
import test from "node:test";
import { JAM_CLOCK_PROBE_BURST, JAM_CLOCK_SAMPLE_MAX, JAM_MET_LOOKAHEAD_MS, JAM_SESSION_NONCE_HEX_CHARS, type JamMetronome } from "./jam-contract.ts";
import { estimateJamClock, JamClockProbeTracker, JamClockSync, JamMetronomeClock } from "./jam-clock.ts";
import { JamSourceChannelRegistry } from "./jam-channel.ts";

const sn = "a".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const otherSn = "b".repeat(JAM_SESSION_NONCE_HEX_CHARS);

function met(overrides: Partial<JamMetronome> = {}): JamMetronome {
  return { t: "m", v: 1, sn, on: 1, rev: 0, bpm: 120, bpb: 4, org: 0, ...overrides };
}

test("NTP math accepts arbitrary process-clock origins and rejects implausible RTT", () => {
  // Remote process has been up an hour longer. Absolute offset size is not clock unsafety.
  assert.deepEqual(estimateJamClock({
    localSentMs: 1_000,
    remoteReceivedMs: 3_601_010,
    remoteSentMs: 3_601_012,
    localReceivedMs: 1_022,
  }), { offsetMs: 3_600_000, rttMs: 20 });
  assert.equal(estimateJamClock({
    localSentMs: 0,
    remoteReceivedMs: 1,
    remoteSentMs: 2,
    localReceivedMs: 3_000,
  }), null);
});

test("the lowest-RTT sample wins and a later two-second offset jump fails sync", () => {
  const sync = new JamClockSync();
  sync.add({ localSentMs: 0, remoteReceivedMs: 1_020, remoteSentMs: 1_030, localReceivedMs: 50 });
  sync.add({ localSentMs: 100, remoteReceivedMs: 1_105, remoteSentMs: 1_106, localReceivedMs: 111 });
  assert.equal(sync.isSynced(), true);
  assert.equal(sync.offsetMs(), 1_000);
  assert.equal(sync.remoteToLocal(2_000), 1_000);
  assert.equal(sync.add({ localSentMs: 200, remoteReceivedMs: 4_205, remoteSentMs: 4_206, localReceivedMs: 211 }), null);
  assert.equal(sync.isSynced(), false);
});

test("old lowest-RTT samples age out of the bounded clock window", () => {
  const sync = new JamClockSync();
  sync.add({ localSentMs: 0, remoteReceivedMs: 1_000, remoteSentMs: 1_000, localReceivedMs: 0 });
  assert.equal(sync.offsetMs(), 1_000);
  for (let index = 0; index < JAM_CLOCK_SAMPLE_MAX; index += 1) {
    const local = 100 + index * 20;
    sync.add({ localSentMs: local, remoteReceivedMs: local + 1_015, remoteSentMs: local + 1_015, localReceivedMs: local + 10 });
  }
  assert.equal(sync.offsetMs(), 1_010, "the historic zero-RTT sample must leave the current window");
});

test("clock replies must match one bounded outstanding probe exactly", () => {
  const tracker = new JamClockProbeTracker();
  const probes = Array.from({ length: JAM_CLOCK_PROBE_BURST }, () => tracker.issue(100));
  assert.ok(probes.every(Boolean));
  assert.equal(tracker.issue(100), null);
  const probe = probes[0]!;
  assert.equal(tracker.accept({ t: "c", r: probe.q, tx: probe.tx + 1, rx: 1_105 }, 110), null);
  const sample = tracker.accept({ t: "c", r: probe.q, tx: probe.tx, rx: 1_105 }, 110);
  assert.deepEqual(sample, {
    localSentMs: 100,
    localReceivedMs: 110,
    remoteReceivedMs: 1_105,
    remoteSentMs: 1_105,
  });
  assert.equal(tracker.accept({ t: "c", r: probe.q, tx: probe.tx, rx: 1_105 }, 110), null, "a reply is one-shot");
});

test("a foreign max revision cannot seize or wedge the current anchor", () => {
  const channels = new JamSourceChannelRegistry();
  const clock = new JamMetronomeClock(channels);
  const alice = channels.open("alice");
  const mallory = channels.open("mallory");
  assert.equal(clock.receive(alice, met(), 0), "started");
  assert.equal(clock.receive(mallory, met({ sn: otherSn, rev: 0xffff_ffff, bpm: 240 }), 3_000), "foreign");
  assert.equal(clock.snapshot()?.source, "alice");
  assert.equal(clock.anchorLeft("alice"), true);
  assert.equal(clock.receive(channels.open("bob"), met({ sn: otherSn, rev: 0 }), 4_000), "started");
  assert.equal(clock.snapshot()?.source, "bob");
});

test("only the anchor can update or stop, and revision churn is rate-limited", () => {
  const channels = new JamSourceChannelRegistry();
  const clock = new JamMetronomeClock(channels);
  const alice = channels.open("alice");
  const bob = channels.open("bob");
  assert.equal(clock.receive(alice, met(), 0), "started");
  assert.equal(clock.receive(alice, met({ rev: 1, bpm: 121 }), 1_000), "too-fast");
  assert.equal(clock.receive(alice, met({ rev: 1, bpm: 121 }), 2_000), "updated");
  assert.equal(clock.receive(alice, met({ rev: 1, on: 0 }), 5_000), "stale");
  assert.equal(clock.receive(bob, met({ sn: otherSn, on: 0, rev: 99 }), 5_000), "foreign");
  assert.equal(clock.receive(alice, met({ rev: 2, on: 0 }), 5_000), "stopped");
});

test("lookahead schedules on the audio clock, stays under 150 ms and never duplicates a beat", () => {
  const channels = new JamSourceChannelRegistry();
  const clock = new JamMetronomeClock(channels);
  const sync = new JamClockSync();
  // Remote clock = local clock + 1000 ms.
  sync.add({ localSentMs: 0, remoteReceivedMs: 1_005, remoteSentMs: 1_006, localReceivedMs: 11 });
  assert.equal(clock.receive(channels.open("alice"), met({ org: 10_000 }), 9_000), "started");
  const first = clock.plan(sync, 9_000, 20);
  assert.equal(first.length, 1);
  assert.equal(first[0].audioTime, 20);
  assert.equal(first[0].accent, true);
  assert.ok(first.every((click) => click.audioTime <= 20 + JAM_MET_LOOKAHEAD_MS / 1_000));
  assert.deepEqual(clock.plan(sync, 9_025, 20.025), [], "the same beat must not be scheduled twice");
  const next = clock.plan(sync, 9_500, 20.5);
  assert.equal(next[0]?.beat, 1);
});

test("without a trustworthy offset the click is explicitly local-only", () => {
  const channels = new JamSourceChannelRegistry();
  const clock = new JamMetronomeClock(channels);
  assert.equal(clock.receive(channels.open("alice"), met({ org: 9_999_999 }), 5_000), "started");
  const clicks = clock.plan(new JamClockSync(), 5_000, 10);
  assert.equal(clicks[0]?.audioTime, 10);
});

test("a stale channel capability cannot seize metronome state", () => {
  const channels = new JamSourceChannelRegistry();
  const clock = new JamMetronomeClock(channels);
  const old = channels.open("alice");
  const current = channels.open("alice");
  assert.equal(clock.receive(old, met(), 0), "stale-channel");
  assert.equal(clock.receive(current, met(), 0), "started");
});
