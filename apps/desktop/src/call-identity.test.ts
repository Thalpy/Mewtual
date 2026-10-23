import assert from "node:assert/strict";
import test from "node:test";
import {
  callFingerprint,
  callJamLabel,
  displayKey,
  identityLabel,
  isCallIdentity,
  isPoliteTowards,
  isVoiceInitiatorTowards,
  isVoiceNonce,
  isVoiceRoomId,
  onlineCallTargets,
  parseCallSignalSender,
  parseVoiceOfferId,
  revokedCallPeers,
  rosterSelfIdentity,
  sameVoiceOffer,
  sharedFingerprints,
  voiceOfferKey,
} from "./call-identity.ts";
import { hangupTargets } from "./voice-signaling.ts";

// Two devices that share their first four bytes: the fingerprint a member can collide with by
// grinding a device key. Everything below must keep them apart.
const VICTIM = "1a2b3c4d" + "0".repeat(56);
const TWIN = "1a2b3c4d" + "f".repeat(56);
const OTHER = "99887766" + "1".repeat(56);
const ME = "00112233" + "2".repeat(56);

test("only the canonical 64-lowercase-hex spelling is a call identity", () => {
  assert.equal(isCallIdentity(VICTIM), true);
  assert.equal(isCallIdentity(VICTIM.toUpperCase()), false);
  assert.equal(isCallIdentity(VICTIM.slice(0, 8)), false, "a fingerprint is not an identity");
  assert.equal(isCallIdentity(`${VICTIM}0`), false);
  assert.equal(isCallIdentity(` ${VICTIM.slice(1)}`), false);
  assert.equal(isCallIdentity(42), false);
  assert.equal(callFingerprint(VICTIM), "1a2b3c4d");
  assert.equal(callJamLabel(TWIN), "1a2b3c4d", "the jam seed/take label stays the published fingerprint");
  assert.equal(displayKey(VICTIM), "1a2b3c4d");
  assert.equal(displayKey("1a2b3c4d"), "1a2b3c4d", "a fingerprint is already a display key");
  assert.equal(displayKey("me"), "me");
});

test("a call-signal sender pair must agree, or the signal is refused", () => {
  assert.deepEqual(parseCallSignalSender({ from: VICTIM, from_fp: "1a2b3c4d" }), { identity: VICTIM, fingerprint: "1a2b3c4d" });
  assert.equal(parseCallSignalSender({ from: VICTIM, from_fp: "99887766" }), null);
  assert.equal(parseCallSignalSender({ from: "1a2b3c4d", from_fp: "1a2b3c4d" }), null, "an old bridge's fingerprint-only event");
  assert.equal(parseCallSignalSender({ from_fp: "1a2b3c4d" }), null);
  assert.equal(parseCallSignalSender({ from: VICTIM.toUpperCase(), from_fp: "1A2B3C4D" }), null);
  assert.equal(parseCallSignalSender({ from: VICTIM, from_fp: 7 }), null);
});

const ROSTER = [
  { fingerprint: "00112233", identity: ME, you: true },
  { fingerprint: "1a2b3c4d", identity: VICTIM },
  { fingerprint: "1a2b3c4d", identity: TWIN },
  { fingerprint: "99887766", identity: OTHER },
];

test("roster revocation compares full identities, so a prefix twin cannot shield a removed device", () => {
  // VICTIM is removed; TWIN, who shares its fingerprint, stays. Compared by fingerprint, the removed
  // device kept its call edge because its twin's fingerprint was still allowed.
  const after = ROSTER.filter((m) => m.identity !== VICTIM);
  assert.deepEqual(revokedCallPeers([VICTIM, TWIN, OTHER], after), [VICTIM]);
  assert.deepEqual(revokedCallPeers([TWIN, OTHER], after), []);
  // Compared by identity there is no confusion in the other direction either: nothing is removed
  // merely because a member's fingerprint matches a peer key.
  assert.deepEqual(revokedCallPeers([VICTIM, TWIN], ROSTER), []);
  // A malformed roster entry allows nobody.
  assert.deepEqual(revokedCallPeers([VICTIM], [{ fingerprint: "1a2b3c4d", identity: "1a2b3c4d" }]), [VICTIM]);
});

test("room-wide signals target identities, never self, and the twin only widens addressing", () => {
  assert.equal(rosterSelfIdentity(ROSTER), ME);
  assert.equal(rosterSelfIdentity([{ fingerprint: "00112233", identity: "00112233", you: true }]), "");
  // Presence is reported per fingerprint, so one online twin makes both look online. Each
  // signal is still routed by its exact identity's own record in the backend.
  assert.deepEqual(onlineCallTargets(ROSTER, new Set(["1a2b3c4d", "00112233"]), ME), [VICTIM, TWIN]);
  assert.deepEqual(onlineCallTargets(ROSTER, new Set(["99887766"]), ME), [OTHER]);
  // The hangup union now runs over identities: an edge with the twin does not stand in for the victim.
  const online = new Set(onlineCallTargets(ROSTER, new Set(["99887766"]), ME));
  assert.deepEqual(hangupTargets([TWIN], ROSTER.map((m) => m.identity), online, ME).sort(), [OTHER, TWIN].sort());
});

test("politeness keeps the order an older fingerprint-comparing build would choose", () => {
  // Distinct prefixes: the full ids order exactly as their fingerprints do.
  assert.equal(isPoliteTowards(ME, OTHER), "00112233" < "99887766");
  assert.equal(isPoliteTowards(OTHER, ME), "99887766" < "00112233");
  // A prefix collision is still decided, consistently and oppositely on each side.
  assert.notEqual(isPoliteTowards(VICTIM, TWIN), isPoliteTowards(TWIN, VICTIM));
});

test("V2 nonces and rooms accept exactly the canonical native spellings", () => {
  assert.equal(isVoiceNonce("ab".repeat(32)), true);
  assert.equal(isVoiceNonce("0".repeat(64)), false, "native never mints the all-zero nonce");
  assert.equal(isVoiceNonce("AB".repeat(32)), false);
  assert.equal(isVoiceNonce("ab".repeat(31)), false);
  assert.equal(isVoiceNonce(7), false);
  assert.equal(isVoiceRoomId("0"), true);
  assert.equal(isVoiceRoomId("7"), true);
  assert.equal(isVoiceRoomId(((1n << 128n) - 1n).toString()), true);
  assert.equal(isVoiceRoomId((1n << 128n).toString()), false, "one above u128 is not a room");
  assert.equal(isVoiceRoomId("07"), false, "a second spelling of 7 must not name another room");
  assert.equal(isVoiceRoomId("-1"), false);
  assert.equal(isVoiceRoomId("1e3"), false);
  assert.equal(isVoiceRoomId(""), false);
  assert.equal(isVoiceRoomId(7), false);
});

test("an OfferId names its exact issuer and native message, and nothing else equals it", () => {
  const message = "cd".repeat(32);
  const offer = parseVoiceOfferId({ issuer: VICTIM, message });
  assert.deepEqual(offer, { issuer: VICTIM, message });
  assert.ok(Object.isFrozen(offer));
  assert.equal(parseVoiceOfferId({ issuer: VICTIM.slice(0, 8), message }), null, "a fingerprint never issues an offer");
  assert.equal(parseVoiceOfferId({ issuer: VICTIM, message: "0".repeat(64) }), null);
  assert.equal(parseVoiceOfferId(null), null);
  assert.equal(parseVoiceOfferId("offer"), null);
  // The twin shares four bytes with the victim; its offer with the same message is another offer.
  const twin = parseVoiceOfferId({ issuer: TWIN, message });
  assert.equal(sameVoiceOffer(offer, twin), false);
  assert.notEqual(voiceOfferKey(offer!), voiceOfferKey(twin!));
  assert.equal(sameVoiceOffer(offer, parseVoiceOfferId({ issuer: VICTIM, message })), true);
  assert.equal(sameVoiceOffer(null, null), false, "absence never matches absence");
});

test("exactly one end of a V2 pair initiates, matching native confirmation order", () => {
  assert.equal(isVoiceInitiatorTowards(ME, OTHER), true);
  assert.equal(isVoiceInitiatorTowards(OTHER, ME), false);
  // Prefix twins still resolve to opposite roles: the full identities decide, not fingerprints.
  assert.notEqual(isVoiceInitiatorTowards(VICTIM, TWIN), isVoiceInitiatorTowards(TWIN, VICTIM));
  assert.equal(isVoiceInitiatorTowards(ME, ME), false, "self is never a counterpart");
  assert.equal(isVoiceInitiatorTowards("00112233", OTHER), false, "a fingerprint has no role");
});

test("a shared fingerprint is flagged in labels instead of silently reusing one name", () => {
  const shared = sharedFingerprints(ROSTER);
  assert.deepEqual([...shared].sort(), [VICTIM, TWIN].sort());
  assert.equal(identityLabel("vic", OTHER, shared), "vic");
  const a = identityLabel("vic", VICTIM, shared);
  const b = identityLabel("vic", TWIN, shared);
  assert.notEqual(a, b, "two devices under one profile name still render differently");
  assert.match(a, /shares a fingerprint/);
});
