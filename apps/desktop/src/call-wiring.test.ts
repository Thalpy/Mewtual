import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

// App.svelte has no node-level harness, so the Gate 5 call-identity and claim-lifecycle wiring
// is pinned at the source level here. The behaviour itself is tested in call-identity.test.ts
// and draw-claims.test.ts; these checks keep the App from silently routing around those modules.

const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
const studio = readFileSync(fileURLToPath(new URL("./Studio.svelte", import.meta.url)), "utf8");

test("call diagnostics describe mesh advertisements without promising media connectivity", () => {
  const start = app.indexOf('<section class="connection-voice"');
  const view = app.slice(start, app.indexOf("</section>", start));
  assert.match(view, /public mesh address advertised/);
  assert.match(view, /calls can try a separate UDP mapping/);
  assert.match(view, /MESH HELPERS/);
  assert.doesNotMatch(view, /this device is directly reachable|behind NAT|each call's media socket is offered|RELAY HOSTS/);
});

/** The text of one `function name(` up to the next function at the same indentation. */
function body(source: string, signature: string): string {
  const start = source.indexOf(signature);
  assert.ok(start >= 0, `${signature} exists`);
  const next = source.slice(start + signature.length).search(/\n {2}(async )?function /);
  return next < 0 ? source.slice(start) : source.slice(start, start + signature.length + next);
}

test("inbound call signals are keyed by the verified full identity, never the fingerprint", () => {
  const listener = app.slice(app.indexOf('("call-signal"'), app.indexOf('("call-signal"') + 700);
  assert.match(listener, /parseCallSignalSender\(e\.payload\)/);
  assert.match(listener, /handleCallSignal\(sender\.identity,/);
  assert.doesNotMatch(listener, /handleCallSignal\(e\.payload\.from_fp/);
  assert.match(body(app, "async function handleCallSignal("), /fromFp === callSelfId\) return/);
});

test("outbound signals, room broadcasts, hangups and revocation all use full identities", () => {
  assert.match(body(app, "async function sendSignal("), /"send_call_signal", \{\s*server,\s*target,/);
  const broadcast = body(app, "async function broadcastOn(");
  assert.match(broadcast, /onlineCallTargets\(membersHere/);
  assert.doesNotMatch(broadcast, /m\.fingerprint/);
  const hangup = body(app, "function announceHangup(");
  assert.match(hangup, /rosterIdentities\(membersHere\)/);
  assert.doesNotMatch(hangup, /\.fingerprint\)/);
  const roster = body(app, "async function reconcileCallRoster(");
  assert.match(roster, /revokedCallPeers\(Object\.keys\(callPeers\), members\)/);
  assert.doesNotMatch(roster, /\.fingerprint/);
  assert.match(body(app, "async function joinVoice("), /rosterSelfIdentity\(membersHere\)/);
  assert.match(body(app, "function createPeer("), /polite: isPoliteTowards\(callSelfId, fp\)/);
  assert.doesNotMatch(app, /\bcallSelfFp\b/, "the old fingerprint self key is gone");
});

test("jam keeps the published fingerprint seed label while authority is the full identity", () => {
  const create = body(app, "function createPeer(");
  assert.match(create, /openSource\(fp, callJamLabel\(fp\)\)/);
  assert.match(body(app, "function ensureJamEngine("), /openLocalSource\(callJamLabel\(callSelfId\)\)/);
  assert.match(body(app, "function jamRecParticipants("), /map\(callJamLabel\)/);
});

test("every edge gets a negotiated draw channel bound to a claim capability, torn down with it", () => {
  // The wiring itself lives in draw-wiring.ts (behaviour-tested there, and run over real WebRTC by
  // scripts/claims-webrtc-check.mjs); the App must use it, for this exact peer and session.
  const create = body(app, "function createPeer(");
  assert.match(create, /peer\.draw = openDrawEdge\(pc, claims, fp, \{\s*live: \(\) => isCurrentCallPeer\(peer\),/);
  const current = body(app, "function isCurrentCallPeer(");
  assert.match(current, /callPeers\[peer\.fp\] === peer/);
  assert.match(current, /isCurrentCallJoin\(peer\.callLease, peer\.server, peer\.channel\)/);
  const join = body(app, "function isCurrentCallJoin(");
  assert.match(join, /lease === activeCallLease/);
  assert.match(join, /callLifecycleSession\.isCurrent\(lease\)/);
  assert.match(create, /drawConnectionState\(claims, fp, pc\.connectionState, peer\.draw, syncDrawClaims\)/);
  assert.doesNotMatch(app, /createDataChannel\(DRAW_CHANNEL_LABEL/, "no second, hand-rolled draw wiring");
  assert.match(body(app, "function removePeer("), /withDrawClaims\(\(s\) => s\.removePeer\(fp\)\)/);
  const leave = body(app, "function leaveVoice(");
  assert.match(leave, /stopDrawClaims\(\)/);
  assert.match(body(app, "async function joinVoice("), /startDrawClaims\(selfId\)/);
  assert.match(body(app, "function stopDrawClaims("), /drawClaims\?\.dispose\(\)/);
});

test("the Studio receives claims only while it is open on the call's own server", () => {
  assert.match(app, /let studioClaimScope = \$derived\(inCall && callServer !== null && activeServerId === callServer && view === "studio"\)/);
  assert.match(app, /claims=\{studioClaimScope \? drawClaimSnapshot : null\} claimActions=\{studioClaimActions\}/);
  // Every asserting action is re-checked against the scope at call time.
  for (const action of ["claim", "autoClaim", "ask"]) {
    assert.match(app, new RegExp(`${action}: \\(obj: string, fr: string\\) => withDrawClaims\\(\\(s\\) => \\{ if \\(studioClaimScope\\) s\\.${action}\\(obj, fr\\)`));
  }
  // Painting claims; claims never gate painting or saving.
  assert.match(body(studio, "function edited("), /claimActions\?\.autoClaim\(objectId, frameId\)/);
  assert.doesNotMatch(body(studio, "function commit("), /claim/i);
  assert.doesNotMatch(studio.slice(studio.indexOf("const canEdit"), studio.indexOf("const canEdit") + 200), /claim/i);
});

test("local per-peer trims stay keyed by fingerprint, so saved levels survive the identity re-key", () => {
  assert.match(app, /const peerLevelKey = \(key: string, kind: PeerAudioKind\) =>\s*kind === "voice" \? `catcoms\.call\.level\.\$\{displayKey\(key\)\}`/);
  assert.match(app, /const peerInstKey = \(key: string\) => `catcoms\.call\.instlevel\.\$\{displayKey\(key\)\}`/);
  // Display helpers accept a full identity and resolve it through the fingerprint-keyed profiles.
  assert.match(body(app, "function nameOf("), /const fp = displayKey\(key\)/);
  assert.match(body(app, "function callNameOf("), /identityLabel\(name, key, callSharedIds\)/);
});

test("the stage context menu acts on the same full-identity key as every per-peer map", () => {
  // Re-keyed to the fingerprint, the menu's mute/volume/instrument actions silently addressed no
  // chain, engine source or budget, and reported a mute that never happened.
  assert.match(app, /use:contextMenu=\{\(\) => peerMenu\(fp\)\}/);
  assert.doesNotMatch(app, /peerMenu\(callFingerprint\(/);
  const menu = body(app, "function peerMenu(");
  assert.match(menu, /setPeerVolume\(fp, v\)/);
  assert.match(menu, /toggleVoicePeer\(fp\)/);
  assert.match(menu, /toggleInstPeer\(fp\)/);
  assert.match(menu, /showProfile\(label\)/, "only the profile card wants the fingerprint");
});

test("claim snapshots are rebuilt only when the session changed something visible", () => {
  // A refused or no-op frame (a flood, an auto-muted peer) and a paint sample on a frame already
  // claimed must not re-render the timeline.
  assert.match(body(app, "function withDrawClaims("), /if \(session\.rev !== before\) syncDrawClaims\(\)/);
  const create = body(app, "function createPeer(");
  assert.match(create, /changed: \(\) => \{\s*if \(drawClaims === claims\) syncDrawClaims\(\);/);
  assert.match(body(app, "function startDrawClaims("), /session\.rev !== before \|\| !session\.isQuiet\(\)/);
  assert.match(body(studio, "function edited("), /claims\.local\.claim\?\.obj !== objectId \|\| claims\.local\.claim\?\.fr !== frameId/);
});

