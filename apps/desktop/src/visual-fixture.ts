import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { InvokeArgs } from "@tauri-apps/api/core";

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
    case "get_members":
      return clone(
        server === 2
          ? [
              { fingerprint: ME, you: true },
              { fingerprint: JUNIPER, you: false },
            ]
          : [
              { fingerprint: ME, you: true },
              { fingerprint: JUNIPER, you: false },
              { fingerprint: MOSS, you: false },
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
      return clone([
        { id: "msg-2", delivered: 2, reachable: 2 },
        { id: "msg-5", delivered: 1, reachable: 1 },
      ]);
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
    // Moss advertises only IPv6 while this device has no IPv6 route, and the voice signalling run
    // fails against a peer whose transport connection has gone.
    case "get_console_log":
      return clone({
        events: [
          {
            seq: 1,
            at_ms: VISUAL_FIXTURE_NOW - 182_000,
            level: "INFO",
            target: "catcoms_net",
            message: "listening address",
            fields: [["address", "/ip4/192.168.1.42/udp/22487/quic-v1"]],
          },
          {
            seq: 2,
            at_ms: VISUAL_FIXTURE_NOW - 121_000,
            level: "WARN",
            target: "catcoms_net",
            message: "dial failed",
            fields: [
              ["addr", "/ip6/2601:441:4581:a5c0:b81d:9e0b:cab1:de04/udp/23123/quic-v1"],
              ["error", "network unreachable"],
            ],
          },
          {
            seq: 3,
            at_ms: VISUAL_FIXTURE_NOW - 96_000,
            level: "WARN",
            target: "catcoms_discovery::eclipse",
            message: "eclipse detector raised CAUTION (sustained isolation signs)",
            fields: [],
          },
          {
            seq: 4,
            at_ms: VISUAL_FIXTURE_NOW - 74_000,
            level: "WARN",
            target: "catcoms_ui",
            message: "voice: router would not map the media port",
            fields: [],
          },
          {
            seq: 5,
            at_ms: VISUAL_FIXTURE_NOW - 61_000,
            level: "WARN",
            target: "catcoms_ui",
            message: 'voice signal failed {"targetFp":"9b31d5a2","type":"ice","error":"dial failure"}',
            fields: [],
          },
          {
            seq: 6,
            at_ms: VISUAL_FIXTURE_NOW - 51_000,
            level: "WARN",
            target: "catcoms_net",
            message: "outbound request failed",
            fields: [["peer", "12D3KooWFixtureMoss"], ["error", "dial failure"]],
          },
          {
            seq: 7,
            at_ms: VISUAL_FIXTURE_NOW - 30_000,
            level: "DEBUG",
            target: "catcoms_sync",
            message: "serving PEX",
            fields: [["count", "2"]],
          },
        ],
        errors: 0,
        warnings: 4,
        dropped: 0,
        latest_seq: 7,
        capacity: 4096,
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
            },
            {
              fingerprint: MOSS,
              peer: "2b5df389",
              addresses: ["/ip6/2601:441:4581:a5c0:b81d:9e0b:cab1:de04/udp/23123/quic-v1"],
              seq: 4,
              connected: false,
              dial_attempts: 8,
              next_dial_in_ms: 812_000,
            },
          ])
        : [];
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
