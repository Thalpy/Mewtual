// call-identity.ts: who a call peer is (design-creative-suite.md 2.18, Gate 5 slice C0d).
//
// Every call authority, session, replay, budget, consent, claim and slot key is the peer's FULL
// device identity: 64 lowercase hex characters, the `DeviceId` the backend verified from the
// signature on the call signal, in exactly the encoding `MemberView.identity` uses. The 8-hex
// fingerprint is a four-byte prefix of that id. A member can grind a device key sharing a victim's
// four bytes, so a fingerprint is a display label only and never decides whose edge, budget or
// claim something is.
//
// Two places deliberately keep the fingerprint, and both are labels rather than authority:
// - display (names, monograms, profile frames), because profiles are fingerprint-keyed app-wide;
//   `sharedFingerprints` lets a surface flag the case where that label is ambiguous;
// - the jam:v2 seed material and jam-take `parts`, whose published contract names the performer
//   by fingerprint (INTERFACES.md 12). Keeping them preserves byte-identical drum noise with older
//   builds in the same room and the take format. `callJamLabel` is that derivation.

const IDENTITY = /^[0-9a-f]{64}$/;

/** True for exactly the canonical full-identity spelling: 64 lowercase hex characters. */
export function isCallIdentity(value: unknown): value is string {
  return typeof value === "string" && IDENTITY.test(value);
}

/** The 8-hex display fingerprint of a full identity (its first four bytes). */
export function callFingerprint(identity: string): string {
  return identity.slice(0, 8);
}

/**
 * The key display code looks profiles up by: a full identity becomes its fingerprint, anything
 * else (already a fingerprint, the local "me" alias) is returned unchanged. Display only.
 */
export function displayKey(key: string): string {
  return isCallIdentity(key) ? callFingerprint(key) : key;
}

/**
 * The jam:v2 performer label for a call identity: its fingerprint. Seed material and take
 * `parts` are a reproducibility label, not authority; the engine keys capabilities, budgets and
 * voice ownership by the full identity separately.
 */
export function callJamLabel(identity: string): string {
  return callFingerprint(identity);
}

/**
 * Validate the sender pair a `call-signal` event carries. The bridge derives both strings from one
 * verified `DeviceId`, so any disagreement means a broken bridge rather than a hostile peer; the
 * signal is refused instead of guessing which half is right.
 */
export function parseCallSignalSender(event: { from?: unknown; from_fp?: unknown }): Readonly<{
  identity: string;
  fingerprint: string;
}> | null {
  const { from, from_fp: fingerprint } = event;
  if (!isCallIdentity(from) || typeof fingerprint !== "string") return null;
  if (fingerprint !== callFingerprint(from)) return null;
  return { identity: from, fingerprint };
}

export type CallRosterMember = Readonly<{ fingerprint: string; identity: string; you?: boolean }>;

/** This device's identity in a roster read, or "" when absent or malformed. */
export function rosterSelfIdentity(members: readonly CallRosterMember[]): string {
  const self = members.find((m) => m.you)?.identity;
  return isCallIdentity(self) ? self : "";
}

/**
 * Members to address a room-wide signal to: everyone the server calls online, minus self.
 *
 * Presence is still reported per fingerprint (`get_online_members`), so a prefix twin who is
 * online makes the member it collides with look online too. That only widens who is addressed:
 * the backend routes each signal by the exact identity's own signed peer record, so it can never
 * be delivered to the wrong device.
 */
export function onlineCallTargets(
  members: readonly CallRosterMember[],
  onlineFingerprints: ReadonlySet<string>,
  self: string,
): string[] {
  const out: string[] = [];
  for (const m of members) {
    if (!isCallIdentity(m.identity) || m.identity === self) continue;
    if (onlineFingerprints.has(m.fingerprint) && !out.includes(m.identity)) out.push(m.identity);
  }
  return out;
}

/** The full identities of a roster read, for revocation and hangup targeting. */
export function rosterIdentities(members: readonly CallRosterMember[]): string[] {
  return members.map((m) => m.identity).filter(isCallIdentity);
}

/**
 * Call peers that are no longer members and must be torn down (THREAT-MODEL: roster revocation
 * closes the removed member's call and data channels).
 *
 * Compared by full identity. Compared by fingerprint, a removed device whose prefix twin is still a
 * member kept its call edge, its jam lane and its claims.
 */
export function revokedCallPeers(peers: Iterable<string>, members: readonly CallRosterMember[]): string[] {
  const allowed = new Set(rosterIdentities(members));
  return [...peers].filter((peer) => !allowed.has(peer));
}

/**
 * Perfect-negotiation role: the lower identity is polite. Two different 8-hex prefixes order the
 * same way as their 64-hex extensions, so a Gate 5 build and an older build that compares
 * fingerprints still pick opposite roles; only an exact prefix collision can disagree, and the
 * older build is ambiguous there already.
 */
export function isPoliteTowards(self: string, peer: string): boolean {
  return self < peer;
}

/** Identities whose fingerprint another roster identity also carries. */
export function sharedFingerprints(members: readonly CallRosterMember[]): ReadonlySet<string> {
  const byFingerprint = new Map<string, Set<string>>();
  for (const m of members) {
    if (!isCallIdentity(m.identity)) continue;
    const fp = callFingerprint(m.identity);
    const set = byFingerprint.get(fp) ?? new Set<string>();
    set.add(m.identity);
    byFingerprint.set(fp, set);
  }
  const shared = new Set<string>();
  for (const set of byFingerprint.values()) if (set.size > 1) for (const identity of set) shared.add(identity);
  return shared;
}

/**
 * A display label that stays honest when a fingerprint is ambiguous. Names and profiles are
 * fingerprint-keyed, so two devices sharing four bytes render under the same name; when that is
 * the case the label adds a longer slice of the full identity and says why.
 */
export function identityLabel(name: string, identity: string, shared: ReadonlySet<string>): string {
  if (!shared.has(identity)) return name;
  return `${name} (${identity.slice(0, 12)}…${identity.slice(-4)}, shares a fingerprint)`;
}
