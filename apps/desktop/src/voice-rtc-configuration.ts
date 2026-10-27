/** Private configuration custody for one native App call. Device settings are local
 * input, never authenticated endpoint/relay policy: native14 binds PC/Ready/profile,
 * not ICE URLs. In particular callers must not feed invite/server TURN hints here.
 *
 * The broker prepays the separate fee BEFORE construction and binds that SAME fee to
 * the accepted native group before capture/key work. App remains inactive. Logical
 * JS copies/frames are bounded here; opaque browser RTC/GC heaps are not measured. */
export const VOICE_RTC_CONFIGURATION_BYTES = 64 * 1024;
export const VOICE_RTC_CONFIGURATION_UNITS = 4096;
const MAX_URLS = 8, MAX_URL_UNITS = 1024, MAX_CREDENTIAL_UNITS = 1024;
// Sixteen indexed media records plus the existing in-progress negotiation. This is
// metadata headroom, not permission for17 peers or another execution/SDK position.
const MAX_PEERS = 17;
export type VoiceRtcDeviceSettings = Readonly<{
  stun: string; turn: string; username: string; credential: string; policy: "all" | "relay";
}>;
/** Synchronous, local-device-only source. Its returned graph is never retained. */
export type VoiceRtcDeviceSettingsSource = () => unknown;
declare const leaseBrand: unique symbol;
export type VoiceRtcConfigurationLease = Readonly<{ [leaseBrand]: true }>;
type Server = Readonly<{ urls: string; username?: string; credential?: string }>;
type Snapshot = Readonly<{ servers: readonly Server[]; policy: "all" | "relay" }>;
type Lease = { owner: VoiceRtcConfigurationOwner; snapshot: Snapshot };
const leases = new WeakMap<VoiceRtcConfigurationLease, Lease>();
const unavailable = (): never => { throw new Error("voice RTC configuration unavailable"); };

function hostPort(value: string): boolean {
  let host: string, port: string | undefined;
  if (value.startsWith("[")) {
    const match = /^(\[[0-9a-fA-F:.]+\])(?::([0-9]+))?$/.exec(value);
    if (!match) return false;
    [, host, port] = match;
    // Validate ONLY a bracketed IP literal, not the non-hierarchical STUN/TURN URI.
    // RFC7064/7065 forbid parsing the whole URI with a generic hierarchical parser.
    try { if (!new URL("http://" + host).hostname.startsWith("[")) return false; }
    catch { return false; }
  } else {
    const match = /^([^:]+)(?::([0-9]+))?$/.exec(value);
    if (!match) return false;
    [, host, port] = match;
    const name = host.endsWith(".") ? host.slice(0, -1) : host;
    if (name.length > 253 || !name.split(".").every(label =>
      label.length >= 1 && label.length <= 63 && /^[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?$/.test(label))) return false;
    if (/^[0-9.]+$/.test(host) && (!/^\d+\.\d+\.\d+\.\d+$/.test(host) ||
        !host.split(".").every(part => Number(part) <= 255))) return false;
  }
  return port === undefined || (port.length <= 5 && Number(port) >= 1 && Number(port) <= 65535);
}
function uri(value: string, turn: boolean): string {
  if (value.length > MAX_URL_UNITS) return unavailable();
  const match = /^(stun|stuns|turn|turns):([^?]+)(?:\?transport=(udp|tcp))?$/i.exec(value);
  if (!match) return unavailable();
  const scheme = match[1].toLowerCase(), transport = match[3]?.toLowerCase();
  if (scheme.startsWith("turn") !== turn || (!turn && transport !== undefined) ||
      (scheme === "turns" && transport === "udp") || !hostPort(match[2])) return unavailable();
  return scheme + ":" + match[2] + (transport ? "?transport=" + transport : "");
}
function snapshot(source: VoiceRtcDeviceSettingsSource, current: () => boolean): Snapshot {
  if (!current()) return unavailable();
  const input = source();
  if (!input || typeof input !== "object") return unavailable();
  // Read ONLY these bounded fields once. Enumerating a caller's unknown keys would
  // allocate an unbounded key list; no other input property is copied or retained.
  const value = input as VoiceRtcDeviceSettings;
  const stun = value.stun, turn = value.turn, username = value.username,
    credential = value.credential, policy = value.policy;
  if (!current() || (policy !== "all" && policy !== "relay") ||
      typeof stun !== "string" || typeof turn !== "string" ||
      typeof username !== "string" || typeof credential !== "string" ||
      stun.length + turn.length + username.length + credential.length > VOICE_RTC_CONFIGURATION_UNITS ||
      username.length > MAX_CREDENTIAL_UNITS || credential.length > MAX_CREDENTIAL_UNITS) return unavailable();
  // Bounds precede trimming/splitting/regular-expression copies. The callback/input
  // lexical frame ends synchronously, before any key/SDP/SDK promise is started.
  const servers: Server[] = [];
  let urls = 0, turns = 0;
  // Iterate at most nine actual tokens, instead of first allocating a large split
  // array for a separator-heavy but bounded setting. Leading/repeated separators
  // are harmless local input and do not create empty server entries.
  function next(text: string, relay: boolean): void {
    for (const token of text.matchAll(/[^\s,]+/g)) {
      if (++urls > MAX_URLS) return unavailable();
      const parsed = uri(token[0], relay);
      if (relay) {
        if (!username || !credential || /[\u0000-\u001f\u007f]/.test(username) ||
            /[\u0000-\u001f\u007f]/.test(credential)) return unavailable();
        turns++;
        servers.push(Object.freeze({ urls: parsed, username, credential }));
      } else if (policy === "all") servers.push(Object.freeze({ urls: parsed }));
    }
  }
  // Relay-only omits STUN entirely; invalid relay settings never fall back to direct.
  next(stun, false); next(turn, true);
  if (policy === "relay" && turns === 0) return unavailable();
  if (!current()) return unavailable();
  return Object.freeze({ servers: Object.freeze(servers), policy });
}

/** Separate prepaid window owner. It holds no App Call/native owner/N/Phase callback;
 * the broker's actual-owner WeakMap alone decides which channel may borrow from it.
 * Snapshot/lease storage is private; credentials never enter diagnostics or handles. */
export class VoiceRtcConfigurationOwner {
  #snapshot: Snapshot | null = null;
  #capturing = false;
  #closing = false;
  readonly #leases = new Set<VoiceRtcConfigurationLease>();
  #waiting: { promise: Promise<void>; resolve(): void } | null = null;
  #joined: Promise<void> | null = null;
  #joining = false;
  capture(source: VoiceRtcDeviceSettingsSource, current: () => boolean): void {
    if (this.#snapshot || this.#capturing || this.#closing || this.#leases.size) return unavailable();
    this.#capturing = true;
    try {
      const observed = snapshot(source, () => {
        const live = !this.#closing && current();
        return live && !this.#closing && this.#capturing;
      });
      if (this.#closing) return unavailable();
      this.#snapshot = observed;
    } finally { this.#capturing = false; this.#notify(); }
  }
  borrow(): VoiceRtcConfigurationLease {
    if (!this.#snapshot || this.#capturing || this.#closing || this.#leases.size >= MAX_PEERS) return unavailable();
    const lease = Object.freeze({}) as VoiceRtcConfigurationLease;
    leases.set(lease, { owner: this, snapshot: this.#snapshot });
    this.#leases.add(lease);
    return lease;
  }
  /** Negative-only. Busy refusal retains the original payment and snapshot until
   * every actual original SDK/peer returns; it never enables another capture/epoch. */
  reset(): void {
    this.#closing = true;
    if (this.#capturing || this.#leases.size || this.#joining) return unavailable();
    this.#snapshot = null;
    this.#waiting = null; this.#joined = null;
    this.#closing = false;
  }
  /** Publish negative state synchronously, before native/SDK cleanup callbacks. */
  close(): void { this.#closing = true; }
  /** Coalesced actual configuration-owner join, outside the shared physical codec.
   * No caller/native/N lexical scope or cancelled-promise approximation is retained.
   * A held original SDK loan keeps this exact join and its group payment pending. */
  join(): Promise<void> {
    if (this.#joined) return this.#joined;
    this.close(); this.#joining = true;
    this.#joined = Promise.resolve().then(async () => {
      if (this.#capturing || this.#leases.size) {
        if (!this.#waiting) {
          let resolve!: () => void;
          const promise = new Promise<void>(yes => { resolve = yes; });
          this.#waiting = { promise, resolve };
        }
        await this.#waiting.promise;
      }
    }).finally(() => { this.#joining = false; });
    return this.#joined;
  }
  #notify(): void { if (!this.#capturing && this.#leases.size === 0) this.#waiting?.resolve(); }
  /** Pure module-private provenance checks; equal settings/labels cannot adopt a lease. */
  owns(lease: VoiceRtcConfigurationLease, observed: Snapshot): boolean {
    return !this.#closing && this.#snapshot === observed && this.#leases.has(lease);
  }
  returned(lease: VoiceRtcConfigurationLease): void { this.#leases.delete(lease); this.#notify(); }
}

export function voiceRtcConfigurationCurrent(lease: VoiceRtcConfigurationLease): boolean {
  const state = leases.get(lease);
  return !!state && state.owner.owns(lease, state.snapshot);
}
/** SDK-only synchronous constructor tail, entered AFTER original native14 authority.
 * No callback/input/temporary configuration graph escapes into an SDP await. Actual
 * peer publication, reentrant End and cleanup remain the original RTC owner's job. */
export function voiceRtcConfiguredPeer(lease: VoiceRtcConfigurationLease,
  Constructor: typeof RTCPeerConnection, current: () => boolean): RTCPeerConnection {
  const state = leases.get(lease);
  if (!state || !state.owner.owns(lease, state.snapshot)) return unavailable();
  const configuration: RTCConfiguration = { bundlePolicy: "max-bundle", rtcpMuxPolicy: "require",
    iceCandidatePoolSize: 0, iceTransportPolicy: state.snapshot.policy,
    iceServers: state.snapshot.servers.map(server => ({ ...server })) };
  if (!current() || !state.owner.owns(lease, state.snapshot)) return unavailable();
  return new Constructor(configuration);
}
/** Return only after actual SDK/peer/callback joins. This is not current authority. */
export function releaseVoiceRtcConfiguration(lease: VoiceRtcConfigurationLease): void {
  const state = leases.get(lease);
  if (!state) return;
  leases.delete(lease);
  state.owner.returned(lease);
}

/** Conservative owned-JS inventory, excluding framework/provider and opaque C++ heaps.
 * Primitive strings may share storage; still charge two full constructor payload copies.
 * Capture happens before peers exist, so its temporary strings do not coexist with17 PCs.
 * A future bound increase MUST update this inventory and its tests before allocation. */
export function voiceRtcConfigurationInventory(): number {
  const capturePeak = 4 * 2 * VOICE_RTC_CONFIGURATION_UNITS + 4096;
  const peersPeak = 3 * 2 * VOICE_RTC_CONFIGURATION_UNITS + MAX_PEERS * (MAX_URLS * 128 + 256) + 4096;
  return Math.max(capturePeak, peersPeak);
}
