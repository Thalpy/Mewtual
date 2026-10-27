import assert from "node:assert/strict";
import test from "node:test";
import { VoiceRtcConfigurationOwner, VOICE_RTC_CONFIGURATION_BYTES, VOICE_RTC_CONFIGURATION_UNITS,
  voiceRtcConfigurationInventory, voiceRtcConfigurationCurrent, voiceRtcConfiguredPeer,
  releaseVoiceRtcConfiguration, type VoiceRtcDeviceSettings, type VoiceRtcConfigurationLease } from "./voice-rtc-configuration.ts";

const settings = (change: Partial<VoiceRtcDeviceSettings> = {}): VoiceRtcDeviceSettings => ({
  stun: "stun:stun.example.test:3478", turn: "turns:relay.example.test:5349?transport=tcp",
  username: "member", credential: "private-password", policy: "all", ...change,
});
function constructor(configurations: RTCConfiguration[]): typeof RTCPeerConnection {
  return class Peer { constructor(configuration: RTCConfiguration) { configurations.push(configuration); } } as unknown as typeof RTCPeerConnection;
}

test("separate configuration inventory covers bounded capture,17 peer dictionaries and constructor overlaps", () => {
  assert.ok(voiceRtcConfigurationInventory() <= VOICE_RTC_CONFIGURATION_BYTES);
  assert.ok(voiceRtcConfigurationInventory() > 32 * 1024);
});

test("private snapshot is immutable across caller mutation and distinct SDK constructor dictionaries", () => {
  const owner = new VoiceRtcConfigurationOwner(), input = { ...settings() }, seen: RTCConfiguration[] = [];
  owner.capture(() => input, () => true);
  input.stun = "stun:changed.example.test"; input.credential = "changed-secret";
  const first = owner.borrow(), second = owner.borrow();
  assert.deepEqual(Reflect.ownKeys(first), []);
  voiceRtcConfiguredPeer(first, constructor(seen), () => true);
  seen[0].iceServers![0].urls = "stun:provider-mutated.example.test";
  seen[0].iceServers![1].credential = "provider-mutated-secret";
  voiceRtcConfiguredPeer(second, constructor(seen), () => true);
  assert.deepEqual(seen[1], { bundlePolicy: "max-bundle", rtcpMuxPolicy: "require", iceCandidatePoolSize: 0,
    iceTransportPolicy: "all", iceServers: [{ urls: "stun:stun.example.test:3478" },
      { urls: "turns:relay.example.test:5349?transport=tcp", username: "member", credential: "private-password" }] });
  assert.notEqual(seen[0].iceServers, seen[1].iceServers);
  releaseVoiceRtcConfiguration(first); releaseVoiceRtcConfiguration(second); owner.reset();
});

test("actual lease return gates reset and a new epoch; equal settings or a forged token are insufficient", () => {
  const owner = new VoiceRtcConfigurationOwner(), other = new VoiceRtcConfigurationOwner(), seen: RTCConfiguration[] = [];
  owner.capture(() => settings(), () => true); other.capture(() => settings(), () => true);
  const first = owner.borrow(), second = other.borrow(), forged = Object.freeze({}) as VoiceRtcConfigurationLease;
  assert.throws(() => voiceRtcConfiguredPeer(forged, constructor(seen), () => true), /configuration unavailable/);
  releaseVoiceRtcConfiguration(forged);
  assert.throws(() => owner.reset(), /configuration unavailable/);
  assert.equal(voiceRtcConfigurationCurrent(first), false); // Closing intent fences constructor immediately.
  assert.equal(voiceRtcConfigurationCurrent(second), true);
  assert.throws(() => owner.capture(() => settings(), () => true));
  releaseVoiceRtcConfiguration(first); releaseVoiceRtcConfiguration(first); owner.reset();
  owner.capture(() => settings({ stun: "", turn: "" }), () => true);
  assert.equal(voiceRtcConfigurationCurrent(first), false, "old allocation never revives");
  assert.equal(voiceRtcConfigurationCurrent(second), true);
  releaseVoiceRtcConfiguration(second); other.reset(); owner.reset(); assert.equal(seen.length, 0);
});

test("bounded original17 metadata leases cannot be expanded by repeated construction or refused reset", () => {
  const owner = new VoiceRtcConfigurationOwner(); owner.capture(() => settings(), () => true);
  const held = Array.from({ length: 17 }, () => owner.borrow());
  assert.throws(() => owner.borrow());
  assert.throws(() => owner.reset());
  assert.throws(() => owner.borrow());
  for (const lease of held) releaseVoiceRtcConfiguration(lease);
  owner.reset(); owner.capture(() => settings(), () => true);
  const next = owner.borrow(); assert.equal(voiceRtcConfigurationCurrent(held[0]), false);
  releaseVoiceRtcConfiguration(next); owner.reset();
});

test("last input getter and current callback can close capture without publishing a configuration", () => {
  for (const stage of ["getter", "current"] as const) {
    const owner = new VoiceRtcConfigurationOwner(); let reads = 0, current = 0;
    const input = { ...settings(), get policy() {
      reads++; if (stage === "getter") assert.throws(() => owner.reset()); return "all" as const;
    } };
    assert.throws(() => owner.capture(() => input, () => {
      if (++current === 2 && stage === "current") assert.throws(() => owner.reset()); return true;
    }));
    assert.equal(reads, 1); assert.throws(() => owner.borrow()); owner.reset();
  }
});

test("relay-only never includes STUN and no relay/credential refusal downgrades to direct", () => {
  const owner = new VoiceRtcConfigurationOwner(), seen: RTCConfiguration[] = [];
  owner.capture(() => settings({ policy: "relay" }), () => true);
  const lease = owner.borrow(); voiceRtcConfiguredPeer(lease, constructor(seen), () => true);
  assert.equal(seen[0].iceTransportPolicy, "relay");
  assert.equal(seen[0].iceServers!.length, 1); assert.match(seen[0].iceServers![0].urls as string, /^turns:/);
  releaseVoiceRtcConfiguration(lease); owner.reset();
  for (const input of [settings({ policy: "relay", turn: "" }), settings({ credential: "" }), settings({ username: "" })])
    assert.throws(() => owner.capture(() => input, () => true));
  assert.equal(seen.length, 1);
});

test("device URI validation accepts normal DNS, IPv4, expanded IPv6 and harmless separator noise", () => {
  for (const url of ["stun:localhost", "STUN:stun.example.test.", "stun:127.0.0.1:65535", "stun:[2001:0db8:0000::1]:3478"]) {
    const owner = new VoiceRtcConfigurationOwner(), seen: RTCConfiguration[] = [];
    owner.capture(() => settings({ stun: ",,  " + url + ",,\n", turn: "" }), () => true);
    const lease = owner.borrow(); voiceRtcConfiguredPeer(lease, constructor(seen), () => true);
    assert.equal(seen[0].iceServers!.length, 1);
    releaseVoiceRtcConfiguration(lease); owner.reset();
  }
  const owner = new VoiceRtcConfigurationOwner();
  owner.capture(() => settings({ stun: ",".repeat(VOICE_RTC_CONFIGURATION_UNITS), turn: "", username: "", credential: "" }), () => true);
  owner.reset();
});

test("oversized/malformed settings and unsupported URI shapes refuse without SDK entry or secret diagnostics", () => {
  const seen: RTCConfiguration[] = [];
  const invalid: unknown[] = [null, {}, Promise.resolve(settings()), settings({ stun: "x".repeat(4097) }),
    settings({ credential: "x".repeat(1025) }), settings({ username: "a\u0000b" }), settings({ stun: "https://stun.example.test" }),
    settings({ stun: "stun://host" }), settings({ stun: "stun:host/path" }), settings({ stun: "stun:host#fragment" }),
    settings({ stun: "stun:user:private-password@host" }), settings({ stun: "stun:host:0" }), settings({ stun: "stun:host:65536" }),
    settings({ stun: "stun:[broken]" }), settings({ stun: "stun:host?transport=tcp" }), settings({ turn: "turns:host?transport=udp" }),
    settings({ turn: "turn:host?transport=quic" }), settings({ turn: "stun:host" }), settings({ stun: Array(9).fill("stun:host").join(",") })];
  for (const value of invalid) {
    const owner = new VoiceRtcConfigurationOwner();
    assert.throws(() => owner.capture(() => value, () => true), error => {
      assert.ok(error instanceof Error); assert.equal(error.message, "voice RTC configuration unavailable"); return true;
    });
    assert.throws(() => owner.borrow()); owner.reset();
  }
  assert.equal(seen.length, 0);
});

test("last constructor-current callback can revoke the original snapshot before SDK entry", () => {
  const owner = new VoiceRtcConfigurationOwner(), seen: RTCConfiguration[] = [];
  owner.capture(() => settings(), () => true); const lease = owner.borrow();
  assert.throws(() => voiceRtcConfiguredPeer(lease, constructor(seen), () => { assert.throws(() => owner.reset()); return true; }));
  assert.equal(seen.length, 0); releaseVoiceRtcConfiguration(lease); owner.reset();
});

test("configuration capture never enumerates or retains an input graph's unrelated properties", () => {
  const owner = new VoiceRtcConfigurationOwner();
  const input = new Proxy(settings(), { ownKeys() { throw new Error("unbounded caller-key enumeration"); } });
  owner.capture(() => input, () => true);
  const lease = owner.borrow(), seen: RTCConfiguration[] = [];
  voiceRtcConfiguredPeer(lease, constructor(seen), () => true);
  assert.equal(seen.length, 1); releaseVoiceRtcConfiguration(lease); owner.reset();
});

test("coalesced configuration join stays pending through actual borrowed ownership and then permits a new call", async () => {
  const owner = new VoiceRtcConfigurationOwner(); owner.capture(() => settings(), () => true);
  const lease = owner.borrow(), joined = owner.join(); assert.equal(owner.join(), joined);
  let returned = false; void joined.then(() => { returned = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(returned, false); assert.throws(() => owner.reset()); assert.equal(voiceRtcConfigurationCurrent(lease), false);
  releaseVoiceRtcConfiguration(lease); await joined; assert.equal(returned, true); owner.reset();
  owner.capture(() => settings({ turn: "" }), () => true); owner.close(); await owner.join(); owner.reset();
});
