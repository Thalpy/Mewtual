import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  DEFAULT_FILE_TRUST_POLICY,
  fileTrustPolicyFor,
  mayAutoLoadFile,
  mayAutoLoadRemoteUrl,
  mayLoadJukeboxFile,
  sanitizeFileTrustPolicies,
  scopedMediaKey,
  toggleTrustedAuthor,
} from "./file-trust.ts";

test("missing and malformed server policies fail closed to on-demand", () => {
  const policies = sanitizeFileTrustPolicies({
    1: { mode: "specific", trustedAuthors: ["alice", "alice", "bob"] },
    2: { mode: "everyone", trustedAuthors: [] },
    3: { mode: "automatic", trustedAuthors: ["mallory"] },
    "03": { mode: "everyone" },
    "-1": { mode: "everyone" },
  });
  assert.deepEqual(policies, {
    1: { mode: "specific", trustedAuthors: ["alice", "bob"] },
    2: { mode: "everyone", trustedAuthors: [] },
    3: { mode: "on-demand", trustedAuthors: ["mallory"] },
  });
  assert.deepEqual(fileTrustPolicyFor(policies, 99), DEFAULT_FILE_TRUST_POLICY);
});

test("specific-file trust cannot authenticate a forged author on a remote URL", () => {
  const policy = { mode: "specific" as const, trustedAuthors: ["alice"] };
  assert.equal(mayAutoLoadRemoteUrl(policy), false);
  assert.equal(mayAutoLoadRemoteUrl({ mode: "everyone", trustedAuthors: [] }), false);
});

test("jukebox adoption is gated unless the listed origin is trusted or playback is explicit", () => {
  const onDemand = DEFAULT_FILE_TRUST_POLICY;
  const specific = { mode: "specific" as const, trustedAuthors: ["alice"] };
  const everyone = { mode: "everyone" as const, trustedAuthors: [] };
  assert.equal(mayLoadJukeboxFile(onDemand, "alice", true, false), false);
  assert.equal(mayLoadJukeboxFile(specific, "mallory", true, false), false);
  assert.equal(mayLoadJukeboxFile(specific, "alice", false, false), false);
  assert.equal(mayLoadJukeboxFile(specific, "alice", true, false), true);
  assert.equal(mayLoadJukeboxFile(everyone, "mallory", false, false), true);
  assert.equal(mayLoadJukeboxFile(onDemand, "mallory", false, true), true);
});

test("media URL cache keys preserve server separation for equal CIDs", () => {
  assert.notEqual(scopedMediaKey(1, "same-cid"), scopedMediaKey(2, "same-cid"));
});

test("security-sensitive roster choices use and reveal the full device identity", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(source, /#each roster as member \(member\.identity\)/);
  assert.match(source, /toggleTrustedFileAuthor\(member\.identity\)/);
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
  const modeSetter = source.slice(source.indexOf("function setFileTrustMode("), source.indexOf("function toggleTrustedFileAuthor("));
  const authorSetter = source.slice(source.indexOf("function toggleTrustedFileAuthor("), source.indexOf("function revokePassiveMedia("));
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

test("specific-member toggles are exact, removable, and bounded", () => {
  let policy = { mode: "specific" as const, trustedAuthors: ["alice"] };
  policy = toggleTrustedAuthor(policy, "bob");
  assert.deepEqual(policy.trustedAuthors, ["alice", "bob"]);
  policy = toggleTrustedAuthor(policy, "alice");
  assert.deepEqual(policy.trustedAuthors, ["bob"]);
  policy = { mode: "specific", trustedAuthors: Array.from({ length: 32 }, (_, i) => `member-${i}`) };
  assert.equal(toggleTrustedAuthor(policy, "one-too-many").trustedAuthors.length, 32);
});

test("automatic loads require the server mode or an exact trusted author", () => {
  assert.equal(mayAutoLoadFile(DEFAULT_FILE_TRUST_POLICY, "alice", true), false);
  assert.equal(mayAutoLoadFile({ mode: "specific", trustedAuthors: ["alice"] }, "alice", true), true);
  assert.equal(mayAutoLoadFile({ mode: "specific", trustedAuthors: ["alice"] }, "alice", false), false);
  assert.equal(mayAutoLoadFile({ mode: "specific", trustedAuthors: ["alice"] }, "bob", true), false);
  assert.equal(mayAutoLoadFile({ mode: "everyone", trustedAuthors: [] }, "mallory", false), true);
});
