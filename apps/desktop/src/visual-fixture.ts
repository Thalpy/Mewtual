import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { InvokeArgs } from "@tauri-apps/api/core";
import { pageOfList, type PageRequest, type UnreadProbe } from "./message-paging.ts";
import { visualStudioFixtureResponse } from "./visual-studio-fixture.ts";
import { createVisualFileUploads } from "./visual-file-uploads.ts";
import { VISUAL_SAVED_FILES, visualTransfersResponse } from "./visual-transfers-fixture.ts";

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

/** A channel's messages. The Transfers fixture (`transfers=1`) adds one message sharing a file
 * the device already saved, so its chat card shows the "Saved" mark (PR #56). */
function channelMessages(key: string, transfers: boolean): Message[] {
  const rows = CHANNEL_MESSAGES[key] ?? [];
  if (!transfers || key !== "general") return rows;
  const saved = VISUAL_SAVED_FILES[0].file;
  // A file reference on a line of its own renders as a file card (`[label](file:CID)`).
  return [...rows, message("msg-6", JUNIPER, `Here is the kit again:\n[${saved.name}](file:${saved.cid})`, 3)];
}

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
export function visualFixtureResponse(command: string, payload: InvokeArgs = {}, studio = false, transfers = false): unknown {
  const args: Record<string, unknown> =
    Array.isArray(payload) || payload instanceof ArrayBuffer || payload instanceof Uint8Array
      ? {}
      : payload;
  // `transfers=1`: one private download row in every Transfers state (Transfers UX, PR #51).
  if (transfers) {
    const response = visualTransfersResponse(command, args,
      () => visualFixtureResponse("get_files", payload, studio) as { has_peers: boolean; files: Array<Record<string, unknown>> });
    if (response.handled) return response.value;
  }
  if (studio) {
    const response = visualStudioFixtureResponse(command, args, VISUAL_FIXTURE_NOW);
    if (response.handled) return response.value;
    // Existing chat fixture ids are descriptive strings; native Studio deliberately requires
    // canonical decimal channels. The explicit opt-in remaps the fixture, never its parser.
    if (command === "resume_session") {
      const servers = visualFixtureResponse(command, payload) as Array<{ server: number; channel: string; channels: { id: string; name: string }[] }>;
      for (const server of servers) if (server.server === 1) { server.channel = "1"; server.channels = server.channels.map((channel, index) => ({ ...channel, id: String(index + 1) })); }
      return servers;
    }
    if (command === "get_channels" && args.server === 1) return [{ id: "1", name: "general" }, { id: "2", name: "design" }, { id: "3", name: "field-notes" }];
    // Studio exports bind a full native device identity, not the descriptive stand-ins used
    // by the old chat-only fixture. Install these before App first loads its roster.
    if (command === "get_members") return (visualFixtureResponse(command, payload) as {fingerprint: string; identity: string; you: boolean}[]).map(member => ({...member, identity: member.fingerprint.repeat(2)}));
    if (command === "get_roles") return Object.fromEntries(Object.entries(visualFixtureResponse(command, payload) as Record<string, string>).map(([identity, role]) => [identity.split("-")[0].repeat(2), role]));
    const channel = args.channel === "1" ? "general" : args.channel === "2" ? "design" : args.channel === "3" ? "notes" : args.channel;
    return visualFixtureResponse(command, { ...args, ...(channel === undefined ? {} : { channel }) });
  }
  const server = Number(args.server ?? 1);
  const channel = String(args.channel ?? "general");

  switch (command) {
    // The visual fixture has no router. It still explicitly models controller acquisition so
    // unlocked App lifecycle cannot silently bypass a newly added native command.
    case "prepare_call_mapping_controller":
      return "fixture-call-controller";
    case "prepare_call_mapping_owner":
      return "fixture-call-owner";
    case "activate_call_mapping_owner":
      return String(args.candidate);
    case "end_call_mapping_owner":
    case "unmap_call_port":
      return null;
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
      return clone(channelMessages(server === 2 ? "dm" : channel, transfers));
    case "get_message_page": {
      const rows = clone(channelMessages(server === 2 ? "dm" : channel, transfers)) as Message[];
      const request: PageRequest = {
        anchor: (args.anchor as PageRequest["anchor"]) ?? { kind: "tail" },
        before: typeof args.before === "number" ? args.before : 0,
        after: typeof args.after === "number" ? args.after : 0,
      };
      const probe = (args.unread as UnreadProbe | undefined) ?? null;
      return pageOfList(rows, request, ME, "@[Rowan]", 1, probe);
    }
    case "get_pinned_messages":
      return clone((channelMessages(server === 2 ? "dm" : channel, transfers)).filter((m) => m.pinned));
    case "get_messages_by_id": {
      // The arrival read: the named rows wherever they sort, each with its addressing bit. Ids
      // that name nothing are absent, exactly as the native command answers.
      const all = clone(channelMessages(server === 2 ? "dm" : channel, transfers)) as Array<{
        id: string;
        text: string;
      }>;
      const wanted = Array.isArray(args.ids) ? (args.ids as string[]) : [];
      return all
        .filter((row) => wanted.includes(row.id))
        .map((row) => ({ ...row, targets_me: row.text.includes("@[Rowan]") }));
    }
    case "get_message_tail": {
      const all = clone(channelMessages(server === 2 ? "dm" : channel, transfers)) as Array<{ id: string; text: string }>;
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
    case "get_private_downloads": return [];
    // A Saved row's read-only file check: one of four words, never a path. The fixture lists no
    // saved rows, so this answers "present" if a captured state ever asks.
    case "check_saved_download": return "present";
    // Show in folder: native checks the file and opens the system file manager; nothing returns.
    case "reveal_saved_download": return null;
    // PR #56: the saved-files list. The chat fixture has saved nothing; `transfers=1` lists one
    // entry per row state. Its check answers nothing, and the list actions change nothing.
    case "get_saved_files": return { state: "ok", files: [] };
    case "check_saved_files": return [];
    case "reveal_saved_file":
    case "forget_saved_file":
    case "reset_saved_files":
      return null;
    case "save_group_file": {
      // Mirrors the native contract: the renderer sends a name and an optional `saveAs` flag,
      // never a path or folder. A fixture "Save as" behaves as if the user closed the dialog, so
      // screenshots never imply a real destination was chosen.
      if (["path", "folder", "dir", "destination"].some((key) => key in args)) {
        throw new Error("save_group_file never accepts a path from the renderer");
      }
      if (args.saveAs === true) throw new Error("Save as was cancelled.");
      const name = typeof args.name === "string" ? args.name : "download";
      // A one-click save lands directly in Downloads, which native asserts with `inDownloads`.
      return { path: `C:\\Users\\fixture\\Downloads\\${name}`, displayed: true, inDownloads: true };
    }
    case "get_cache_status":
    case "set_cache_limit": {
      // Lantern Room sits just past the 90% warning so the footer shows its amber bar and the
      // leaving-soon chip; the DM is calm. A limit choice is echoed back, not persisted.
      const GIB = 1024 ** 3;
      const chosen = command === "set_cache_limit" ? (args.bytes as number | null) : 5 * GIB;
      if (server !== 1) {
        return { usage_bytes: Math.round(0.4 * GIB), limit_bytes: 5 * GIB, unlimited: false, pinned_bytes: 0,
          leaving_soon_files: 0, leaving_soon_bytes: 0, nearly_full: false, over_limit: false,
          held_by_pins: false, paused: null };
      }
      const usage = Math.round(4.6 * GIB);
      return {
        usage_bytes: usage, limit_bytes: chosen, unlimited: chosen === null,
        pinned_bytes: Math.round(0.8 * GIB),
        leaving_soon_files: chosen === null ? 0 : 3,
        leaving_soon_bytes: chosen === null ? 0 : Math.round(0.4 * GIB),
        nearly_full: chosen !== null && usage * 100 > chosen * 90,
        over_limit: chosen !== null && usage > chosen,
        held_by_pins: false, paused: null,
      };
    }
    case "get_storage_health":
      return { listed_files: 2, unique_files: 2, referenced_chunks: 2, verified_chunks: 2,
        missing_chunks: 0, not_cached_chunks: 0, unreadable_chunks: 0, invalid_manifests: 0, verified_bytes: 2048,
        has_peers: true, checked_at_ms: VISUAL_FIXTURE_NOW, logical_bytes: 2048,
        local_estimated_bytes: 2048, pinned_files: 0, pinned_logical_bytes: 0,
        pinned_local_estimated_bytes: 0, categories: [], largest_files: [], local_files: [] };
    case "get_kept_files":
      return { supported: true, allocated_bytes: 0, limit_bytes: 1073741824, files: [], transfers: [], error: null };
    case "file_available":
      return true;
    case "get_file_usage":
      return { wiki_pages: [], status_count: 0, chat_count: 0, event_count: 0, pinned: false };
    case "get_wiki_pinned_cids":
      return [];
    case "get_statuses":
      return clone([
        message("status-1", JUNIPER, "Visual review at 14:00 — bring compact-window notes.", 90),
      ]);
    case "get_invite":
      return server === 1 ? "fixture-invite-not-for-pairing" : null;
    case "get_roles":
      return server === 1 ? { [`${ME}-full-device-id`]: "owner", [`${JUNIPER}-full-device-id`]: "admin", [`${MOSS}-full-device-id`]: "member" } : {};
    case "get_livery":
      return { preset: "", accent: "", tokens: {}, icon: "", cursor: "", name: "", banner: "" };
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
          "Voice tries its own direct connection checks. If voice cannot connect, configure TURN in Settings, Calls.",
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
  // The existing DEV+localhost entry point still owns availability and readiness. Nothing in
  // production recognizes this flag, and default chat fixtures remain byte-for-byte unchanged.
  const studio = new URLSearchParams(window.location.search).get("studio") === "1";
  const transfers = new URLSearchParams(window.location.search).get("transfers") === "1";
  const uploads = studio ? createVisualFileUploads(ME.repeat(2)) : null;
  mockIPC(
    (command, payload) => {
      const args = payload as Record<string, unknown> ?? {};
      const fileResponse = uploads?.response(command, args);
      let response = fileResponse?.handled ? fileResponse.value : visualFixtureResponse(command, payload, studio, transfers);
      if (uploads && command === "get_files") {
        const list = response as { files: unknown[]; has_peers: boolean };
        response = { ...list, files: [...list.files, ...uploads.files(Number(args.server))] };
      }
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
