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

test("malformed server policies fail closed to on-demand; a missing one is media only", () => {
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
  assert.equal(DEFAULT_FILE_TRUST_POLICY.mode, "media");
});

test("the retired specific mode reads as on-demand with its trusted people kept as overrides", () => {
  const policies = sanitizeFileTrustPolicies({ 1: { mode: "specific", trustedAuthors: ["alice"] } });
  assert.deepEqual(policies[1], { mode: "on-demand", trustedAuthors: ["alice"], blockedAuthors: [] });
  // Same behaviour as before the upgrade: alice loads, nobody else does.
  assert.equal(mayAutoLoadFile(policies[1], "alice", true), true);
  assert.equal(mayAutoLoadFile(policies[1], "bob", true), false);
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
  assert.equal(mayAutoLoadFile(onDemand, "alice", true, true), false);
  assert.equal(mayAutoLoadFile({ ...onDemand, trustedAuthors: ["alice"] }, "alice", true, false), true);
  assert.equal(mayAutoLoadFile({ ...onDemand, trustedAuthors: ["alice"] }, "alice", false, true), false, "always needs attested authorship");
  // Media only: media loads, other files do not.
  assert.equal(mayAutoLoadFile(media, "alice", true, true), true);
  assert.equal(mayAutoLoadFile(media, "alice", true, false), false);
  assert.equal(mayAutoLoadFile(media, "mallory", false, true), true, "like everyone, media only does not need attestation");
  // Everyone: all files, unless the person is marked never.
  assert.equal(mayAutoLoadFile(everyone, "mallory", false, false), true);
  assert.equal(mayAutoLoadFile({ ...everyone, blockedAuthors: ["mallory"] }, "mallory", false, true), false, "never holds even on a claimed name");
  assert.equal(mayAutoLoadFile({ ...media, blockedAuthors: ["alice"] }, "alice", true, true), false);
});
