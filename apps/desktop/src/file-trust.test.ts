import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  DEFAULT_FILE_TRUST_POLICY,
  authorOverride,
  fileTrustPolicyFor,
  mayAutoLoadFile,
  mayAutoLoadRemoteUrl,
  mayLoadJukeboxFile,
  sanitizeFileTrustPolicies,
  scopedMediaKey,
  setAuthorOverride,
} from "./file-trust.ts";

const onDemand = { mode: "on-demand" as const, trustedAuthors: [], blockedAuthors: [] };
const media = { mode: "media" as const, trustedAuthors: [], blockedAuthors: [] };
const everyone = { mode: "everyone" as const, trustedAuthors: [], blockedAuthors: [] };

test("malformed server policies fail closed to on-demand, and so does a missing one", () => {
  const policies = sanitizeFileTrustPolicies({
    1: { mode: "media", trustedAuthors: ["alice", "alice", "bob"], blockedAuthors: ["bob"] },
    2: { mode: "everyone", trustedAuthors: [] },
    3: { mode: "automatic", trustedAuthors: ["mallory"] },
    "03": { mode: "everyone" },
    "-1": { mode: "everyone" },
  });
  assert.deepEqual(policies, {
    // One person cannot be both trusted and blocked: the block wins.
    1: { mode: "media", trustedAuthors: ["alice"], blockedAuthors: ["bob"] },
    2: { mode: "everyone", trustedAuthors: [], blockedAuthors: [] },
    3: { mode: "on-demand", trustedAuthors: ["mallory"], blockedAuthors: [] },
  });
  assert.deepEqual(fileTrustPolicyFor(policies, 99), DEFAULT_FILE_TRUST_POLICY);
});

test("SEC-DEFAULT-001: the shipped default decodes untrusted media with no gesture, knowingly", () => {
  // This is a pin on an accepted risk, not an endorsement. Alpha ships media-only as the default,
  // which means a current member's image reaches the platform decoders inside the webview that
  // holds the account, before anyone clicks anything. Nothing in this file makes that safe, and
  // nothing anywhere else does either: the native container validation establishes that the bytes
  // are the ones the sender sent, which is a different claim.
  //
  // The test exists so the decision stays deliberate. Anyone narrowing it to on-demand is doing
  // the safe thing and should edit this freely. Anyone widening it further, or restoring it after
  // a narrowing, is making a security decision that needs a demonstrated privilege boundary
  // around automatic decoding on the packaged Windows runtime, not an argument that it is
  // probably fine.
  assert.equal(DEFAULT_FILE_TRUST_POLICY.mode, "media");
  assert.deepEqual(DEFAULT_FILE_TRUST_POLICY.trustedAuthors, []);
  assert.deepEqual(DEFAULT_FILE_TRUST_POLICY.blockedAuthors, []);
  // The default is what an unconfigured server gets, so the two must not drift apart. Whatever
  // the mode is, the default must never arrive pre-loaded with someone else's overrides.
  const unconfigured = fileTrustPolicyFor({}, 7);
  assert.deepEqual(unconfigured, DEFAULT_FILE_TRUST_POLICY);
  // The one thing the default must still narrow, whatever it is set to: a file the renderer does
  // not recognise as media never loads on its own. That is the difference between this mode and
  // "everyone", and it is the half of the promise that does not depend on decoder safety.
  assert.equal(mayAutoLoadFile(unconfigured, "alice", true, "non-media"), false);
  assert.equal(mayAutoLoadFile(unconfigured, "mallory", false, "non-media"), false);
});

test("the retired specific mode reads as on-demand with its trusted people kept as overrides", () => {
  const policies = sanitizeFileTrustPolicies({ 1: { mode: "specific", trustedAuthors: ["alice"] } });
  assert.deepEqual(policies[1], { mode: "on-demand", trustedAuthors: ["alice"], blockedAuthors: [] });
  // Same behaviour as before the upgrade: alice loads, nobody else does.
  assert.equal(mayAutoLoadFile(policies[1], "alice", true, "validated-media"), true);
  assert.equal(mayAutoLoadFile(policies[1], "bob", true, "validated-media"), false);
});

test("a per-person override cannot authenticate a forged author on a remote URL", () => {
  assert.equal(mayAutoLoadRemoteUrl({ ...onDemand, trustedAuthors: ["alice"] }), false);
  assert.equal(mayAutoLoadRemoteUrl(everyone), false);
});

test("jukebox adoption is gated unless the listed origin is trusted or playback is explicit", () => {
  const trustAlice = { ...onDemand, trustedAuthors: ["alice"] };
  assert.equal(mayLoadJukeboxFile(onDemand, "alice", true, false), false);
  assert.equal(mayLoadJukeboxFile(trustAlice, "mallory", true, false), false);
  assert.equal(mayLoadJukeboxFile(trustAlice, "alice", false, false), false);
  assert.equal(mayLoadJukeboxFile(trustAlice, "alice", true, false), true);
  assert.equal(mayLoadJukeboxFile(everyone, "mallory", false, false), true);
  assert.equal(mayLoadJukeboxFile(onDemand, "mallory", false, true), true);
  // A jukebox entry is media, so the media-only mode plays it.
  assert.equal(mayLoadJukeboxFile(media, "mallory", false, false), true);
});

test("media URL cache keys preserve server separation for equal CIDs", () => {
  assert.notEqual(scopedMediaKey(1, "same-cid"), scopedMediaKey(2, "same-cid"));
});

test("security-sensitive roster choices use and reveal the full device identity", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(source, /#each roster as member \(member\.identity\)/);
  assert.match(source, /setFileAuthorOverride\(member\.identity, /);
  assert.match(source, /Full device identity:/);
});

test("jukebox playback uses the call-server index and exposes an explicit consent action", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(source, /callFiles\.find\(\(candidate\) => candidate\.cid === cid\)/);
  assert.match(source, /mayLoadJukeboxFile\(/);
  assert.match(source, />LOAD TRACK<\/button>/);
  assert.match(source, /Click to allow it for this call/);
  assert.match(source, /if \(inCall && e\.payload\.server === callServer\) void refreshCallFiles\(\)/);
  assert.ok(source.indexOf("activeCallLease = joinLease;") < source.indexOf("void refreshCallFiles();", source.indexOf("activeCallLease = joinLease;")));
});

test("server onboarding is gated until vault-sealed trust policy has loaded", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const continuityGate = source.indexOf("{:else if !uiStateReady}");
  const onboarding = source.indexOf("{:else if servers.length === 0 || showAdd}");
  assert.ok(continuityGate >= 0 && continuityGate < onboarding);
  assert.match(source, /let uiStateReady = \$state\(false\)/);
  assert.match(source, /if \(!r\.is_dm\) \{[\s\S]*\[r\.server\]: \{ mode: onboardingFileTrust/);
  assert.match(source, /\[r\.server\]: \{ mode: onboardingFileTrust[\s\S]*void saveUiStateImmediately\(\)/);
});

test("file-trust changes bypass the ordinary continuity debounce", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const modeSetter = source.slice(source.indexOf("function setFileTrustMode("), source.indexOf("function setFileAuthorOverride("));
  const authorSetter = source.slice(source.indexOf("function setFileAuthorOverride("), source.indexOf("function revokePassiveMedia("));
  assert.match(modeSetter, /void saveUiStateImmediately\(\)/);
  assert.doesNotMatch(modeSetter, /scheduleUiStateSave\(\)/);
  assert.match(authorSetter, /void saveUiStateImmediately\(\)/);
  assert.doesNotMatch(authorSetter, /scheduleUiStateSave\(\)/);
});

test("leaving the call server ends capture before awaiting native server removal", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const leave = source.slice(source.indexOf("async function leaveServer("), source.indexOf("async function addChannel("));
  assert.ok(leave.indexOf("if (inCall && callServer === id) leaveVoice();") < leave.indexOf('await invoke("leave_server"'));
});

test("per-person overrides are exact, exclusive, removable, and bounded", () => {
  let policy = { ...media, trustedAuthors: ["alice"] };
  policy = setAuthorOverride(policy, "bob", "always");
  assert.deepEqual(policy.trustedAuthors, ["alice", "bob"]);
  policy = setAuthorOverride(policy, "alice", "never");
  assert.deepEqual(policy.trustedAuthors, ["bob"]);
  assert.deepEqual(policy.blockedAuthors, ["alice"]);
  assert.equal(authorOverride(policy, "alice"), "never");
  assert.equal(authorOverride(policy, "bob"), "always");
  assert.equal(authorOverride(policy, "carol"), "follow");
  policy = setAuthorOverride(policy, "alice", "follow");
  assert.deepEqual(policy.blockedAuthors, []);
  assert.equal(policy.mode, "media", "an override never changes the mode");
  const full = { ...media, trustedAuthors: Array.from({ length: 32 }, (_, i) => `member-${i}`) };
  assert.equal(setAuthorOverride(full, "one-too-many", "always").trustedAuthors.length, 32);
});

test("the mode decides the default and the overrides win either way", () => {
  // On demand: nothing passive, unless the person is marked always.
  assert.equal(mayAutoLoadFile(onDemand, "alice", true, "validated-media"), false);
  assert.equal(mayAutoLoadFile({ ...onDemand, trustedAuthors: ["alice"] }, "alice", true, "non-media"), true);
  assert.equal(mayAutoLoadFile({ ...onDemand, trustedAuthors: ["alice"] }, "alice", false, "validated-media"), false, "always needs attested authorship");
  // Media only: media loads, other files do not.
  assert.equal(mayAutoLoadFile(media, "alice", true, "validated-media"), true);
  assert.equal(mayAutoLoadFile(media, "alice", true, "non-media"), false);
  assert.equal(mayAutoLoadFile(media, "mallory", false, "validated-media"), true, "like everyone, media only does not need attestation");
  // Everyone: all files, unless the person is marked never.
  assert.equal(mayAutoLoadFile(everyone, "mallory", false, "non-media"), true);
  assert.equal(mayAutoLoadFile({ ...everyone, blockedAuthors: ["mallory"] }, "mallory", false, "validated-media"), false, "never holds even on a claimed name");
  assert.equal(mayAutoLoadFile({ ...media, blockedAuthors: ["alice"] }, "alice", true, "validated-media"), false);
});

test("SEC-MEDIA-002: the media-only mode never passively loads a non-media file", () => {
  // The mode's whole promise is that it is narrower than "everyone". A caller that classifies a
  // document, an archive or an unrecognised type must get a click, whoever sent it, and a
  // per-person always override is the only thing that widens it.
  assert.equal(mayAutoLoadFile(media, "alice", true, "non-media"), false);
  assert.equal(mayAutoLoadFile(media, "mallory", false, "non-media"), false);
  assert.equal(mayAutoLoadFile({ ...media, trustedAuthors: ["alice"] }, "alice", true, "non-media"), true);
  assert.equal(mayAutoLoadFile({ ...media, trustedAuthors: ["alice"] }, "bob", true, "non-media"), false);
});

test("a never override outranks every mode, both classifications and any claimed authorship", () => {
  // Failing closed on a name nobody attested costs nothing, so the block does not ask for proof.
  for (const base of [onDemand, media, everyone]) {
    const blocked = { ...base, blockedAuthors: ["mallory"] };
    for (const fileClass of ["validated-media", "non-media"] as const) {
      for (const verified of [true, false]) {
        assert.equal(
          mayAutoLoadFile(blocked, "mallory", verified, fileClass),
          false,
          `${base.mode}/${fileClass}/verified=${verified}`,
        );
      }
    }
    // And the block still wins when the same person is somehow also on the trusted list.
    assert.equal(mayAutoLoadFile({ ...blocked, trustedAuthors: ["mallory"] }, "mallory", true, "validated-media"), false);
  }
});
