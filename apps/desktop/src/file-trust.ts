/** Local, vault-sealed policy for content a server may fetch/render without a click. */
export type FileTrustMode = "on-demand" | "specific" | "everyone";

export type FileTrustPolicy = {
  mode: FileTrustMode;
  /** Full origin DeviceIds whose attested files may be fetched/rendered automatically. */
  trustedAuthors: string[];
};

export type FileTrustPolicies = Record<number, FileTrustPolicy>;

export const DEFAULT_FILE_TRUST_POLICY: FileTrustPolicy = {
  mode: "on-demand",
  trustedAuthors: [],
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

/** Bound and validate policy state before it can control automatic untrusted-media decoding. */
export function sanitizeFileTrustPolicies(value: unknown): FileTrustPolicies {
  const policies: FileTrustPolicies = {};
  for (const [key, raw] of Object.entries(record(value)).slice(0, MAX_SERVERS)) {
    const server = Number(key);
    if (!Number.isSafeInteger(server) || server < 0 || String(server) !== key) continue;
    const item = record(raw);
    const mode: FileTrustMode = item.mode === "everyone" || item.mode === "specific"
      ? item.mode
      : "on-demand";
    const trustedAuthors = Array.isArray(item.trustedAuthors)
      ? [...new Set(item.trustedAuthors.filter((author): author is string =>
        typeof author === "string" && author.length > 0 && author.length <= MAX_IDENTITY_CHARS
      ))].slice(0, MAX_AUTHORS)
      : [];
    policies[server] = { mode, trustedAuthors };
  }
  return policies;
}

export function fileTrustPolicyFor(
  policies: FileTrustPolicies,
  server: number,
): FileTrustPolicy {
  return policies[server] ?? DEFAULT_FILE_TRUST_POLICY;
}

/**
 * Whether a passive UI surface may fetch/decode this member's file without a user gesture.
 * Explicit Download/Play/Open actions deliberately bypass this helper.
 */
export function mayAutoLoadFile(
  policy: FileTrustPolicy,
  author: string,
  authorVerified: boolean,
): boolean {
  return policy.mode === "everyone" ||
    (authorVerified && policy.mode === "specific" && policy.trustedAuthors.includes(author));
}

/**
 * Remote URLs have no Mewtual file-origin attestation. A message's display author is not an
 * authentication boundary, so "specific people" cannot safely turn an arbitrary URL into a
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
  return explicitlyApproved || mayAutoLoadFile(policy, author, authorVerified);
}

/** Media URLs are capabilities for one server, even when another server references the same CID. */
export function scopedMediaKey(server: number, cid: string): string {
  return `${server}:${cid}`;
}

/** Toggle one authenticated full roster identity without letting UI state grow without bound. */
export function toggleTrustedAuthor(policy: FileTrustPolicy, identity: string): FileTrustPolicy {
  const trustedAuthors = policy.trustedAuthors.filter((author) => author !== identity);
  if (trustedAuthors.length !== policy.trustedAuthors.length) {
    return { mode: "specific", trustedAuthors };
  }
  if (!identity || identity.length > MAX_IDENTITY_CHARS || trustedAuthors.length >= MAX_AUTHORS) {
    return { mode: "specific", trustedAuthors };
  }
  return { mode: "specific", trustedAuthors: [...trustedAuthors, identity] };
}
