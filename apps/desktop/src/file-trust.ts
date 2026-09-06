/** Local, vault-sealed policy for content a server may fetch/render without a click. */
export type FileTrustMode = "on-demand" | "media" | "everyone";

/**
 * A per-person override that applies whatever the server's mode says: `always` fetches that
 * person's attested files passively even under on-demand, `never` keeps them click-only even
 * under everyone. `follow` is the absence of an override.
 */
export type FileAuthorOverride = "follow" | "always" | "never";

export type FileTrustPolicy = {
  mode: FileTrustMode;
  /** Full origin DeviceIds whose attested files may be fetched/rendered automatically. */
  trustedAuthors: string[];
  /** Full origin DeviceIds whose files stay click-only whatever the mode says. */
  blockedAuthors: string[];
};

export type FileTrustPolicies = Record<number, FileTrustPolicy>;

/**
 * Media only: images, audio and video from the group load passively; documents, archives and
 * anything the renderer does not recognise as media wait for a click. The middle setting, and
 * the default, because it is what most people mean by "show me the pictures".
 */
export const DEFAULT_FILE_TRUST_POLICY: FileTrustPolicy = {
  mode: "media",
  trustedAuthors: [],
  blockedAuthors: [],
};

// UI continuity has a 1 MiB native envelope. Keep this relationship-bearing slice comfortably
// below it even when every row uses the maximum full-identity length.
const MAX_SERVERS = 64;
const MAX_AUTHORS = 32;
const MAX_IDENTITY_CHARS = 128;

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function identities(value: unknown): string[] {
  return Array.isArray(value)
    ? [...new Set(value.filter((author): author is string =>
      typeof author === "string" && author.length > 0 && author.length <= MAX_IDENTITY_CHARS
    ))].slice(0, MAX_AUTHORS)
    : [];
}

/**
 * Bound and validate policy state before it can control automatic untrusted-media decoding.
 *
 * A stored mode this build does not know fails closed to on-demand. The retired `specific`
 * mode reads as on-demand with its trusted list kept: under the override rules that is exactly
 * the behaviour it had, so nobody's choice changes on upgrade.
 */
export function sanitizeFileTrustPolicies(value: unknown): FileTrustPolicies {
  const policies: FileTrustPolicies = {};
  for (const [key, raw] of Object.entries(record(value)).slice(0, MAX_SERVERS)) {
    const server = Number(key);
    if (!Number.isSafeInteger(server) || server < 0 || String(server) !== key) continue;
    const item = record(raw);
    const mode: FileTrustMode = item.mode === "everyone" || item.mode === "media"
      ? item.mode
      : "on-demand";
    const trustedAuthors = identities(item.trustedAuthors);
    const blocked = new Set(identities(item.blockedAuthors));
    // One person cannot be both; the block wins, because it is the one that fails closed.
    policies[server] = {
      mode,
      trustedAuthors: trustedAuthors.filter((author) => !blocked.has(author)),
      blockedAuthors: [...blocked],
    };
  }
  return policies;
}

export function fileTrustPolicyFor(
  policies: FileTrustPolicies,
  server: number,
): FileTrustPolicy {
  return policies[server] ?? DEFAULT_FILE_TRUST_POLICY;
}

/** The override recorded for one full identity, if any. */
export function authorOverride(policy: FileTrustPolicy, identity: string): FileAuthorOverride {
  if (policy.blockedAuthors.includes(identity)) return "never";
  if (policy.trustedAuthors.includes(identity)) return "always";
  return "follow";
}

/**
 * Whether a passive UI surface may fetch/decode this member's file without a user gesture.
 * Explicit Download/Play/Open actions deliberately bypass this helper.
 *
 * `isMedia` is whether the file is something the renderer treats as media (an image, audio or
 * video it would decode inline); it is what the `media` mode turns on. A `never` override holds
 * even for an unverified claim of authorship, because failing closed on a claimed name costs
 * nothing; an `always` override needs the authorship attested, because it grants something.
 */
export function mayAutoLoadFile(
  policy: FileTrustPolicy,
  author: string,
  authorVerified: boolean,
  isMedia = true,
): boolean {
  if (policy.blockedAuthors.includes(author)) return false;
  if (authorVerified && policy.trustedAuthors.includes(author)) return true;
  if (policy.mode === "everyone") return true;
  return policy.mode === "media" && isMedia;
}

/**
 * Remote URLs have no Mewtual file-origin attestation. A message's display author is not an
 * authentication boundary, so a per-person override cannot safely turn an arbitrary URL into a
 * passive network request. Only the explicit whole-server mode permits that behaviour.
 */
export function mayAutoLoadRemoteUrl(_policy: FileTrustPolicy): boolean {
  // A third-party URL has no group/file attestation and may target localhost or a private LAN.
  // Keep it click-only in every mode until a public-address/DNS-rebinding-safe fetcher exists.
  return false;
}

/** The jukebox follows remote transport, but local explicit consent still overrides its policy. */
export function mayLoadJukeboxFile(
  policy: FileTrustPolicy,
  author: string,
  authorVerified: boolean,
  explicitlyApproved: boolean,
): boolean {
  // A jukebox entry is audio or video by construction, so it counts as media here.
  return explicitlyApproved || mayAutoLoadFile(policy, author, authorVerified, true);
}

/** Media URLs are capabilities for one server, even when another server references the same CID. */
export function scopedMediaKey(server: number, cid: string): string {
  return `${server}:${cid}`;
}

/**
 * Record one authenticated full roster identity's override without letting UI state grow
 * without bound. `follow` removes the person from both lists.
 */
export function setAuthorOverride(
  policy: FileTrustPolicy,
  identity: string,
  override: FileAuthorOverride,
): FileTrustPolicy {
  const trustedAuthors = policy.trustedAuthors.filter((author) => author !== identity);
  const blockedAuthors = policy.blockedAuthors.filter((author) => author !== identity);
  const cleared = { ...policy, trustedAuthors, blockedAuthors };
  if (override === "follow" || !identity || identity.length > MAX_IDENTITY_CHARS) return cleared;
  if (override === "always") {
    return trustedAuthors.length >= MAX_AUTHORS ? cleared : { ...cleared, trustedAuthors: [...trustedAuthors, identity] };
  }
  return blockedAuthors.length >= MAX_AUTHORS ? cleared : { ...cleared, blockedAuthors: [...blockedAuthors, identity] };
}
