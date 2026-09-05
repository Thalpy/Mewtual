import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { InvokeArgs } from "@tauri-apps/api/core";
import { pageOfList, type PageRequest, type UnreadProbe } from "./message-paging.ts";

export const VISUAL_FIXTURE_NOW = Date.UTC(2026, 7, 20, 12, 0, 0);

const ME = "a4f29c110b7d8365a4f29c110b7d8365";
const JUNIPER = "62e80f475ac4931162e80f475ac49311";
const MOSS = "9b31d5a277c04efa9b31d5a277c04efa";

type Message = {
  id: string;
  author: string;
  text: string;
  ts: number;
  edited: number;
  reactions: Array<{ emoji: string; by: string[] }>;
  reply_to: string;
  pinned: boolean;
};

const message = (
  id: string,
  author: string,
  text: string,
  minutesAgo: number,
  extra: Partial<Message> = {},
): Message => ({
  id,
  author,
  text,
  ts: VISUAL_FIXTURE_NOW - minutesAgo * 60_000,
  edited: 0,
  reactions: [],
  reply_to: "",
  pinned: false,
  ...extra,
});

const CHANNEL_MESSAGES: Record<string, Message[]> = {
  general: [
    message("msg-1", JUNIPER, "Morning! The visual review build is ready.", 74),
    message("msg-2", ME, "Nice. I tightened the spacing around the channel header.", 69, {
      reactions: [{ emoji: "👍", by: [JUNIPER, MOSS] }],
    }),
    message("msg-3", MOSS, "The new layout holds up at 900 × 640 too.", 42),
    message("msg-4", JUNIPER, "@[Rowan] could you check the unread divider and composer next?", 18, {
      reply_to: "msg-2",
      pinned: true,
    }),
    message("msg-5", ME, "On it — this screenshot is coming from deterministic fixture data.", 7),
  ],
  design: [
    message("design-1", MOSS, "I left two options in the mockups: quiet borders and soft glow.", 55),
    message("design-2", JUNIPER, "Quiet borders feel more like the rest of Mewtual.", 48),
  ],
  notes: [message("notes-1", ME, "Remember to verify the compact window before handoff.", 31)],
  dm: [
    message("dm-1", JUNIPER, "The fixture can cover DMs as well as server channels.", 26),
    message("dm-2", ME, "Perfect — one stable URL per state is the goal.", 22),
  ],
};

const PROFILES = [
  {
    fingerprint: ME,
    name: "Rowan",
    color: "#8d7cf5",
    font: "rounded",
    effect: "",
    description: "Building small, understandable tools for private communities.",
    bubble: "",
    avatar: "",
    banner: "",
  },
  {
    fingerprint: JUNIPER,
    name: "Juniper",
    color: "#5fc7a1",
    font: "",
    effect: "",
    description: "Design systems, field notes, and very strong tea.",
    bubble: "",
    avatar: "",
    banner: "",
  },
  {
    fingerprint: MOSS,
    name: "Moss",
    color: "#e6a85c",
    font: "mono",
    effect: "",
    description: "Keeps the release checklist honest.",
    bubble: "",
    avatar: "",
    banner: "",
  },
];

const clone = <T>(value: T): T => structuredClone(value);

/**
 * One canonical diagnostic event, with the parts a fixture rarely sets defaulted.
 *
 * The console reads the whole canonical shape now: the section it belongs to, the phase it was in,
 * its trace, its references and the capture mode it was rendered at. Writing eighteen fields out
 * seven times would bury the two incidents this fixture exists to re-enact, so the defaults sit
 * here and each event states only what makes it itself.
 */
const dbgEvent = (e: {
  seq: number;
  at_ms: number;
  section: string;
  view: string;
  level: string;
  code: string;
  target: string;
  phase?: string;
  operation?: string;
  trace?: string;
  duration_ms?: number | null;
  refs?: [string, string][];
  fields?: { name: string; value: string; sensitive?: boolean }[];
}) => ({
  seq: e.seq,
  at_ms: e.at_ms,
  monotonic_ms: 0,
  section: e.section,
  view: e.view,
  level: e.level,
  code: e.code,
  phase: e.phase ?? "observation",
  operation: e.operation ?? "",
  trace: e.trace ?? "",
  span: "",
  parent_span: "",
  refs: e.refs ?? [],
  duration_ms: e.duration_ms ?? null,
  attempt: null,
  target: e.target,
  fields: (e.fields ?? []).map((f) => ({
    name: f.name,
    value: f.value,
    kind: "bridged",
    sensitive: f.sensitive ?? false,
  })),
  fields_dropped: 0,
  capture: "enhanced",
  capture_epoch: 1,
});

/** The code an un-migrated `tracing` event carries. Matches `BRIDGED_CODE` in `debug-console.ts`. */
const BRIDGED = "LOG.TRACING.EVENT";

/**
 * Return deterministic native-command data for the browser-rendered visual fixture.
 *
 * Keeping this as a pure function makes the contract unit-testable and makes an unsupported IPC
 * call fail loudly. That is preferable to a screenshot which looks plausible while silently
 * omitting a newly-added backend dependency.
 */
export function visualFixtureResponse(command: string, payload: InvokeArgs = {}): unknown {
  const args: Record<string, unknown> =
    Array.isArray(payload) || payload instanceof ArrayBuffer || payload instanceof Uint8Array
      ? {}
      : payload;
  const server = Number(args.server ?? 1);
  const channel = String(args.channel ?? "general");

  switch (command) {
    case "resume_session":
      return clone([
        {
          server: 1,
          name: "Lantern Room",
          invite: "fixture-invite-not-for-pairing",
          channel: "general",
          channels: [
            { id: "general", name: "general" },
            { id: "design", name: "design" },
            { id: "notes", name: "field-notes" },
          ],
          is_dm: false,
        },
        {
          server: 2,
          name: "Juniper",
          invite: "",
          channel: "dm",
          channels: [{ id: "dm", name: "Juniper" }],
          is_dm: true,
        },
      ]);
    case "vault_exists":
      return true;
    case "get_ui_state":
      return JSON.stringify({
        version: 1,
        drafts: { "1:general": "A deterministic composer draft" },
        readMarks: { "1:general": VISUAL_FIXTURE_NOW - 25 * 60_000 },
      });
    case "save_ui_state":
      return null;
    case "get_inbox":
      return clone([
        {
          server: 1,
          server_name: "Lantern Room",
          is_dm: false,
          channel: "general",
          message_id: "msg-4",
          author: JUNIPER,
          author_name: "Juniper",
          text: "@[Rowan] could you check the unread divider and composer next?",
          ts: VISUAL_FIXTURE_NOW - 18 * 60_000,
          mention: true,
          reply: true,
        },
      ]);
    case "get_messages":
      return clone(CHANNEL_MESSAGES[server === 2 ? "dm" : channel] ?? []);
    case "get_message_page": {
      const rows = clone(CHANNEL_MESSAGES[server === 2 ? "dm" : channel] ?? []) as Message[];
      const request: PageRequest = {
        anchor: (args.anchor as PageRequest["anchor"]) ?? { kind: "tail" },
        before: typeof args.before === "number" ? args.before : 0,
        after: typeof args.after === "number" ? args.after : 0,
      };
      const probe = (args.unread as UnreadProbe | undefined) ?? null;
      return pageOfList(rows, request, ME, "@[Rowan]", 1, probe);
    }
    case "get_pinned_messages":
      return clone((CHANNEL_MESSAGES[server === 2 ? "dm" : channel] ?? []).filter((m) => m.pinned));
    case "get_messages_by_id": {
      // The arrival read: the named rows wherever they sort, each with its addressing bit. Ids
      // that name nothing are absent, exactly as the native command answers.
      const all = clone(CHANNEL_MESSAGES[server === 2 ? "dm" : channel] ?? []) as Array<{
        id: string;
        text: string;
      }>;
      const wanted = Array.isArray(args.ids) ? (args.ids as string[]) : [];
      return all
        .filter((row) => wanted.includes(row.id))
        .map((row) => ({ ...row, targets_me: row.text.includes("@[Rowan]") }));
    }
    case "get_message_tail": {
      const all = clone(CHANNEL_MESSAGES[server === 2 ? "dm" : channel] ?? []) as Array<{ id: string; text: string }>;
      const limit = typeof args.limit === "number" && args.limit > 0 ? Math.trunc(args.limit) : all.length;
      const cursor = typeof args.afterId === "string" ? args.afterId : "";
      const at = cursor ? all.findIndex((row) => row.id === cursor) : -1;
      const addresses = (row: { text: string }) => row.text.includes("@[Rowan]");
      return {
        rows: all.slice(-limit).map((row) => ({ ...row, targets_me: addresses(row) })),
        addressed_after_cursor: all.slice(at + 1).some(addresses),
      };
    }
    case "get_members":
      return clone(
        server === 2
          ? [
              { fingerprint: ME, identity: `${ME}-full-device-id`, you: true },
              { fingerprint: JUNIPER, identity: `${JUNIPER}-full-device-id`, you: false },
            ]
          : [
              { fingerprint: ME, identity: `${ME}-full-device-id`, you: true },
              { fingerprint: JUNIPER, identity: `${JUNIPER}-full-device-id`, you: false },
              { fingerprint: MOSS, identity: `${MOSS}-full-device-id`, you: false },
            ],
      );
    case "get_online_members":
      return server === 2 ? [JUNIPER] : [JUNIPER];
    case "get_profiles":
      return clone(server === 2 ? PROFILES.slice(0, 2) : PROFILES);
    case "get_files":
      return clone({
        has_peers: true,
        files: [
          {
            name: "visual-review-notes.md",
            size: 4182,
            mime: "text/markdown",
            cid: "55aabbeeff0011223344556677889900",
            author: JUNIPER,
            author_verified: true,
            author_identity: `${JUNIPER}-full-device-id`,
            path: "reviews/visual-review-notes.md",
            held: 1,
            total: 1,
            expires: null,
            expires_known: true,
          },
        ],
      });
    case "get_wiki_pinned_cids":
      return [];
    case "get_statuses":
      return clone([
        message("status-1", JUNIPER, "Visual review at 14:00 — bring compact-window notes.", 90),
      ]);
    case "get_invite":
      return server === 1 ? "fixture-invite-not-for-pairing" : null;
    case "get_roles":
      return server === 1 ? { [ME]: "owner", [JUNIPER]: "admin", [MOSS]: "member" } : {};
    case "get_livery":
      return { preset: "", accent: "", tokens: {}, icon: "", cursor: "" };
    case "get_channel_topic":
      return channel === "general" ? "A calm place to build and review Mewtual together" : "";
    case "get_delivery":
      return clone({
        revision: 1,
        states: [
          { id: "msg-2", delivered: 2, reachable: 2, any_peer: true },
          { id: "msg-5", delivered: 1, reachable: 1, any_peer: true },
        ],
      });
    case "get_badges":
      return server === 1 ? { [MOSS]: { label: "release", color: "#e6a85c" } } : {};
    case "get_events":
      return clone([
        {
          id: "event-1",
          title: "Compact-window visual review",
          body: "Walk through chat, settings, files, and the server space at the native window size.",
          start_ts: VISUAL_FIXTURE_NOW + 2 * 60 * 60_000,
          end_ts: VISUAL_FIXTURE_NOW + 3 * 60 * 60_000,
          author: ME,
          image: "",
        },
      ]);
    // The empty Wiki state is enough to exercise its real navigation and lazy-loaded Help view.
    // Keep every read used by `refreshWiki` explicit so a future dependency still fails loudly.
    case "get_wiki_pages":
      return [];
    case "get_wiki_map":
    case "get_wiki_meta":
      return {};
    case "get_wiki_review_days":
      return 0;
    case "get_wiki_pending":
      return [];
    case "get_devices":
      return {};
    case "get_dm_requests":
      return [];
    case "get_moderation":
      return { events: [], votes: [] };
    case "get_connectivity":
      return clone({
        action: "found",
        subject: "Lantern Room",
        // The join key between this panel and the debug console's record of the same attempt.
        trace: "7f2c",
        at: VISUAL_FIXTURE_NOW - 3 * 60_000,
        server: 1,
        advertised: [
          "/ip4/192.168.1.42/tcp/22487/p2p/12D3KooWFixture",
          "/ip4/192.168.1.42/udp/22487/quic-v1/p2p/12D3KooWFixture",
        ],
        public_direct: false,
        upnp:
          "no mapping obtained within 25s (UPnP unavailable; PCP unavailable; NAT-PMP unavailable)",
        autonat:
          "not tested: no public address candidate and AutoNAT server were available together",
        steps: [
          {
            at: VISUAL_FIXTURE_NOW - 3 * 60_000,
            kind: "listen",
            target: "port 22487",
            detail: "bound IPv4 + IPv6 over TCP + QUIC; 2 addresses auto-detected",
            status: "ok",
          },
          {
            at: VISUAL_FIXTURE_NOW - 3 * 60_000,
            kind: "invite",
            target: "",
            detail: "invite minted carrying 2 addresses and 0 rendezvous entries",
            status: "ok",
          },
        ],
        last_error: "",
      });
    // The debug console's sources. The data deliberately re-enacts the two incidents the console
    // was built for, so the fixture shows it doing its job rather than showing an empty shell:
    // Moss advertises only IPv6 while this device has no observed public IPv6 candidate, and the voice signalling run
    // fails against a peer whose transport connection has gone.
    case "save_diagnostics_report":
      return clone({
        path: "C:\\fixture\\logs\\mewtual-diagnostics-eb887278-1787000000000.txt",
        file: "mewtual-diagnostics-eb887278-1787000000000.txt",
        bytes: 4096,
      });
    case "get_console_log":
      return clone({
        events: [
          // Un-migrated call sites: prose under the bridge's code, which is what most of the record
          // still looks like and therefore what the console has to stay readable against.
          dbgEvent({
            seq: 1,
            at_ms: VISUAL_FIXTURE_NOW - 182_000,
            section: "transport",
            view: "network",
            level: "INFO",
            code: BRIDGED,
            target: "catcoms_net",
            fields: [
              { name: "message", value: "listening address" },
              { name: "address", value: "/ip4/192.168.1.42/udp/22487/quic-v1", sensitive: true },
            ],
          }),
          dbgEvent({
            seq: 2,
            at_ms: VISUAL_FIXTURE_NOW - 121_000,
            section: "transport",
            view: "network",
            level: "WARN",
            code: BRIDGED,
            target: "catcoms_net",
            fields: [
              { name: "message", value: "dial failed" },
              {
                name: "addr",
                value: "/ip6/2601:441:4581:a5c0:b81d:9e0b:cab1:de04/udp/23123/quic-v1",
                sensitive: true,
              },
              { name: "error", value: "network unreachable" },
            ],
          }),
          dbgEvent({
            seq: 3,
            at_ms: VISUAL_FIXTURE_NOW - 96_000,
            section: "discovery",
            view: "network",
            level: "WARN",
            code: BRIDGED,
            target: "catcoms_discovery::eclipse",
            fields: [
              { name: "message", value: "eclipse detector raised CAUTION (sustained isolation signs)" },
            ],
          }),
          // A migrated call site: a stable code, a phase, a trace and typed fields. It shows in the
          // voice section because it says it is a voice event, not because its text says "voice".
          dbgEvent({
            seq: 4,
            at_ms: VISUAL_FIXTURE_NOW - 74_000,
            section: "voice",
            view: "voice",
            level: "WARN",
            code: "VOICE.PORT.MAP_REFUSED",
            target: "catcoms_ui",
            phase: "failure",
            operation: "start_call",
            trace: "7f2c000000000031",
            duration_ms: 2140,
            fields: [{ name: "mechanism", value: "upnp" }],
          }),
          dbgEvent({
            seq: 5,
            at_ms: VISUAL_FIXTURE_NOW - 61_000,
            section: "voice",
            view: "voice",
            level: "WARN",
            code: "VOICE.SIGNAL.NO_MEMBER_ROUTE",
            target: "catcoms_ui",
            phase: "failure",
            operation: "send_call_signal",
            trace: "7f2c000000000031",
            refs: [["peer", "peer-2b5df389"]],
            fields: [{ name: "signal", value: "ice" }],
          }),
          dbgEvent({
            seq: 6,
            at_ms: VISUAL_FIXTURE_NOW - 51_000,
            section: "transport",
            view: "network",
            level: "WARN",
            code: BRIDGED,
            target: "catcoms_net",
            fields: [
              { name: "message", value: "outbound request failed" },
              { name: "peer", value: "12D3KooWFixtureMoss" },
              { name: "error", value: "dial failure" },
            ],
          }),
          dbgEvent({
            seq: 7,
            at_ms: VISUAL_FIXTURE_NOW - 30_000,
            section: "sync",
            view: "backend",
            level: "DEBUG",
            code: BRIDGED,
            target: "catcoms_sync",
            fields: [
              { name: "message", value: "serving PEX" },
              { name: "count", value: "2" },
            ],
          }),
        ],
        errors: 0,
        warnings: 4,
        dropped: 0,
        filtered: 118,
        latest_seq: 7,
        capacity: 4096,
        capture: "enhanced",
        session_id: "eb887278",
      });
    // A fixed Enhanced snapshot. The mode buttons therefore re-render the same canned page rather
    // than changing what it shows, because this function is deterministic on purpose: the same
    // command and arguments must give the same answer, or a screenshot stops being reproducible.
    // What Safe and Enhanced actually do to a value is pinned by unit tests in `render.rs`, the
    // desktop bridge and `debug-console.test.ts`, which is the right place for a property that is
    // about rendering rather than about layout.
    // One dead event forwarder, because the fixture exists to show the console doing its job and
    // this is the failure the rest of the console cannot show: everything else keeps reporting
    // normally while the thing that was meant to be doing the work is gone.
    case "get_task_health":
      return clone([
        {
          id: 1,
          kind: "server_actor",
          server: 1,
          started_ms: VISUAL_FIXTURE_NOW - 600_000,
          last_beat_ms: null,
          state: "running",
          fault: false,
          cause: null,
        },
        {
          id: 2,
          kind: "event_forwarder",
          server: 1,
          started_ms: VISUAL_FIXTURE_NOW - 600_000,
          last_beat_ms: null,
          state: "panicked",
          fault: true,
          cause: "index out of bounds: the len is 0 but the index is 0",
        },
        {
          id: 3,
          kind: "discovery_timer",
          server: 1,
          started_ms: VISUAL_FIXTURE_NOW - 600_000,
          last_beat_ms: VISUAL_FIXTURE_NOW - 20_000,
          state: "running",
          fault: false,
          cause: null,
        },
      ]);
    case "get_capture_config":
    case "set_capture_mode":
    case "set_section_capture":
      return clone({
        mode: "enhanced",
        expires_at_restart: false,
        reveals_addresses: true,
        sections: [
          { id: "diag", view: "backend", level: "INFO" },
          { id: "startup", view: "backend", level: "DEBUG" },
          { id: "ui", view: "frontend", level: "DEBUG" },
          { id: "ipc", view: "backend", level: "DEBUG" },
          { id: "runtime", view: "backend", level: "DEBUG" },
          { id: "vault", view: "storage", level: "DEBUG" },
          { id: "storage", view: "storage", level: "DEBUG" },
          { id: "identity", view: "backend", level: "DEBUG" },
          { id: "membership", view: "backend", level: "DEBUG" },
          { id: "transport", view: "network", level: "DEBUG" },
          { id: "reachability", view: "network", level: "DEBUG" },
          { id: "discovery", view: "network", level: "DEBUG" },
          { id: "join", view: "network", level: "DEBUG" },
          { id: "sync", view: "backend", level: "DEBUG" },
          { id: "channels", view: "backend", level: "DEBUG" },
          { id: "documents", view: "backend", level: "DEBUG" },
          { id: "files", view: "storage", level: "DEBUG" },
          { id: "voice", view: "voice", level: "DEBUG" },
          { id: "devices", view: "backend", level: "DEBUG" },
          { id: "updates", view: "backend", level: "DEBUG" },
          { id: "performance", view: "backend", level: "DEBUG" },
          { id: "privacy", view: "backend", level: "DEBUG" },
        ],
      });
    case "get_member_routes":
      return server === 1
        ? clone([
            {
              fingerprint: JUNIPER,
              peer: "7c41a9de",
              addresses: ["/ip4/198.51.100.24/udp/31484/quic-v1"],
              seq: 6,
              connected: true,
              dial_attempts: 0,
              next_dial_in_ms: 0,
              health: "claimed_peer_connected_direct",
              binding: "self_asserted",
              active_paths: [{ family: "ipv4", transport: "quic_v1", direction: "listener" }],
              last_success: {
                path: { family: "ipv4", transport: "quic_v1", direction: "listener" },
                age_ms: 12_000,
              },
              candidate_families: ["ipv4"],
              candidate_transports: ["quic_v1"],
              actions: [],
              indirect_health: "unknown",
              indirect_witnesses: 0,
              indirect_age_ms: null,
              reciprocal_pending: false,
            },
            {
              fingerprint: MOSS,
              peer: "2b5df389",
              addresses: ["/ip6/2601:441:4581:a5c0:b81d:9e0b:cab1:de04/udp/23123/quic-v1"],
              seq: 4,
              connected: false,
              dial_attempts: 8,
              next_dial_in_ms: 812_000,
              health: "claimed_peer_dial_cooling_down",
              binding: "self_asserted",
              active_paths: [],
              last_success: null,
              candidate_families: ["ipv6"],
              candidate_transports: ["quic_v1"],
              actions: [
                { scope: "this_device", kind: "wait_for_automatic_recovery" },
                { scope: "this_device", kind: "probe_through_members" },
                { scope: "this_device", kind: "retry_group_now" },
              ],
              indirect_health: "reachable_via_member",
              indirect_witnesses: 1,
              indirect_age_ms: 8_000,
              reciprocal_pending: true,
            },
          ])
        : [];
    case "manual_fallback_redial":
      return "submitted";
    case "get_call_transport":
      return clone({
        public_direct: false,
        autonat: "not tested: no public address candidate and AutoNAT server were available together",
        public_ipv4: ["213.105.231.38"],
        public_ipv6: [],
        bridges: [],
        relay_likely_required: true,
        router_maps: true,
        advice:
          "This device is behind NAT and no member is offering to host. Calls to peers who are also behind NAT need a relay.",
      });
    case "get_debug_logging":
    case "test_debug_logging":
      // Deliberately the interesting case: the preference is off while the sink from before the
      // toggle is still writing, which is the disagreement the settings page exists to show.
      return clone({
        enabled: false,
        active: true,
        state: "active",
        error: "",
        session: "eb887278",
        dir: "C:\\fixture\\logs",
        file: "debug_log_20260823_120000.txt",
        events_written: 1284,
        bytes_written: 190_432,
        events_dropped: 0,
        events_truncated: 0,
        queue_depth: 0,
        queue_high_water: 12,
        session_quota_bytes: 52_428_800,
      });
    case "get_switchboard_status":
    case "set_switchboard_offered":
      return clone({
        offered: false,
        eligible: true,
        online: [
          { fingerprint: JUNIPER, addresses: 2 },
          { fingerprint: MOSS, addresses: 1 },
        ],
        reason: "This device has an advertised public or relayed candidate route it can offer.",
      });
    case "get_channels":
      return server === 1
        ? clone([
            { id: "general", name: "general" },
            { id: "design", name: "design" },
            { id: "notes", name: "field-notes" },
          ])
        : clone([{ id: "dm", name: "Juniper" }]);
    case "plugin:window|is_maximized":
      return false;
    case "plugin:updater|check":
      return null;
    default:
      throw new Error(`Visual fixture does not implement Tauri command: ${command}`);
  }
}

/** Install a browser-safe Tauri bridge before App.svelte is evaluated. */
export function installVisualFixture(name: string): void {
  if (name !== "chat") {
    throw new Error(`Unknown visual fixture "${name}". Available fixtures: chat`);
  }

  mockWindows("main");
  mockIPC(
    (command, payload) => {
      const response = visualFixtureResponse(command, payload);
      // `get_moderation` is the final awaited load in switchServer. Once it has returned, wait for
      // fonts and the next task so Svelte can flush its pending microtasks. Do not use
      // requestAnimationFrame here: screenshot browsers intentionally run in the background and
      // Chromium may throttle background frames indefinitely.
      if (command === "get_moderation") {
        void document.fonts.ready.then(() => {
          setTimeout(() => {
            document.documentElement.dataset.visualReady = name;
          }, 0);
        });
      }
      return response;
    },
    { shouldMockEvents: true },
  );

  // Freeze application wall-clock reads so relative presence, event grouping, and unread state do
  // not drift between captures. CSS motion is disabled separately so the screenshot never lands
  // on an arbitrary transition frame.
  Date.now = () => VISUAL_FIXTURE_NOW;
  document.documentElement.dataset.visualFixture = name;
  const style = document.createElement("style");
  style.dataset.visualFixture = name;
  style.textContent = `
    *, *::before, *::after {
      animation: none !important;
      caret-color: transparent !important;
      scroll-behavior: auto !important;
      transition: none !important;
    }
  `;
  document.head.append(style);
}
