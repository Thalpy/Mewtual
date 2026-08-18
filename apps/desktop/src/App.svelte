<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { onMount, tick, untrack } from "svelte";
  import { renderMessage, renderWiki, parseRedirect, tocDirective } from "./render";
  import { plainSummary } from "./wikitext";
  import { refLabel, fileMarker, statusMarker, wikiMarker, eventMarker, insertInto } from "./refs";
  import { buildWikiTree, visibleRows, ancestorsOf } from "./wikitree";
  import { extractInfobox, infoboxTemplate } from "./infobox";
  import { diffLines, diffStats, type DiffLine } from "./linediff";
  import {
    type Placement, type SpaceState, angularOffsets, applyOffsets, clampPitch, defaultSpace,
    lassoCapture, parseSpace, project, unproject, wrapYaw,
  } from "./space";
  import QRCode from "qrcode";
  import jsQR from "jsqr";
  // The repo-root logo, bundled by Vite as a same-origin asset (the CSP allows img-src 'self').
  import logoUrl from "../../../assets/mewtual-logo.svg";
  // Raw rather than as URLs: inline SVG inherits currentColor, so one drawing themes itself
  // across every preset. These are build-time assets, never anything a peer can reach.
  import earsSvg from "../../../assets/cat/ears.svg?raw";
  import catIdleArt from "../../../assets/cat/mascot-idle.svg?raw";
  import catBlinkArt from "../../../assets/cat/mascot-blink.svg?raw";
  import catSleepArt from "../../../assets/cat/mascot-sleep.svg?raw";
  import catAlertArt from "../../../assets/cat/mascot-alert.svg?raw";
  import catSyncArt from "../../../assets/cat/mascot-sync.svg?raw";
  import { decodeAudio, encodeAudio, MAX_AUDIO_PAYLOAD } from "./audiocode";
  import {
    type MelodyEvent, NOTE_NAMES, noteName, DUR_MAX_MS, DUR_NAMES, durClass, normalizeEvent,
    encodeMelody, melodyBits as bitsOf, chordName, PC_SHARP, TREBLE_LINES, BASS_LINES, yOf,
    STAFF_TOP, STAFF_BOT, HEAD_RX, HEAD_RY, buildSheet, scoreText,
  } from "./melody";
  import {
    SIGIL_VIEW, SIGIL_C, R_INNER, R_OUTER, R_TEXT, R_EMOJI, NODE_R, LATTICE, nodeLabel, hitNode,
    appendHit, classifyGesture, encodeSigil, encodeSigilPath, segmentCount,
    sigilBits as sigilBitsOf, normalizeWord, MAX_SIGIL_EMOJI, SIGIL_COLORS, COLOR_NAMES,
    coloredCount, ringGlyphs, ringPoints, ringPathD,
  } from "./sigil";

  type Reaction = { emoji: string; by: string[] };
  type Msg = { id: string; author: string; text: string; ts: number; edited: number; reactions: Reaction[]; reply_to: string; pinned: boolean };
  type InboxEntry = {
    server: number; server_name: string; is_dm: boolean;
    channel: string; message_id: string; author: string; author_name: string;
    text: string; ts: number; mention: boolean; reply: boolean;
  };
  const QUICK_EMOJI = ["👍", "❤️", "😂", "🎉", "😮", "😢", "🔥", "👀"];
  type Channel = { id: string; name: string };
  type Member = { fingerprint: string; you: boolean };
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string; description: string; bubble: string; avatar: string; banner: string };
  // `expires`: ms-epoch deadline for this listing's CIRCULATION, or null. `expires_known` tells
  // "explicitly kept forever" (known + null) from "recorded before expiry existed" (!known).
  type UiFile = { name: string; size: number; mime: string; cid: string; author: string; path: string; held: number; total: number; expires: number | null; expires_known: boolean };
  // Where a file is referenced across the server (Properties → "Used in"). `pinned` mirrors
  // `wiki_pages.length > 0`: a wiki-embedded file never drops out of circulation.
  type UiFileUsage = { wiki_pages: string[]; status_count: number; chat_count: number; event_count: number; pinned: boolean };
  type Found = { server: number; channel: string; is_dm: boolean };
  type Reloaded = { server: number; name: string; invite: string; channel: string; is_dm: boolean };

  // One server in the rail (each its own encrypted group). Per-server UI state lives here;
  // messages/roster/profiles/files are loaded for the active server on switch + events.
  type ServerState = {
    id: number;
    name: string;
    channels: Channel[];
    active: string; // active channel id
    unread: string[]; // channel ids with unread
    invite: string; // founder's invite ("" for a joiner)
    dot: boolean; // activity while not the active server
    isDm: boolean; // a 1:1 DM (shown behind the DMs circle) rather than a server
  };

  let servers = $state<ServerState[]>([]);
  let activeServerId = $state<number | null>(null);
  // DM-home mode: the rail's DMs circle is active and the sidebar shows the friends/DM list. Kept in
  // sync with the active group's kind by switchServer (a DM ⇒ dmHome, a server ⇒ not).
  let dmHome = $state(false);
  // Inbox mode: the rail's inbox icon is active and the content area shows the cross-server
  // mention/reply inbox instead of a server/DM.
  let inboxView = $state(false);
  let inboxItems = $state<InboxEntry[]>([]);
  let inboxLoading = $state(false);
  // Servers shown on the rail vs DMs shown behind the DMs circle.
  let railServers = $derived(servers.filter((s) => !s.isDm));
  let dmList = $derived(servers.filter((s) => s.isDm));
  // Per-DM activity stats (no message text), keyed by server id: for the friends-list sortings.
  type DmStat = { server: number; count: number; first_ts: number; last_ts: number; active_days: number };
  let dmStats = $state<Record<number, DmStat>>({});
  type DmSort = "recent" | "activity" | "reconnect" | "alpha";
  let dmSort = $state<DmSort>("recent");
  // The DM list sorted by the chosen mode: recent (last message), activity (avg msgs per active
  // day), reconnect (high past volume × a long silence), or alphabetical.
  let sortedDmList = $derived.by(() => {
    const arr = [...dmList];
    if (dmSort === "alpha") return arr.sort((a, b) => a.name.localeCompare(b.name));
    const now = Date.now();
    const key = (id: number): number => {
      const s = dmStats[id];
      if (!s || !s.count) return 0;
      if (dmSort === "recent") return s.last_ts;
      if (dmSort === "activity") return s.active_days ? s.count / s.active_days : 0; // no dated activity ⇒ bottom
      const gapDays = s.last_ts ? (now - s.last_ts) / 86_400_000 : 0; // reconnect
      return s.count * gapDays;
    };
    return arr.sort((a, b) => key(b.id) - key(a.id));
  });
  let showAdd = $state(false); // showing the found/join form to add a server
  let showNewDm = $state(false); // the "New DM" composer (friend name → friend code to share)
  let showAddFriend = $state(false); // the "Add friend" composer (paste a friend code)
  let dmName = $state(""); // the friend's name for a new/accepted DM
  let dmInvite = $state(""); // a pasted friend code (Add friend)
  // Incoming DM (friend) requests delivered in-band over a shared server, aggregated across servers.
  type DmRequest = { server: number; from_fp: string; from_name: string; invite: string };
  let dmRequests = $state<DmRequest[]>([]);
  let notice = $state(""); // a transient confirmation (e.g. "Friend request sent")
  let showSettings = $state(false); // the personal/app Settings takeover
  let showServerSettings = $state(false); // the per-server Settings takeover
  let serverNameDraft = $state("");
  // The settings takeovers are Discord-shaped: a sidebar of pages, one page shown at a
  // time. Page ids are stable route names; the catalogs below are the sidebars' contents.
  // `setSearch` filters BOTH sidebars by label (cleared on open, "/" focuses it).
  let settingsPage = $state("appearance");
  let serverSettingsPage = $state("overview");
  let setSearch = $state("");
  type SetPage = { id: string; label: string; cat: string; danger?: boolean };
  const USER_SET_PAGES: SetPage[] = [
    { id: "profile", label: "My Profile", cat: "Account" },
    { id: "devices", label: "Devices", cat: "Account" },
    { id: "vault", label: "Vault & Lock", cat: "Account" },
    { id: "verify", label: "Verification", cat: "Account" },
    { id: "appearance", label: "Appearance", cat: "App" },
    { id: "space", label: "Server Space", cat: "App" },
    { id: "notifications", label: "Notifications", cat: "App" },
    { id: "voice", label: "Voice & Calls", cat: "App" },
    { id: "chatmedia", label: "Chat & Media", cat: "App" },
    { id: "keybinds", label: "Keybinds", cat: "App" },
    { id: "network", label: "Network", cat: "Connection" },
    { id: "updates", label: "Updates", cat: "Connection" },
  ];
  const SRV_SET_PAGES: SetPage[] = [
    { id: "overview", label: "Overview", cat: "Overview" },
    { id: "livery", label: "Livery", cat: "Overview" },
    { id: "members", label: "Members", cat: "People" },
    { id: "badges", label: "Badges", cat: "People" },
    { id: "sdevices", label: "Devices", cat: "People" },
    { id: "invites", label: "Invites", cat: "People" },
    { id: "emoji", label: "Emoji & Stickers", cat: "Content" },
    { id: "calls", label: "Calls & Relay", cat: "Voice" },
    { id: "leave", label: "Leave Server", cat: "Danger", danger: true },
  ];
  // One filter for both sidebars: category headers only survive while a page of theirs does.
  function filterPages(pages: SetPage[], q: string): SetPage[] {
    const n = q.trim().toLowerCase();
    return n ? pages.filter((p) => p.label.toLowerCase().includes(n)) : pages;
  }
  function openSettings(page: string = settingsPage) {
    settingsPage = page;
    setSearch = "";
    showSettings = true;
  }

  function openServerSettings(id: number | null = null, page: string = serverSettingsPage) {
    if (id !== null && id !== activeServerId) switchServer(id);
    serverNameDraft = cur?.name ?? "";
    // The draft never carries the images: set_livery ignores them (set_server_icon /
    // set_server_cursor own those fields).
    liveryDraft = { preset: livery.preset, accent: livery.accent, tokens: { ...livery.tokens }, icon: "", cursor: "" };
    serverSettingsPage = page;
    setSearch = "";
    showServerSettings = true;
  }
  async function renameServer() {
    const name = serverNameDraft.trim();
    if (activeServerId === null || !cur || !name || name === cur.name) return;
    try {
      await invoke("rename_server", { server: activeServerId, name });
      cur.name = name;
    } catch (e) {
      error = String(e);
    }
  }
  let showFeedback = $state(false); // the Send-feedback overlay
  let feedbackKind = $state<"bug" | "feature">("bug");
  let feedbackTitle = $state("");
  let feedbackText = $state("");
  let feedbackCopied = $state(false);
  let feedbackOpened = $state(false);
  // Notification-sound preference (wired to actual playback in 10g), persisted locally.
  let soundOn = $state(typeof localStorage !== "undefined" ? localStorage.getItem("catcoms.sound") !== "off" : true);
  function toggleSound() {
    soundOn = !soundOn;
    try { localStorage.setItem("catcoms.sound", soundOn ? "on" : "off"); } catch { /* ignore */ }
  }

  // Appearance: the whole theme is a token map in app.css; these choices only flip
  // data-attributes / one CSS variable on <html>, so they can never fork the layout.
  // Semantic colours (green=presence, gold=mentions, red=danger) are constant in every preset.
  type Appearance = { preset: string; accent: string; density: string; chrome: string; flat: boolean; icons: string; motion: string; clock: string; scale: number };
  const APPEARANCE_KEY = "catcoms.appearance";
  // clock: "" = the locale's habit, "12"/"24" force a convention. scale: chat text size in
  // percent (100 = the density's own base size); clamped where applied, not where stored.
  const APPEARANCE_DEFAULT: Appearance = { preset: "", accent: "", density: "", chrome: "terminal", flat: true, icons: "", motion: "", clock: "", scale: 100 };
  function loadAppearance(): Appearance {
    try {
      return { ...APPEARANCE_DEFAULT, ...JSON.parse(localStorage.getItem(APPEARANCE_KEY) ?? "{}") };
    } catch {
      return { ...APPEARANCE_DEFAULT };
    }
  }
  let appearance = $state<Appearance>(loadAppearance());
  const PRESETS = [
    { id: "", name: "Nightshade", sw: "#977df2" },
    { id: "aurum", name: "Aurum", sw: "#e2a83d" },
    { id: "verdant", name: "Verdant", sw: "#57c77a" },
    { id: "garnet", name: "Garnet", sw: "#e0574b" },
    { id: "slate", name: "Slate", sw: "#6ca0d8" },
  ];
  const ACCENT_CHOICES = ["#977df2", "#e2a83d", "#e0574b", "#57c77a", "#6ca0d8"];
  // Server livery (design-livery.md): the active server's published scheme. Every value is
  // UNTRUSTED (any member's client may have written the doc): sanitized on read, and only
  // ever able to recolor: preset id, accent, and an allow-list of colour tokens. Semantic
  // tokens (--ok/--warn/--danger) and layout are never livery-controllable.
  type Livery = { preset: string; accent: string; tokens: Record<string, string>; icon: string; cursor: string };
  const emptyLivery = (): Livery => ({ preset: "", accent: "", tokens: {}, icon: "", cursor: "" });
  let livery = $state<Livery>(emptyLivery());
  // Rail icons for every (non-DM) server, fetched from each server's livery doc and kept
  // fresh by livery-changed events. Values are sanitized base64 (rendered as data: URLs).
  let serverIcons = $state<Record<number, string>>({});
  let liveryDraft = $state<Livery>(emptyLivery()); // Server-settings editor draft
  const LIVERY_TOKENS = [
    "--bg-0", "--panel", "--bg-elev", "--border", "--border-soft",
    "--text", "--text-2", "--muted", "--faint", "--accent", "--accent-hi",
  ];
  const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;
  // 64 KiB decoded ≈ 87.4k base64 chars; anything longer or non-base64 is dropped.
  const ICON_B64 = /^[A-Za-z0-9+/=]{0,90000}$/;
  // Typed non-colour livery vocabulary (design-livery-customisation-safety.md): every value
  // is an enum/catalog ID validated here: never a family string, never a URL, never CSS.
  const LIVERY_RADIUS: Record<string, { r: string; rlg: string }> = {
    sharp: { r: "0px", rlg: "2px" },
    soft: { r: "", rlg: "" }, // the default scale: clears the override
    round: { r: "8px", rlg: "14px" },
  };
  const LIVERY_FONTS: Record<string, string> = {
    system: `"Segoe UI", system-ui, -apple-system, sans-serif`,
    serif: `Georgia, "Times New Roman", serif`,
    mono: `"Cascadia Code", ui-monospace, Consolas, monospace`,
    rounded: `"Comic Sans MS", "Segoe UI", system-ui, sans-serif`,
  };
  const LIVERY_PATTERNS = ["none", "grid", "diag", "dots"];
  function sanitizeLivery(l: Livery): Livery {
    const out = emptyLivery();
    if (PRESETS.some((p) => p.id === l.preset)) out.preset = l.preset;
    if (HEX_COLOR.test(l.accent)) out.accent = l.accent;
    for (const [k, v] of Object.entries(l.tokens ?? {})) {
      if (LIVERY_TOKENS.includes(k) && HEX_COLOR.test(v)) out.tokens[k] = v;
      else if (k === "radius" && v in LIVERY_RADIUS) out.tokens[k] = v;
      else if (k === "font" && v in LIVERY_FONTS) out.tokens[k] = v;
      else if (k === "pattern" && LIVERY_PATTERNS.includes(v)) out.tokens[k] = v;
    }
    if (typeof l.icon === "string" && ICON_B64.test(l.icon)) out.icon = l.icon;
    if (typeof l.cursor === "string" && ICON_B64.test(l.cursor) && l.cursor.length <= 24000) out.cursor = l.cursor;
    return out;
  }

  // Deep-validate a livery cursor before it may touch the pointer: decodes, bounds the
  // dimensions, and requires a minimum opaque area so a hostile admin can't hide the cursor.
  // Returns a ready `cursor:` value ("" = rejected); the `, auto` fallback always rides along.
  let liveryCursorUrl = $state("");
  async function validateCursor(b64: string): Promise<string> {
    if (!b64) return "";
    const url = "data:image/png;base64," + b64;
    const img = new Image();
    const ok = await new Promise<boolean>((res) => {
      img.onload = () => res(true);
      img.onerror = () => res(false);
      img.src = url;
    });
    if (!ok || img.naturalWidth > 64 || img.naturalHeight > 64 || img.naturalWidth < 8 || img.naturalHeight < 8) return "";
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const ctx = c.getContext("2d");
    if (!ctx) return "";
    ctx.drawImage(img, 0, 0);
    const px = ctx.getImageData(0, 0, c.width, c.height).data;
    let opaque = 0;
    for (let i = 3; i < px.length; i += 4) if (px[i] > 64) opaque++;
    if (opaque < 24) return ""; // effectively invisible: griefing, not theming
    return `url(${url}) 2 2, auto`;
  }
  let liveryActive = $derived(!!(livery.preset || livery.accent || Object.keys(livery.tokens).length));
  // Per-server "use my own theme" opt-out (precedence rule 1 in the design doc).
  const liveryOptOutKey = (id: number) => `catcoms.appearance.override.${id}`;
  let liveryOptOut = $state(false);
  function loadLiveryOptOut(id: number) {
    try { liveryOptOut = localStorage.getItem(liveryOptOutKey(id)) === "1"; } catch { liveryOptOut = false; }
  }
  function setLiveryOptOut(v: boolean) {
    liveryOptOut = v;
    if (activeServerId === null) return;
    try {
      if (v) localStorage.setItem(liveryOptOutKey(activeServerId), "1");
      else localStorage.removeItem(liveryOptOutKey(activeServerId));
    } catch { /* best-effort */ }
  }
  // A server's livery only applies while you are actually inside that server and have not opted
  // out. The theme effect and the title bar's hairline both hang off this one rule.
  let followLiveryNow = $derived(
    liveryActive && !liveryOptOut && !dmHome && !inboxView && activeServerId !== null,
  );
  async function refreshLivery() {
    if (activeServerId === null || cur?.isDm) {
      livery = emptyLivery();
      return;
    }
    try {
      livery = sanitizeLivery(await invoke<Livery>("get_livery", { server: activeServerId }));
      liveryCursorUrl = await validateCursor(livery.cursor);
    } catch {
      livery = emptyLivery(); // failed/malformed reads degrade to "no livery", never an error
      liveryCursorUrl = "";
    }
  }
  async function publishLivery() {
    if (activeServerId === null) return;
    try {
      await invoke("set_livery", {
        server: activeServerId,
        preset: liveryDraft.preset,
        accent: liveryDraft.accent,
        tokens: liveryDraft.tokens,
      });
      await refreshLivery();
    } catch (e) {
      error = String(e);
    }
  }
  async function removeLivery() {
    if (activeServerId === null) return;
    try {
      await invoke("set_livery", { server: activeServerId, preset: "", accent: "", tokens: {} });
      liveryDraft = emptyLivery();
      await refreshLivery();
    } catch (e) {
      error = String(e);
    }
  }
  // Fetch one server's icon into the rail store (any server, active or not).
  async function refreshServerIconFor(id: number) {
    try {
      const l = sanitizeLivery(await invoke<Livery>("get_livery", { server: id }));
      if (l.icon) serverIcons[id] = l.icon;
      else delete serverIcons[id];
    } catch {
      /* unreachable server actor: keep whatever we had */
    }
  }
  function refreshAllServerIcons() {
    for (const s of servers) if (!s.isDm) refreshServerIconFor(s.id);
  }
  async function setServerIcon(icon: string) {
    if (activeServerId === null) return;
    try {
      await invoke("set_server_icon", { server: activeServerId, icon });
      await refreshLivery();
      await refreshServerIconFor(activeServerId);
    } catch (e) {
      error = String(e);
    }
  }
  async function loadServerIcon(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      await setServerIcon(await fileToSquareJpegB64(file, 128));
    } catch (err) {
      error = String(err);
    }
  }
  // Custom ground tint for a livery: two stops washed into Nightshade's grounds (floor
  // and rail toward the first, panels toward the second). The result is a plain #rrggbb
  // per token, so it rides the EXISTING colour allow-list: every client's read-side
  // sanitizer already accepts it, no protocol change and no new fields.
  const NIGHT_GROUNDS: Record<string, string> = {
    "--bg-0": "#131218",
    "--panel": "#1a1922",
    "--bg-elev": "#232130",
    "--border": "#2e2b3d",
    "--border-soft": "#24222f",
  };
  const GROUND_KEYS = Object.keys(NIGHT_GROUNDS);
  function hexMix(a: string, b: string, t: number): string {
    const c = (s: string, i: number) => parseInt(s.slice(i, i + 2), 16);
    const mix = (i: number) => Math.round(c(a, i) + (c(b, i) - c(a, i)) * t).toString(16).padStart(2, "0");
    return `#${mix(1)}${mix(3)}${mix(5)}`;
  }
  // Two independent tint targets: BACKGROUND (the floor and rail) and SIDEBARS (panels,
  // inputs, borders), each a colour + an intensity. Intensity is the mix ratio, capped at
  // 60% so the grounds stay dark and the untouched text tokens stay legible.
  let liveryTintBgC = $state("#3d2350");
  let liveryTintBgS = $state(28);
  let liveryTintSideC = $state("#23163a");
  let liveryTintSideS = $state(28);
  function tintTokens(): Record<string, string> {
    const tb = Math.max(0, Math.min(60, liveryTintBgS)) / 100;
    const ts = Math.max(0, Math.min(60, liveryTintSideS)) / 100;
    return {
      "--bg-0": hexMix(NIGHT_GROUNDS["--bg-0"], liveryTintBgC, tb),
      "--border-soft": hexMix(NIGHT_GROUNDS["--border-soft"], liveryTintBgC, tb * 0.9),
      "--panel": hexMix(NIGHT_GROUNDS["--panel"], liveryTintSideC, ts),
      "--bg-elev": hexMix(NIGHT_GROUNDS["--bg-elev"], liveryTintSideC, ts * 1.05),
      "--border": hexMix(NIGHT_GROUNDS["--border"], liveryTintSideC, ts * 1.05),
    };
  }
  let draftTinted = $derived(GROUND_KEYS.some((k) => k in liveryDraft.tokens));
  function applyTint() {
    liveryDraft = { ...liveryDraft, tokens: { ...liveryDraft.tokens, ...tintTokens() } };
  }
  function clearTint() {
    const tokens = { ...liveryDraft.tokens };
    for (const k of GROUND_KEYS) delete tokens[k];
    liveryDraft = { ...liveryDraft, tokens };
  }
  // The whole draft as inline custom properties for the Livery preview: colours, corners
  // and interface font, so every control previews before anything is published. The
  // preset rides a data-attribute (the palette rules match a scoped element too), as does
  // the background pattern.
  function liveryDraftVars(): string {
    const parts: string[] = [];
    const accent = liveryDraft.accent || PRESETS.find((p) => p.id === liveryDraft.preset)?.sw || "";
    if (accent) parts.push(`--accent:${accent}`, `--accent-hi:color-mix(in oklab, ${accent} 80%, white)`);
    for (const [k, v] of Object.entries(liveryDraft.tokens)) {
      if (LIVERY_TOKENS.includes(k) && HEX_COLOR.test(v)) parts.push(`${k}:${v}`);
    }
    const rad = LIVERY_RADIUS[liveryDraft.tokens["radius"] ?? ""];
    if (rad?.r) parts.push(`--r:${rad.r}`, `--r-lg:${rad.rlg}`);
    const font = LIVERY_FONTS[liveryDraft.tokens["font"] ?? ""];
    if (font && liveryDraft.tokens["font"] !== "system") parts.push(`--ui:${font}`);
    return parts.join(";");
  }
  // Livery-draft token editing (radius/font/pattern enum ids; "" removes the override).
  function setDraftToken(key: string, val: string) {
    const tokens = { ...liveryDraft.tokens };
    if (val) tokens[key] = val;
    else delete tokens[key];
    liveryDraft = { ...liveryDraft, tokens };
  }
  // Cursor upload: contain-fit into 32×32 PNG (alpha preserved), re-encoded client-side.
  async function fileToCursorPngB64(file: File): Promise<string> {
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      await new Promise((resolve, reject) => {
        img.onload = () => resolve(null);
        img.onerror = () => reject(new Error("could not load image"));
        img.src = url;
      });
      const size = 32;
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        const scale = Math.min(size / img.width, size / img.height);
        const w = img.width * scale;
        const h = img.height * scale;
        ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
      }
      return canvas.toDataURL("image/png").split(",")[1] ?? "";
    } finally {
      URL.revokeObjectURL(url);
    }
  }
  async function setServerCursor(cursor: string) {
    if (activeServerId === null) return;
    try {
      await invoke("set_server_cursor", { server: activeServerId, cursor });
      await refreshLivery();
    } catch (e) {
      error = String(e);
    }
  }
  async function loadServerCursor(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      const b64 = await fileToCursorPngB64(file);
      if (!(await validateCursor(b64))) {
        error = "That image won't work as a cursor: it needs a visible (mostly opaque) shape.";
        return;
      }
      await setServerCursor(b64);
    } catch (err) {
      error = String(err);
    }
  }

  // Apply the effective theme: user per-server opt-out > server livery > user appearance.
  // Density and terminal-chrome are always personal: a livery only recolors.
  $effect(() => {
    const el = document.documentElement;
    const set = (k: string, v: string) => (v ? el.setAttribute("data-" + k, v) : el.removeAttribute("data-" + k));
    const followLivery = followLiveryNow;
    const preset = followLivery ? livery.preset : appearance.preset;
    const accent = followLivery ? livery.accent : appearance.accent;
    set("preset", preset);
    set("density", appearance.density);
    set("chrome", appearance.chrome === "clean" ? "clean" : "terminal");
    // Hover motion is personal and never livery-controllable: a server must not be able to
    // start animating a viewer's chrome. "off" is also what prefers-reduced-motion gets.
    set("motion", appearance.motion === "off" ? "off" : "");
    for (const t of [...LIVERY_TOKENS, "--r", "--r-lg", "--ui", "--livery-cursor"]) el.style.removeProperty(t);
    el.removeAttribute("data-livery-pattern");
    el.removeAttribute("data-livery-cursor");
    if (followLivery) {
      for (const [k, v] of Object.entries(livery.tokens)) {
        if (LIVERY_TOKENS.includes(k)) el.style.setProperty(k, v);
      }
      const rad = LIVERY_RADIUS[livery.tokens["radius"] ?? ""];
      if (rad?.r) {
        el.style.setProperty("--r", rad.r);
        el.style.setProperty("--r-lg", rad.rlg);
      }
      const font = LIVERY_FONTS[livery.tokens["font"] ?? ""];
      if (font && livery.tokens["font"] !== "system") el.style.setProperty("--ui", font);
      const pattern = livery.tokens["pattern"];
      if (pattern && pattern !== "none") el.setAttribute("data-livery-pattern", pattern);
      if (liveryCursorUrl) {
        el.style.setProperty("--livery-cursor", liveryCursorUrl);
        el.setAttribute("data-livery-cursor", "1");
      }
    }
    if (accent) {
      el.style.setProperty("--accent", accent);
      el.style.setProperty("--accent-hi", `color-mix(in oklab, ${accent} 80%, white)`);
    }
    // Chat text size: a personal multiplier on the density's own base. Never livery-controllable,
    // and cleared first so returning the slider to 100% restores the token untouched.
    el.style.removeProperty("--fs-msg");
    const scale = Math.min(140, Math.max(70, appearance.scale || 100));
    if (scale !== 100) {
      const base = appearance.density === "compact" ? 0.78 : 0.84;
      el.style.setProperty("--fs-msg", `${((base * scale) / 100).toFixed(3)}rem`);
    }
    try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(appearance)); } catch { /* best-effort */ }
  });

  // Persistence (9f): a passphrase gate. On launch the app is locked until the user enters
  // their passphrase, which unlocks the on-disk vault and reloads their servers (or, on
  // first run, sets the passphrase and starts empty).
  let locked = $state(true);
  let passphrase = $state("");
  let unlocking = $state(false);

  // --- First run --------------------------------------------------------------------------
  // `unlock` CREATES the vault when there isn't one, so the gate cannot be one screen: on a
  // fresh install a typo would quietly become your secret forever, with nothing to compare it
  // against. `vaultExists` (asked once, before the gate is drawn) splits the two: an existing
  // vault gets the unlock screen, a fresh machine gets setup, which makes you enter the secret
  // twice before anything is written.
  // null = still asking; the gate renders nothing rather than guessing wrong for a frame.
  let vaultExists = $state<boolean | null>(null);
  type SetupStep = "welcome" | "secret" | "confirm" | "look";
  let setupStep = $state<SetupStep>("welcome");
  // "new" founds an identity here; "sync" does the same, then drops straight into the grant
  // ceremony, because a companion device still needs its own local vault to hold the grant.
  let setupPath = $state<"new" | "sync">("new");
  let setupFirst = $state(""); // the secret as first entered, held only until it is confirmed
  let setupMismatch = $state(false);
  let syncIntent = $state(false); // opens the link-a-device panel once the vault is live
  // Setup only: the gate's method tabs and entry panels are shared, and this is what they feed.
  let inSetup = $derived(vaultExists === false);
  // Is a secret-entry panel actually on screen? The melody game turns the whole keyboard into a
  // piano, so it must not be armed while the wizard is showing the welcome or appearance step.
  let gateEntry = $derived(locked && (!inSetup || setupStep === "secret" || setupStep === "confirm"));

  // Clear whichever entry surface the chosen method uses, so "enter it again" starts blank.
  function clearUnlockEntry() {
    stopPlayback();
    releaseAll();
    passphrase = "";
    sigilStrokes = [];
    sigilDrawing = [];
    sigilColors = Array(19).fill(0);
    sigilEmojis = [];
    sigilWord = "";
    melodySeq = [];
  }
  // Step 1 → 2: hold what was entered and blank the surface, so the confirmation is a real
  // second performance rather than a second look at the same drawing.
  function setupCapture() {
    const s = unlockSecret();
    if (!s) return;
    setupFirst = s;
    setupMismatch = false;
    clearUnlockEntry();
    setupStep = "confirm";
  }
  // Step 2: the two performances have to encode identically. They are the same scheme-prefixed
  // strings the KDF sees, so this compares exactly what the vault would be sealed under.
  function setupConfirm() {
    if (!unlockSecret()) return;
    if (unlockSecret() !== setupFirst) {
      setupMismatch = true;
      clearUnlockEntry();
      return;
    }
    setupMismatch = false;
    setupStep = "look";
  }
  // What Enter (and the primary button) means on the gate, which is not always "unlock": during
  // setup the same keystroke must advance the wizard, never write the vault early.
  function gateSubmit() {
    if (!unlockSecret()) return;
    if (!inSetup) {
      unlock();
      return;
    }
    if (setupStep === "secret") setupCapture();
    else if (setupStep === "confirm") setupConfirm();
  }
  function setupRestart() {
    setupFirst = "";
    setupMismatch = false;
    clearUnlockEntry();
    setupStep = "secret";
  }
  // Last step: this is the call that writes the vault. Everything before it is reversible.
  async function setupFinish() {
    if (!setupFirst) return;
    syncIntent = setupPath === "sync";
    await unlock(setupFirst);
    if (!locked) {
      setupFirst = "";
      vaultExists = true;
      if (syncIntent) pairBegin(); // the companion's half of the ceremony, ready to hand over
    }
  }

  // --- Unlock minigames -----------------------------------------------------------------
  // Input surfaces ONLY: every method deterministically encodes to a scheme-prefixed string
  // that feeds the SAME vault KDF ("unlock" invoke): the vault crypto is untouched, and a
  // passphrase remains the recommended, highest-entropy option. The scheme prefix means the
  // same finger pattern on different games can never collide into the same secret.
  type UnlockMethod = "pass" | "sigil" | "melody";
  let unlockMethod = $state<UnlockMethod>("pass");
  // Sigil lock, one screen, freely re-editable in any order: a path drawn over a fixed
  // 19-node magic circle, per-node colour MARKS (optional), a focus-emoji SET, and a masked
  // magic word; folded into one "sigil:v1:…" secret. This REPLACES the spell lock
  // ("spell:v1:", glyphs by catalog index), which is RETIRED exactly as melody v1/v2 were: a
  // vault sealed under spell:v1 must be re-entered under a scheme this build can still
  // produce. Encoding, lattice geometry, the entropy model, the tap-vs-drag classifier and
  // the ring-inscription policy live in `sigil.ts` (pure + unit-tested); only
  // pointer/keyboard input and the ceremony visuals (glow, particles, the cat) belong here.
  let sigilStrokes = $state<number[][]>([]); // committed strokes of node indices: order AND direction significant
  let sigilDrawing = $state<number[]>([]); // the stroke in progress (one pointer drag, or keyboard taps)
  let sigilColors = $state<number[]>(Array(19).fill(0)); // per-node mark 0–3, independent of the path
  let sigilEmojis = $state<string[]>([]); // selection order is UI-only: the encoder canonicalizes
  let sigilWord = $state("");
  // Opt-in ring reveal. The word is the ONE factor a shoulder-surfer can't capture (the path,
  // marks and emoji are drawn in the open), so by default the ring shows a CONSTANT-count
  // rune band derived from (session seed, word): it reshuffles as you type but leaks nothing
  //; not even the length (a per-character or tiled inscription would).
  let sigilShowWord = $state(false);
  let sigilSeed = $state(1);
  let sigilSecret = $derived(encodeSigil(sigilStrokes, sigilColors, sigilEmojis, sigilWord));
  let sigilBits = $derived(sigilBitsOf(sigilStrokes, sigilColors, sigilEmojis, sigilWord));
  let sigilComplete = $derived(!!sigilSecret); // a boolean, so effects don't churn per keystroke
  let sigilWordLen = $derived([...normalizeWord(sigilWord)].length);
  let sigilMarked = $derived(coloredCount(sigilColors));
  let sigilSvgEl = $state<SVGSVGElement | undefined>();
  let sigilTracing = false; // a pointer press is live (as opposed to a keyboard-built stroke)
  let sigilDownPt: { x: number; y: number } | null = null; // where the press began, viewBox units
  let sigilTravel = 0; // max distance strayed from the down-point: feeds classifyGesture
  // Pointer coordinates → viewBox coordinates. The SVG is square (aspect-ratio: 1), so one
  // scale factor serves both axes.
  function sigilXY(e: PointerEvent): { x: number; y: number } | null {
    if (!sigilSvgEl) return null;
    const r = sigilSvgEl.getBoundingClientRect();
    if (!r.width) return null;
    return { x: ((e.clientX - r.left) * SIGIL_VIEW) / r.width, y: ((e.clientY - r.top) * SIGIL_VIEW) / r.height };
  }
  function sigilCommitStroke() {
    // A stroke needs a segment to mean anything: a single latched node is not a path (it is a
    // colour tap; see sigilPointerUp), so it can never silently fork the path field.
    if (sigilDrawing.length >= 2) sigilStrokes = [...sigilStrokes, sigilDrawing];
    sigilDrawing = [];
  }
  function cycleSigilColor(i: number) {
    sigilColors = sigilColors.map((c, j) => (j === i ? (c + 1) % SIGIL_COLORS : c));
  }
  function sigilPointerDown(e: PointerEvent) {
    if (sigilDrawing.length && !sigilTracing) sigilCommitStroke(); // seal a pending keyboard stroke first
    const p = sigilXY(e);
    const hit = p ? hitNode(p.x, p.y) : -1;
    if (hit < 0 || !p) return; // presses begin ON a node; dead space is not a start point
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    sigilTracing = true;
    sigilDownPt = p;
    sigilTravel = 0;
    sigilDrawing = [hit];
  }
  function sigilPointerMove(e: PointerEvent) {
    if (!sigilTracing) return;
    const p = sigilXY(e);
    if (!p) return;
    if (sigilDownPt) sigilTravel = Math.max(sigilTravel, Math.hypot(p.x - sigilDownPt.x, p.y - sigilDownPt.y));
    sigilDrawing = appendHit(sigilDrawing, hitNode(p.x, p.y)); // hard snap: appendHit ignores misses
  }
  function sigilPointerUp() {
    if (!sigilTracing) return;
    sigilTracing = false;
    // One set of nodes, two verbs: classifyGesture (pure, unit-tested) splits them. A tap
    // (one node, travel ≤ TAP_SLOP) cycles that node's mark; entering a second node makes the
    // press a path stroke (lifting the pointer IS the stroke separator); a long wander that
    // latched nothing new is dropped on the floor.
    const g = classifyGesture(sigilDrawing, sigilTravel);
    if (g === "colour") cycleSigilColor(sigilDrawing[0]);
    else if (g === "path") sigilStrokes = [...sigilStrokes, sigilDrawing];
    sigilDrawing = [];
    sigilDownPt = null;
  }
  // Keyboard path: every node is focusable; Enter/Space adds it to the working stroke (the
  // explicit "end stroke" button plays the role the pointer-lift plays for a drag) and C
  // cycles the node's colour mark; the keyboard twin of the tap.
  function sigilNodeKey(e: KeyboardEvent, i: number) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      sigilDrawing = appendHit(sigilDrawing, i);
    } else if (e.key === "c" || e.key === "C") {
      e.preventDefault();
      cycleSigilColor(i);
    }
  }
  function sigilUndo() {
    if (sigilDrawing.length) sigilDrawing = [];
    else sigilStrokes = sigilStrokes.slice(0, -1);
  }
  function toggleSigilEmoji(em: string) {
    // Click to select, click again to deselect. The array keeps UI order; the ENCODER sorts;
    // so deselect/reselect churn can never change the secret.
    if (sigilEmojis.includes(em)) sigilEmojis = sigilEmojis.filter((x) => x !== em);
    else if (sigilEmojis.length < MAX_SIGIL_EMOJI) sigilEmojis = [...sigilEmojis, em];
  }
  // Cat summoning: pure latency theatre. It starts WITH the invoke("unlock") call : the KDF
  // costs real time regardless, so the animation fills a wait that exists anyway : never
  // delays resolution, and a FAILED unlock aborts it: no cat for a wrong word.
  let sigilSummon = $state(false);
  let sigilSummonTimer: ReturnType<typeof setTimeout> | null = null;
  const reducedMotion = () =>
    typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  function startSummon() {
    if (reducedMotion()) return;
    sigilSummon = true;
    if (sigilSummonTimer) clearTimeout(sigilSummonTimer);
    sigilSummonTimer = setTimeout(() => (sigilSummon = false), 900);
  }
  function abortSummon() {
    sigilSummon = false;
    if (sigilSummonTimer) {
      clearTimeout(sigilSummonTimer);
      sigilSummonTimer = null;
    }
  }
  // Reseed the ring runes every time the sigil lock comes up: a camera that filmed one session
  // must not be able to line yesterday's inscription up against today's.
  $effect(() => {
    if (locked && unlockMethod === "sigil") {
      sigilSeed = (typeof crypto !== "undefined" ? crypto.getRandomValues(new Uint32Array(1))[0] : Date.now()) || 1;
    }
  });
  // Particles float up from the assembled circle. Loop discipline is the point: it runs ONLY
  // while the completed circle is actually on screen and the tab is visible, and the effect
  // teardown (unlock, method switch, component destroy) cancels it : a rAF loop left running
  // on a lock screen burns battery forever. One canvas, not N animated DOM nodes.
  let sigilFx = $state<HTMLCanvasElement | undefined>();
  $effect(() => {
    const canvas = sigilFx;
    if (!locked || unlockMethod !== "sigil" || !sigilComplete || !canvas || reducedMotion()) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const W = canvas.width;
    const H = canvas.height;
    type Mote = { x: number; y: number; vx: number; vy: number; r: number; life: number };
    const motes: Mote[] = [];
    let raf = 0;
    let last = performance.now();
    const tint = getComputedStyle(canvas).color; // .sigil-fx sets color: var(--accent)
    const step = (now: number) => {
      const dt = Math.min(50, now - last); // clamp so a background stall can't teleport motes
      last = now;
      if (motes.length < 40 && Math.random() < 0.3) {
        const a = Math.random() * Math.PI * 2;
        const rr = 60 + Math.random() * 160;
        motes.push({
          x: W / 2 + Math.cos(a) * rr,
          y: H / 2 + (Math.abs(Math.sin(a)) * rr) / 2 + 30,
          vx: (Math.random() - 0.5) * 14,
          vy: -30 - Math.random() * 50,
          r: 1.6 + Math.random() * 2.6,
          life: 1,
        });
      }
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = tint;
      for (let i = motes.length - 1; i >= 0; i--) {
        const m = motes[i];
        m.x += (m.vx * dt) / 1000;
        m.y += (m.vy * dt) / 1000;
        m.life -= dt / 2800;
        if (m.life <= 0 || m.y < -6) {
          motes.splice(i, 1);
          continue;
        }
        ctx.globalAlpha = Math.min(0.7, m.life);
        ctx.beginPath();
        ctx.arc(m.x, m.y, m.r, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      raf = requestAnimationFrame(step);
    };
    // Hidden tab ⇒ no frames. rAF is throttled when hidden anyway, but "throttled" is not
    // "stopped", and a lock screen is exactly the surface that sits hidden for days.
    const onVis = () => {
      cancelAnimationFrame(raf);
      if (!document.hidden) {
        last = performance.now();
        raf = requestAnimationFrame(step);
      }
    };
    document.addEventListener("visibilitychange", onVis);
    if (!document.hidden) raf = requestAnimationFrame(step);
    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("visibilitychange", onVis);
      ctx.clearRect(0, 0, W, H);
    };
  });
  // Melody lock: ABSOLUTE MIDI notes: C6 is not C4; octaves carry meaning (and entropy).
  // The on-screen piano shows two octaves with a shift, so any register a MIDI controller
  // played is reachable on screen too. v3 records what a score records: notes that overlap
  // in time collapse into ONE chord event, and how long the event was held quantises to a
  // note value. Encoded "melody:v3:60+64+67.1-62.0-…" (chord tones joined by "+", ascending
  // and de-duplicated so fingering order can't change the secret; ".N" is the duration class
  // and is omitted entirely when rhythm is off). v1 (pitch-class-folded) and v2 (bare notes)
  // are retired: a vault sealed under either must be re-entered under a scheme this build
  // can still produce. The theory and the engraving live in `melody.ts` (pure + unit-tested);
  // only the audio and the input handling need to be here.
  let melodySeq = $state<MelodyEvent[]>([]);
  let melodyOctave = $state(4); // base octave of the on-screen keys (C4 = MIDI 60)
  // Rhythm is opt-out because it is the one setting that can lock a correct player out; the
  // choice is remembered locally (it leaks nothing about the tune itself).
  let melodyRhythm = $state(typeof localStorage !== "undefined" ? localStorage.getItem("catcoms.melody.rhythm") !== "off" : true);
  function toggleRhythm() {
    melodyRhythm = !melodyRhythm;
    try { localStorage.setItem("catcoms.melody.rhythm", melodyRhythm ? "on" : "off"); } catch { /* ignore */ }
  }
  let melodySecret = $derived(encodeMelody(melodySeq, melodyRhythm));
  let melodyBits = $derived(bitsOf(melodySeq, melodyRhythm));
  function bitsTier(b: number): "danger" | "warn" | "ok" {
    return b >= 44 ? "ok" : b >= 28 ? "warn" : "danger";
  }
  // A small synth so the keys sing (its own context: the notification chime has one too).
  // Notes sustain while held, so what you hear is the note length you are about to record.
  let synthCtx: AudioContext | null = null;
  const noteHz = (note: number) => 440 * Math.pow(2, (note - 69) / 12); // A4 = 440
  // Voices are keyed `src:note` so the lock ("me"), call playback, and each call peer can all
  // sound the same pitch at once without stealing each other's oscillators. The lock-side
  // callers never pass src/wave and behave exactly as before.
  const voices = new Map<string, { osc: OscillatorNode; gain: GainNode }>();
  const voiceKey = (note: number, src: string) => `${src}:${note}`;
  function startTone(note: number, src = "me", wave: OscillatorType = "triangle", level = 0.16) {
    try {
      synthCtx ??= new AudioContext();
      if (synthCtx.state === "suspended") void synthCtx.resume();
      stopTone(note, src);
      const o = synthCtx.createOscillator();
      const g = synthCtx.createGain();
      o.type = wave;
      o.frequency.value = noteHz(note);
      const t = synthCtx.currentTime;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(level, t + 0.012); // quick attack
      g.gain.exponentialRampToValueAtTime(level * 0.45, t + 0.4); // settle to a sustain that holds
      o.connect(g).connect(synthCtx.destination);
      o.start();
      o.stop(t + 8); // hard backstop so a lost note-off can never leave a drone
      voices.set(voiceKey(note, src), { osc: o, gain: g });
    } catch {
      /* no audio output: the note still registers */
    }
  }
  function stopTone(note: number, src = "me") {
    const v = voices.get(voiceKey(note, src));
    if (!v || !synthCtx) return;
    voices.delete(voiceKey(note, src));
    try {
      const t = synthCtx.currentTime;
      v.gain.gain.cancelScheduledValues(t);
      v.gain.gain.setValueAtTime(Math.max(v.gain.gain.value, 0.0001), t);
      v.gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.12);
      v.osc.stop(t + 0.16);
    } catch {
      /* already stopped */
    }
  }
  // A short confirmation blip: used by the register controls (z/x and 1–7), which must NEVER
  // land in the sequence. It sounds the C you just moved to, so the shift is audible.
  function playBlip(note: number) {
    try {
      synthCtx ??= new AudioContext();
      if (synthCtx.state === "suspended") void synthCtx.resume();
      const o = synthCtx.createOscillator();
      const g = synthCtx.createGain();
      o.type = "sine";
      o.frequency.value = noteHz(note);
      const t = synthCtx.currentTime;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.1, t + 0.008);
      g.gain.exponentialRampToValueAtTime(0.0001, t + 0.16);
      o.connect(g).connect(synthCtx.destination);
      o.start();
      o.stop(t + 0.18);
    } catch {
      /* silent is fine */
    }
  }
  // --- Note on/off: overlapping notes are one chord, hold time is the note value -----------
  // A group opens on the first note-down and commits when the LAST held note lifts, so legato
  // playing groups (as it does on a real keyboard) and staccato playing does not.
  let heldNotes = $state<number[]>([]); // sounding right now: drives key highlighting
  let chordBuf = $state<number[]>([]); // everything the open group has touched
  let holdMs = $state(0); // live length of the open group, for the "holding…" readout
  let groupStart = 0;
  let holdTimer: ReturnType<typeof setInterval> | null = null;
  function noteOn(note: number) {
    if (heldNotes.includes(note)) return; // key repeat / duplicate note-on
    if (playing) stopPlayback(); // playing over the playback would just be two tunes at once
    if (!chordBuf.length) {
      groupStart = performance.now();
      holdMs = 0;
      holdTimer ??= setInterval(() => (holdMs = performance.now() - groupStart), 40);
    }
    heldNotes = [...heldNotes, note];
    if (!chordBuf.includes(note)) chordBuf = [...chordBuf, note];
    startTone(note);
  }
  function noteOff(note: number) {
    if (!heldNotes.includes(note)) return;
    heldNotes = heldNotes.filter((n) => n !== note);
    stopTone(note);
    if (!heldNotes.length) commitGroup();
  }
  function commitGroup() {
    if (!chordBuf.length) return;
    // normalizeEvent sorts + de-dupes: the secret must depend on which notes, not on finger order.
    melodySeq = [...melodySeq, normalizeEvent(chordBuf, durClass(performance.now() - groupStart))];
    chordBuf = [];
    holdMs = 0;
    if (holdTimer) { clearInterval(holdTimer); holdTimer = null; }
  }
  // Panic release: window blur or leaving the melody tab must not strand a held note.
  function releaseAll() {
    for (const n of heldNotes) stopTone(n);
    heldNotes = [];
    chordBuf = [];
    holdMs = 0;
    if (holdTimer) { clearInterval(holdTimer); holdTimer = null; }
  }
  // DAW-style computer-keyboard mapping while the melody tab is up (a=C … j=B, k=C an
  // octave up; z/x shift the register down/up and 1–7 jump straight to it).
  const KEY_TO_PC: Record<string, number> = { a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11, k: 12 };
  const noteAt = (pc: number) => (melodyOctave + 1) * 12 + pc;
  // key → note fixed at press time, so shifting register mid-hold still releases the right note.
  const keyNotes = new Map<string, number>();
  function setOctave(oct: number) {
    melodyOctave = Math.min(7, Math.max(1, oct));
    playBlip((melodyOctave + 1) * 12); // sound the new bottom C so the shift is audible
  }

  // --- Playback -----------------------------------------------------------------------------
  // Hearing the sequence back is how you learn a tune well enough to reproduce it, and with
  // rhythm on it is the only way to check that your "half" really read as a half. Plays the
  // RECORDED durations, not the ones you happened to hold: what you hear is what is sealed.
  const PLAY_MS = [170, 360, 680, 1050];
  let playing = $state(false);
  let playIdx = $state(-1); // event currently sounding: highlighted on the staff
  let playToken = 0; // bumping this cancels an in-flight playback
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  function stopPlayback() {
    playToken++;
    for (const k of [...voices.keys()]) {
      if (!k.startsWith("me:")) continue; // never silence a call peer's notes from here
      const n = Number(k.slice(3));
      if (!heldNotes.includes(n)) stopTone(n);
    }
    playing = false;
    playIdx = -1;
  }
  async function playMelody() {
    if (playing) return stopPlayback(); // the button is a play/stop toggle
    if (!melodySeq.length || heldNotes.length) return;
    const token = ++playToken;
    playing = true;
    for (let i = 0; i < melodySeq.length && token === playToken; i++) {
      const ev = melodySeq[i];
      playIdx = i;
      for (const n of ev.notes) startTone(n);
      await sleep(PLAY_MS[melodyRhythm ? ev.dur : 1]);
      for (const n of ev.notes) stopTone(n);
      await sleep(70); // a hair of silence so repeated notes are audibly separate
    }
    if (token === playToken) {
      playing = false;
      playIdx = -1;
    }
  }

  // Engraving geometry comes from melody.ts; the width is whatever the lock column gives us, so
  // the staff always spans the panel and only a long tune scrolls.
  let sheetW = $state(560);
  let sheet = $derived(buildSheet(melodySeq, melodyRhythm, sheetW));
  let sheetText = $derived(scoreText(melodySeq, melodyRhythm));
  // Web MIDI (Chromium/WebView2): a connected controller feeds the same pitch-class handler,
  // so you can literally play your unlock tune. Feature-detected; denied/absent is fine.
  let midiName = $state("");
  let midiTried = false;
  async function initMidi() {
    if (midiTried) return;
    midiTried = true;
    try {
      const nav = navigator as Navigator & { requestMIDIAccess?: () => Promise<MIDIAccess> };
      if (!nav.requestMIDIAccess) return;
      const access = await nav.requestMIDIAccess();
      const wire = () => {
        let name = "";
        for (const input of access.inputs.values()) {
          name = input.name ?? "MIDI device";
          input.onmidimessage = (m: MIDIMessageEvent) => {
            const d = m.data;
            if (!d || d.length < 3) return;
            const status = d[0] & 0xf0;
            // Note-off is either 0x80 or a 0x90 with zero velocity: controllers disagree.
            const isOn = status === 0x90 && d[2] > 0;
            const isOff = status === 0x80 || (status === 0x90 && d[2] === 0);
            // Route by surface: the melody lock while locked, the call instrument drawer while
            // in a call. Never both, and never anywhere else.
            if (locked && unlockMethod === "melody") {
              if (isOn) noteOn(d[1]);
              else if (isOff) noteOff(d[1]);
            } else if (inCall && instOpen) {
              if (isOn) instNoteOn(d[1]);
              else if (isOff) instNoteOff(d[1]);
            }
          };
        }
        midiName = name;
      };
      access.onstatechange = wire;
      wire();
    } catch {
      midiName = ""; // permission denied or no MIDI subsystem: on-screen keys remain
    }
  }
  function unlockSecret(): string {
    return unlockMethod === "pass" ? passphrase : unlockMethod === "sigil" ? sigilSecret : melodySecret;
  }

  // --- M6: alternate carry channels for pairing blobs (QR + sound) -----------------------
  // Paste remains the baseline; QR and the acoustic channel are conveniences for the same
  // strings. QR fits both legs when small enough; sound is request-leg-sized only.
  const QR_MAX_CHARS = 2600; // v40-L capacity headroom; beyond this we say "use paste"
  // Svelte action: render `text` as a QR into the canvas (re-renders when text changes).
  function qr(canvas: HTMLCanvasElement, text: string) {
    const draw = (t: string) => {
      if (!t || t.length > QR_MAX_CHARS) return;
      QRCode.toCanvas(canvas, t, { margin: 1, width: 220 }).catch(() => {});
    };
    draw(text);
    return {
      update(t: string) {
        draw(t);
      },
    };
  }
  // Camera QR scan: one shot: resolves with the decoded text or null (no camera / denied
  // / closed). The video runs in a small overlay; jsQR scans frames until a hit.
  let scanOpen = $state(false);
  let scanVideoEl = $state<HTMLVideoElement | undefined>(undefined);
  let scanStream: MediaStream | null = null;
  let scanTimer: ReturnType<typeof setInterval> | null = null;
  let scanTarget: ((text: string | null) => void) | null = null;
  async function scanQr(into: (text: string | null) => void) {
    if (scanOpen) return;
    scanTarget = into;
    scanOpen = true;
    try {
      scanStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" } });
      await tick();
      if (!scanVideoEl) throw new Error("no video element");
      scanVideoEl.srcObject = scanStream;
      await scanVideoEl.play();
      const c = document.createElement("canvas");
      const ctx = c.getContext("2d", { willReadFrequently: true });
      scanTimer = setInterval(() => {
        if (!scanVideoEl || !ctx || scanVideoEl.videoWidth === 0) return;
        c.width = scanVideoEl.videoWidth;
        c.height = scanVideoEl.videoHeight;
        ctx.drawImage(scanVideoEl, 0, 0);
        const img = ctx.getImageData(0, 0, c.width, c.height);
        const hit = jsQR(img.data, img.width, img.height);
        if (hit?.data) closeScan(hit.data);
      }, 180);
    } catch {
      closeScan(null);
      error = "Camera unavailable: paste the code instead.";
    }
  }
  function closeScan(result: string | null) {
    if (scanTimer) clearInterval(scanTimer);
    scanTimer = null;
    scanStream?.getTracks().forEach((t) => t.stop());
    scanStream = null;
    scanOpen = false;
    const cb = scanTarget;
    scanTarget = null;
    cb?.(result);
  }
  // Acoustic channel. Send: render the blob as FSK and play it. Receive: record ~30s of
  // mic audio and try to decode: stops early on success.
  let soundBusy = $state<"" | "send" | "listen">("");
  async function sendBySound(text: string) {
    if (soundBusy) return;
    const payload = new TextEncoder().encode(text);
    if (payload.length > MAX_AUDIO_PAYLOAD) {
      error = "Too large to send as sound: use QR or paste.";
      return;
    }
    soundBusy = "send";
    try {
      const ctx = new AudioContext();
      const wave = encodeAudio(payload, ctx.sampleRate);
      const buf = ctx.createBuffer(1, wave.length, ctx.sampleRate);
      buf.copyToChannel(new Float32Array(wave), 0);
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.connect(ctx.destination);
      await new Promise<void>((res) => {
        src.onended = () => res();
        src.start();
      });
      await ctx.close();
    } catch {
      error = "Audio output unavailable.";
    } finally {
      soundBusy = "";
    }
  }
  async function listenForSound(into: (text: string) => void) {
    if (soundBusy) return;
    soundBusy = "listen";
    let stream: MediaStream | null = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const ctx = new AudioContext();
      const src = ctx.createMediaStreamSource(stream);
      const proc = ctx.createScriptProcessor(4096, 1, 1);
      const chunks: Float32Array[] = [];
      let total = 0;
      const done = new Promise<Uint8Array | null>((res) => {
        proc.onaudioprocess = (e) => {
          const d = e.inputBuffer.getChannelData(0);
          chunks.push(new Float32Array(d));
          total += d.length;
          // Try a decode once a second over the accumulated tail; stop at ~35s.
          if (chunks.length % 12 === 0 || total > ctx.sampleRate * 35) {
            const all = new Float32Array(total);
            let at = 0;
            for (const c of chunks) {
              all.set(c, at);
              at += c.length;
            }
            const hit = decodeAudio(all, ctx.sampleRate);
            if (hit || total > ctx.sampleRate * 35) res(hit);
          }
        };
      });
      src.connect(proc);
      proc.connect(ctx.destination);
      const hit = await done;
      proc.disconnect();
      src.disconnect();
      await ctx.close();
      if (hit) into(new TextDecoder().decode(hit));
      else error = "Didn't catch a transmission: try again closer to the speaker, or paste it.";
    } catch {
      error = "Microphone unavailable: paste the code instead.";
    } finally {
      stream?.getTracks().forEach((t) => t.stop());
      soundBusy = "";
    }
  }

  // --- Multi-device pairing (design-multi-device.md v2.1; ceremony is offline-first) ---
  // Origin side: paste the new device's pairing blob → confirm the SAS (the human gate;
  // nothing is minted or sent before Accept) → mint the sealed all-server grant bundle.
  // The bundle's wrap passphrase is a TRANSPORT passphrase invented for the trip: never
  // the vault passphrase. Admission itself lands in M3; until then the new device holds
  // its grants locally.
  let showLinkDevice = $state(false);
  let linkBlob = $state("");
  let linkInfo = $state<{ deviceId: string; sas: string; servers: string[]; dmCount: number } | null>(null);
  let linkName = $state("");
  let linkPass = $state("");
  let linkBundle = $state("");
  let linkBusy = $state(false);
  function fmtSas(s: string): string {
    return s.length === 6 ? `${s.slice(0, 3)} ${s.slice(3)}` : s;
  }
  async function linkRead() {
    linkBusy = true;
    try {
      const r = await invoke<{ device_id: string; sas: string; servers: string[]; dm_count: number }>(
        "pairing_read",
        { blob: linkBlob.trim() },
      );
      linkInfo = { deviceId: r.device_id, sas: r.sas, servers: r.servers, dmCount: r.dm_count };
    } catch (e) {
      error = String(e);
      linkInfo = null;
    } finally {
      linkBusy = false;
    }
  }
  // Mint takes NO blob: the backend mints only the request the popup showed (the pending
  // ceremony stored at pairing_read), so approved-device === certified-device.
  async function linkMint() {
    if (!linkInfo) return;
    linkBusy = true;
    try {
      const r = await invoke<{ bundle: string }>("pairing_mint", {
        passphrase: linkPass,
        deviceName: linkName.trim() || "device",
        turn: turnMapForMint(),
      });
      linkBundle = r.bundle;
    } catch (e) {
      error = String(e);
    } finally {
      linkBusy = false;
    }
  }
  function closeLinkDevice(declined = false) {
    // Declining (or closing on an un-minted read) burns the nonce backend-side: the
    // design makes a pairing request single-use either way.
    if (declined || (linkInfo && !linkBundle)) invoke("pairing_decline").catch(() => {});
    showLinkDevice = false;
    linkBlob = "";
    linkInfo = null;
    linkName = "";
    linkPass = "";
    linkBundle = "";
  }
  // New-device side (onboarding): generate the pairing blob to carry to the master device,
  // then paste the returned bundle + transport passphrase.
  let pairBlob = $state("");
  let pairDeviceId = $state(""); // this device's id short-code: eyeball-match on the master's popup
  let pairBundle = $state("");
  let pairPass = $state("");
  let pairSummary = $state("");
  async function pairBegin() {
    try {
      const r = await invoke<{ blob: string; device_id: string }>("pairing_begin");
      pairBlob = r.blob;
      pairDeviceId = r.device_id;
    } catch (e) {
      error = String(e);
    }
  }
  async function pairOpen() {
    try {
      const r = await invoke<{ sas: string; device_name: string; servers: { name: string; group_id: string; origin: string }[] }>(
        "pairing_open",
        { bundle: pairBundle.trim(), passphrase: pairPass },
      );
      pairSummary =
        `Grant opened: this device is "${r.device_name}". Final check: this code must match the one the master's popup showed: ${fmtSas(r.sas)}. If it doesn't, discard this grant. ` +
        `Granted for ${r.servers.length} server${r.servers.length === 1 ? "" : "s"}: ${r.servers.map((s) => s.name).join(", ")}.`;
      // The blob and transport passphrase have done their job: don't keep them around.
      pairBundle = "";
      pairPass = "";
    } catch (e) {
      error = String(e);
    }
  }

  let busy = $state(false);
  let error = $state("");
  let displayName = $state("me");
  let advertise = $state(""); // optional reachable address (LAN/public IP) for the founder
  let relay = $state(""); // optional relay-node multiaddr (zero-config NAT traversal)
  // Optional rendezvous multiaddr: when set, the founder registers there so a joiner discovers
  // it with no hard-coded address (just the pasted invite). Persisted as a default (it's usually a
  // stable infra node), pre-filled into the Found form and editable in Settings → Network.
  let rendezvous = $state(
    typeof localStorage !== "undefined" ? (localStorage.getItem("catcoms.rendezvous") ?? "") : ""
  );
  let joinInvite = $state(""); // pasted invite (joiner)
  let copied = $state(false);
  let newChannel = $state("");

  let messages = $state<Msg[]>([]);
  let messagesEl = $state<HTMLUListElement | undefined>(undefined);
  // In-channel message search (Ctrl+F): match indices into the loaded messages + the current one.
  // Beyond the plain substring there's an advanced filter set (Ctrl+Shift+F): author, date range,
  // attachment kind, reactions, reply/pin/edit state: plus a sort order, so the same pass answers
  // "the clip Dana posted last week" as well as "where did we say quorum". Filters stand on their
  // own: with an empty query the filters alone select the matches.
  type SearchSort = "oldest" | "newest" | "author" | "reactions" | "replies";
  type SearchFilters = ReturnType<typeof noFilters>;
  // A hit is (channel, index in that channel's list, the message): the index drives the in-pane
  // highlight/scroll for the open channel, the id drives the jump when a hit lives elsewhere.
  type SearchHit = { ch: string; idx: number; m: Msg };
  const SEARCH_RESULT_CAP = 50; // rows rendered in the results list (stepping still covers them all)
  // Facets that select messages. `sort` and the two match modifiers are deliberately absent from
  // the "n filters" count: they shape the query/order, they don't narrow on their own.
  const NON_FACETS = ["sort", "caseSensitive", "wholeWord"];
  function noFilters() {
    return {
      channel: "", // "" = the open channel · "*" = every channel here · else a channel id
      from: "", // author fingerprint ("" = anyone)
      mentions: "", // a member the message @-mentions
      after: "", // yyyy-mm-dd (inclusive, local day)
      before: "", // yyyy-mm-dd (inclusive, local day)
      hasImage: false,
      hasVideo: false,
      hasAudio: false,
      hasFile: false, // a non-media attachment
      hasLink: false,
      isReply: false,
      hasReplies: false,
      isPinned: false,
      isEdited: false,
      mentionsMe: false,
      fromMe: false,
      reacted: false,
      reactedByMe: false,
      emoji: "", // a specific reaction emoji
      caseSensitive: false,
      wholeWord: false,
      sort: "oldest" as SearchSort,
    };
  }
  let showSearch = $state(false);
  let showSearchAdv = $state(false);
  let searchQuery = $state("");
  let searchPos = $state(0);
  let searchInput = $state<HTMLInputElement | undefined>(undefined);
  let filters = $state<SearchFilters>(noFilters());
  // cid → MIME, from the fileshare index, for classifying a message's `![alt](cid:…)` embeds.
  let fileMime = $derived.by(() => new Map(files.map((f) => [f.cid.toLowerCase(), f.mime] as const)));
  const EMBED_RE = /!\[[^\]]*\]\(cid:([0-9a-fA-F]{1,64})\)/g;
  // What a message carries. `safeMime` accepts only image/video/audio, so anything else: and any
  // cid not in the index yet: reads as a plain attachment, matching how the embed resolver treats it.
  function msgKinds(text: string) {
    const k = { image: false, video: false, audio: false, file: false, link: false };
    for (const m of text.matchAll(EMBED_RE)) {
      const mime = safeMime(fileMime.get(m[1].toLowerCase()) ?? "");
      if (mime.startsWith("image/")) k.image = true;
      else if (mime.startsWith("video/")) k.video = true;
      else if (mime.startsWith("audio/")) k.audio = true;
      else k.file = true;
    }
    k.link = /\bhttps?:\/\/\S/i.test(text);
    return k;
  }
  // A yyyy-mm-dd date input → a local-time bound (start of that day, or its last millisecond so
  // "before" is inclusive). Returns null for an empty/half-typed value, which disables the bound.
  function dayBound(s: string, end: boolean): number | null {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
    if (!m) return null;
    const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
    if (Number.isNaN(d.getTime())) return null;
    if (end) d.setHours(23, 59, 59, 999);
    return d.getTime();
  }
  // How many facets are narrowing the result: drives the "Filters (n)" badge and lets an
  // empty query still search (filters alone are a valid search).
  let filterCount = $derived(
    Object.entries(filters).filter(([k, v]) => !NON_FACETS.includes(k) && v !== "" && v !== false).length
  );
  function reactionCount(m: Msg): number {
    return m.reactions.reduce((n, r) => n + r.by.length, 0);
  }
  // The text predicate, honouring the case/whole-word modifiers. Null when there's no query, so
  // the caller can tell "match everything" from "match nothing".
  function textMatcher(raw: string): ((t: string) => boolean) | null {
    const q = raw.trim();
    if (!q) return null;
    if (!filters.wholeWord) {
      if (filters.caseSensitive) return (t) => t.includes(q);
      const lower = q.toLowerCase();
      return (t) => t.toLowerCase().includes(lower);
    }
    // `\b` misbehaves when the query starts/ends with punctuation, so bound on non-word-or-edge.
    const esc = q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const re = new RegExp(`(?:^|\\W)${esc}(?:\\W|$)`, filters.caseSensitive ? "" : "i");
    return (t) => re.test(t);
  }

  // ---- corpus: which channels the search covers -------------------------------------------
  // The open channel always reads the live `messages`; any other in-scope channel is fetched once
  // into `chanMsgs` (a snapshot, dropped when the search closes) so search can span the server.
  let chanMsgs = $state<Record<string, Msg[]>>({});
  let scopeLoading = $state(false);
  let searchChannels = $derived.by(() => cur?.channels ?? []);
  function channelName(id: string): string {
    return searchChannels.find((c) => c.id === id)?.name ?? id;
  }
  async function loadScope() {
    const id = activeServerId;
    const s = cur;
    if (id === null || !s) return;
    const want = filters.channel === "*" ? s.channels.map((c) => c.id) : filters.channel ? [filters.channel] : [];
    const need = want.filter((c) => c !== s.active && !chanMsgs[c]);
    if (!need.length) return;
    scopeLoading = true;
    try {
      const loaded = await Promise.all(
        need.map((c) =>
          invoke<Msg[]>("get_messages", { server: id, channel: c }).then((msgs) => [c, msgs] as const)
        )
      );
      if (activeServerId !== id) return; // server switched mid-fetch: drop the stale snapshot
      for (const [c, msgs] of loaded) chanMsgs[c] = msgs;
    } catch (e) {
      error = String(e);
    } finally {
      scopeLoading = false;
    }
  }
  let searchCorpus = $derived.by(() => {
    const active = cur?.active ?? "";
    const ids = filters.channel === "*" ? searchChannels.map((c) => c.id) : [filters.channel || active];
    const out: SearchHit[] = [];
    for (const ch of ids) {
      const list = ch === active ? messages : chanMsgs[ch];
      if (list) list.forEach((m, idx) => out.push({ ch, idx, m }));
    }
    return out;
  });
  // Reply counts over the corpus, not just the open channel, so "has replies" and the replies sort
  // stay correct in a server-wide search.
  let corpusReplies = $derived.by(() => {
    const n = new Map<string, number>();
    for (const h of searchCorpus) if (h.m.reply_to) n.set(h.m.reply_to, (n.get(h.m.reply_to) ?? 0) + 1);
    return n;
  });

  let searchMatches = $derived.by(() => {
    const match = textMatcher(searchQuery);
    if (!match && !filterCount) return [] as SearchHit[];
    const after = dayBound(filters.after, false);
    const before = dayBound(filters.before, true);
    const mentionMark = filters.mentions ? `@[${mentionName(nameOf(filters.mentions))}]` : "";
    const wantKind = filters.hasImage || filters.hasVideo || filters.hasAudio || filters.hasFile || filters.hasLink;
    const out: SearchHit[] = [];
    for (const h of searchCorpus) {
      const m = h.m;
      if (match && !match(m.text)) continue;
      if (filters.from && m.author !== filters.from) continue;
      if (filters.fromMe && m.author !== myFp) continue;
      if (mentionMark && !m.text.includes(mentionMark)) continue;
      if (after !== null && m.ts < after) continue;
      if (before !== null && m.ts > before) continue;
      if (filters.isReply && !m.reply_to) continue;
      if (filters.hasReplies && !(m.id && corpusReplies.get(m.id))) continue;
      if (filters.isPinned && !m.pinned) continue;
      if (filters.isEdited && !m.edited) continue;
      if (filters.mentionsMe && !mentionsMe(m.text)) continue;
      if (filters.reacted && !m.reactions.length) continue;
      if (filters.reactedByMe && !m.reactions.some((r) => r.by.includes(myFp))) continue;
      if (filters.emoji && !m.reactions.some((r) => r.emoji === filters.emoji)) continue;
      if (wantKind) {
        const k = msgKinds(m.text);
        if (filters.hasImage && !k.image) continue;
        if (filters.hasVideo && !k.video) continue;
        if (filters.hasAudio && !k.audio) continue;
        if (filters.hasFile && !k.file) continue;
        if (filters.hasLink && !k.link) continue;
      }
      out.push(h);
    }
    return sortMatches(out);
  });
  // Sorted by timestamp rather than corpus position: a multi-channel corpus is grouped by channel,
  // so "oldest first" has to interleave the channels to mean anything.
  function sortMatches(hits: SearchHit[]): SearchHit[] {
    const s = filters.sort;
    return hits.sort((a, b) => {
      if (s === "newest") return b.m.ts - a.m.ts;
      if (s === "author") {
        const c = nameOf(a.m.author).localeCompare(nameOf(b.m.author), undefined, { sensitivity: "base" });
        return c || a.m.ts - b.m.ts;
      }
      if (s === "reactions") return reactionCount(b.m) - reactionCount(a.m) || b.m.ts - a.m.ts;
      if (s === "replies") {
        const ra = a.m.id ? (corpusReplies.get(a.m.id) ?? 0) : 0;
        const rb = b.m.id ? (corpusReplies.get(b.m.id) ?? 0) : 0;
        return rb - ra || b.m.ts - a.m.ts;
      }
      return a.m.ts - b.m.ts;
    });
  }
  // In-pane highlighting only concerns the open channel; hits elsewhere are reached from the list.
  let searchMatchSet = $derived(
    new Set(searchMatches.filter((h) => h.ch === (cur?.active ?? "")).map((h) => h.idx))
  );
  // The cursor is clamped rather than reset: a filter edit or an incoming message can shrink the
  // match list under it, and an out-of-range `searchPos` would otherwise blank the highlight.
  let searchPosClamped = $derived(searchMatches.length ? Math.min(searchPos, searchMatches.length - 1) : 0);
  let searchCur = $derived<SearchHit | undefined>(searchMatches[searchPosClamped]);
  // Everyone the pickers can offer: the roster (so you can filter on someone who hasn't spoken
  // here) plus any author in the corpus (so a departed member is still selectable).
  let searchPeople = $derived.by(() => {
    const seen = new Map<string, string>();
    for (const r of roster) seen.set(r.fingerprint, nameOf(r.fingerprint));
    for (const h of searchCorpus) if (!seen.has(h.m.author)) seen.set(h.m.author, nameOf(h.m.author));
    return [...seen]
      .map(([fp, name]) => ({ fp, name }))
      .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
  });
  let searchEmoji = $derived.by(() => {
    const seen = new Set<string>();
    for (const h of searchCorpus) for (const r of h.m.reactions) seen.add(r.emoji);
    return [...seen].sort();
  });

  // ---- the member typeaheads (From · Mentions) ---------------------------------------------
  type Picker = { q: string; open: boolean; idx: number };
  let fromPick = $state<Picker>({ q: "", open: false, idx: 0 });
  let mentionPick = $state<Picker>({ q: "", open: false, idx: 0 });
  function pickerOptions(p: Picker) {
    const q = p.q.trim().toLowerCase();
    return (q ? searchPeople.filter((x) => x.name.toLowerCase().includes(q)) : searchPeople).slice(0, 8);
  }
  function choosePerson(p: Picker, o: { fp: string; name: string }, set: (fp: string) => void) {
    set(o.fp);
    p.q = o.name;
    p.open = false;
    refilter();
  }
  // Typing narrows the list; emptying the box drops the filter (so it behaves like a text field
  // that happens to autocomplete, not a modal picker you have to explicitly reset).
  function onPickerInput(p: Picker, v: string, current: string, set: (fp: string) => void) {
    p.q = v;
    p.open = true;
    p.idx = 0;
    if (!v.trim() && current) {
      set("");
      refilter();
    }
  }
  function onPickerKey(e: KeyboardEvent, p: Picker, set: (fp: string) => void) {
    const opts = pickerOptions(p);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      p.open = true;
      p.idx = Math.min(p.idx + 1, opts.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      p.idx = Math.max(p.idx - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (p.open && opts[p.idx]) choosePerson(p, opts[p.idx], set);
    } else if (e.key === "Escape" && p.open) {
      e.stopPropagation(); // close the list, not the whole search bar
      p.open = false;
    }
  }

  function scrollToMatch(msgIdx: number) {
    messagesEl?.querySelector(`[data-mi="${msgIdx}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
  }
  // Go to a hit, following it into another channel if that's where it lives. The channel we leave
  // is snapshotted first so its hits don't blink out of the result list mid-switch.
  async function goToHit(h: SearchHit, pos: number) {
    searchPos = pos;
    if (cur && h.ch !== cur.active) {
      chanMsgs[cur.active] = messages;
      await switchTo(h.ch, true);
      if (h.m.id) jumpToMessageId(h.m.id);
      else scrollToMatch(h.idx); // a legacy message has no id: its index still holds
      return;
    }
    scrollToMatch(h.idx);
  }
  function stepMatch(dir: number) {
    if (!searchMatches.length) return;
    const pos = (searchPosClamped + dir + searchMatches.length) % searchMatches.length;
    goToHit(searchMatches[pos], pos);
  }
  function onSearchInput(v: string) {
    searchQuery = v;
    refilter();
  }
  // Re-run from the top after any query/filter change (deriveds recompute on read, so the first
  // match below is already the new one). Only scrolls: refining a query never yanks you into
  // another channel; that's reserved for ↑/↓ and clicking a result.
  function refilter() {
    searchPos = 0;
    const h = searchMatches[0];
    if (h && h.ch === (cur?.active ?? "")) scrollToMatch(h.idx);
  }
  function clearFilters() {
    Object.assign(filters, noFilters());
    fromPick.q = "";
    mentionPick.q = "";
    searchPos = 0;
  }
  function openSearch(advanced = false) {
    showSearch = true;
    if (advanced) showSearchAdv = true;
    queueMicrotask(() => searchInput?.focus());
  }
  function closeSearch() {
    showSearch = false;
    showSearchAdv = false;
    searchQuery = "";
    chanMsgs = {};
    clearFilters();
  }
  // The date shortcuts: "today" and the last n days, in the local calendar.
  function quickRange(days: number) {
    const iso = (d: Date) =>
      `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const now = new Date();
    const from = new Date(now);
    from.setDate(now.getDate() - days);
    filters.after = iso(from);
    filters.before = iso(now);
    refilter();
  }

  // Quick switcher (Ctrl/Cmd+K): one palette over channels, server surfaces, servers and DMs.
  type QuickItem = { label: string; hint: string; run: () => void };
  let showQuickSwitch = $state(false);
  let quickQuery = $state("");
  let quickIdx = $state(0);
  const QUICK_SURFACES: { label: string; tab: Tab }[] = [
    { label: "Files", tab: "files" },
    { label: "Status", tab: "status" },
    { label: "Wiki", tab: "wiki" },
    { label: "Events", tab: "events" },
    { label: "Transfers", tab: "downloads" },
  ];
  let quickItems = $derived.by(() => {
    const out: QuickItem[] = [];
    for (const c of cur?.channels ?? []) {
      out.push({ label: `#${c.name}`, hint: "channel", run: () => { switchTo(c.id); view = "chat"; } });
    }
    if (cur) {
      for (const s of QUICK_SURFACES) out.push({ label: s.label, hint: "surface", run: () => switchView(s.tab) });
    }
    for (const s of railServers) {
      if (s.id !== activeServerId) out.push({ label: s.name, hint: "server", run: () => switchServer(s.id) });
    }
    for (const d of dmList) {
      if (d.id !== activeServerId) out.push({ label: d.name, hint: "dm", run: () => switchServer(d.id) });
    }
    return out;
  });
  let quickResults = $derived.by(() => {
    const q = quickQuery.trim().toLowerCase();
    const hits = q ? quickItems.filter((i) => i.label.toLowerCase().includes(q)) : quickItems;
    return hits.slice(0, 10);
  });
  function openQuickSwitch() {
    quickQuery = "";
    quickIdx = 0;
    showQuickSwitch = true;
  }
  function closeQuickSwitch() {
    showQuickSwitch = false;
    quickQuery = "";
    quickIdx = 0;
  }
  function runQuick(item: QuickItem) {
    closeQuickSwitch();
    item.run();
  }
  function onQuickKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      quickIdx = Math.min(quickIdx + 1, Math.max(quickResults.length - 1, 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      quickIdx = Math.max(quickIdx - 1, 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const item = quickResults[quickIdx];
      if (item) runQuick(item);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation(); // the global Escape chain would otherwise close a second layer too
      closeQuickSwitch();
    }
  }

  // Jump-to-unread: per `server:channel`, the timestamp of the newest message you've seen, persisted
  // to localStorage. Entering a channel snapshots the PRIOR mark as `dividerTs` (so a "New" divider
  // renders before the first message past it), then the mark advances to the latest once loaded.
  let readMarks = $state<Record<string, number>>(loadReadMarks());
  let dividerTs = $state(Number.POSITIVE_INFINITY);
  function loadReadMarks(): Record<string, number> {
    try {
      return JSON.parse(localStorage.getItem("catcoms.readmarks") ?? "{}") as Record<string, number>;
    } catch {
      return {};
    }
  }
  function persistReadMarks() {
    try {
      localStorage.setItem("catcoms.readmarks", JSON.stringify(readMarks));
    } catch {
      /* storage unavailable: read marks are best-effort */
    }
  }
  function chanKey(): string | null {
    if (activeServerId === null || !cur?.active) return null;
    return `${activeServerId}:${cur.active}`;
  }
  function captureDivider() {
    const k = chanKey();
    dividerTs = k ? (readMarks[k] ?? Number.POSITIVE_INFINITY) : Number.POSITIVE_INFINITY;
  }
  function advanceReadMark() {
    const k = chanKey();
    if (!k || !messages.length) return;
    const latest = messages.reduce((a, m) => Math.max(a, m.ts), 0);
    if ((readMarks[k] ?? 0) < latest) {
      readMarks[k] = latest;
      persistReadMarks();
    }
  }
  // Index of the first message newer than the read boundary (-1 if all read).
  // Own messages never count as unread: sending shouldn't raise a "New messages" divider.
  let firstUnreadIdx = $derived(messages.findIndex((m) => m.ts > dividerTs && m.author !== myFp));
  // How many messages sit past that boundary; the divider and the header jump both name it.
  let unreadCount = $derived(firstUnreadIdx < 0 ? 0 : messages.slice(firstUnreadIdx).filter((m) => m.author !== myFp).length);
  // Is this row one of the unread ones? Own messages never count (you just sent them).
  const isUnread = (m: Msg) => firstUnreadIdx >= 0 && m.ts > dividerTs && m.author !== myFp;

  // Day dividers in the log ("thu 2026-08-14" between messages from different days).
  function sameDay(a: number, b: number): boolean {
    const da = new Date(a), db = new Date(b);
    return da.getFullYear() === db.getFullYear() && da.getMonth() === db.getMonth() && da.getDate() === db.getDate();
  }
  function dayLabel(ts: number): string {
    const d = new Date(ts);
    const wd = d.toLocaleDateString(undefined, { weekday: "short" }).toLowerCase();
    const mm = String(d.getMonth() + 1).padStart(2, "0");
    const dd = String(d.getDate()).padStart(2, "0");
    return `${wd} ${d.getFullYear()}-${mm}-${dd}`;
  }

  let draft = $state("");
  let members = $state(1);
  let roster = $state<Member[]>([]);
  // Fingerprints of members reachable right now (a live connection): drives the roster's online
  // dots + the online count. Refreshed with the roster and updated live by 'connectivity-changed'.
  let onlineMembers = $state<Set<string>>(new Set());
  // Per-member presence timing OBSERVED this session for the active server (wall-clock ms): when we
  // saw a member come online / go offline. Only set on a transition we witnessed, so durations are
  // honest (a member already online at load shows "Online" with no fabricated duration). Per-server.
  let onlineSince = $state<Record<string, number>>({});
  let lastSeen = $state<Record<string, number>>({});
  // Ticks every 60s so relative presence times ("Last seen 5m ago") stay current without a reload.
  let nowTick = $state(Date.now());
  let rosterFilter = $state("");
  let filteredRoster = $derived.by(() => {
    const q = rosterFilter.trim().toLowerCase();
    if (!q) return roster;
    return roster.filter(
      (m) => m.fingerprint.toLowerCase().includes(q) || nameOf(m.fingerprint).toLowerCase().includes(q),
    );
  });
  // The member column is split into an "online" then an "offline" group (the offline group is
  // omitted entirely when empty). Both are filtered by the roster search first. Companion
  // devices never appear top-level: they nest under their origin (multi-device M4), and a
  // member counts as online when ANY of their devices is reachable.
  let memberOnline = (m: Member) =>
    m.you ||
    onlineMembers.has(m.fingerprint) ||
    Object.entries(deviceMap).some(([fp, d]) => d.origin === m.fingerprint && onlineMembers.has(fp));
  let onlineRoster = $derived(filteredRoster.filter((m) => !deviceMap[m.fingerprint] && memberOnline(m)));
  let offlineRoster = $derived(filteredRoster.filter((m) => !deviceMap[m.fingerprint] && !memberOnline(m)));
  // Members reachable right now (self always counts): the roster header's "N online".
  let onlineCount = $derived(roster.filter((m) => m.you || onlineMembers.has(m.fingerprint)).length);
  // Compact mono abbreviation for a role badge in a narrow roster row (owner → OWN, admin → ADM).
  function roleAbbr(role: string): string {
    return role === "owner" ? "OWN" : role === "admin" ? "ADM" : role.slice(0, 3).toUpperCase();
  }
  let profiles = $state<Record<string, Prof>>({});
  let files = $state<UiFile[]>([]);
  // Whether ≥1 peer is currently reachable to fetch missing chunks from (a soft availability hint;
  // refreshed alongside the file list). Distinguishes "downloadable" from "no peers online".
  let hasPeers = $state(false);
  let uploading = $state(false);
  let folder = $state(""); // current folder in the Files tab
  let newFolder = $state(""); // new-folder name input
  let dragOver = $state(false); // composer drag-over highlight
  let statuses = $state<Msg[]>([]);
  let statusDraft = $state("");
  let statusEl = $state<HTMLUListElement | undefined>(undefined);
  // Cache of resolved embed media: ciphertext-CID hex -> data: URL (avoids re-fetching).
  const embedCache = new Map<string, string>();

  // Custom emoji (10f): files under the "emoji" folder. code -> cid, and resolved code -> URL.
  let emojiUrls = $state<Record<string, string>>({});
  let newEmojiCode = $state("");
  let newEmojiSize = $state(0); // 0 = default inline size; else a pixel size up to the sticker max
  let showEmoji = $state(false);

  // Right-click context menu (one shared instance). `onSelect` returning true keeps the menu
  // open (used to swap in a confirm prompt for destructive actions).
  type MenuItem =
    | { divider: true }
    | { label: string; icon?: string; danger?: boolean; disabled?: boolean; onSelect: () => unknown };
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let menuEl = $state<HTMLElement | undefined>();
  let composerEl = $state<HTMLTextAreaElement | undefined>();

  // Group the file list into the current folder's subfolders + files-here (folder browser).
  let folderView = $derived.by(() => {
    const base = folder === "" ? "" : folder + "/";
    const subs = new Set<string>();
    const here: UiFile[] = [];
    for (const f of files) {
      const p = f.path ?? "";
      if (p === folder) {
        here.push(f);
      } else if (folder === "" || p.startsWith(base)) {
        const rest = folder === "" ? p : p.slice(base.length);
        const seg = rest.split("/")[0];
        if (seg) subs.add(seg);
      }
    }
    return { subs: [...subs].sort(), here };
  });
  let breadcrumbs = $derived(folder === "" ? [] : folder.split("/"));

  // Custom emoji map: files in the "emoji" folder, keyed by name (sans extension), lowercased.
  // Custom emoji live under the "emoji" fileshare folder, named `<code>` or `<code>~<px>` (the
  // optional size suffix lets an emoji render as a larger "sticker"). `emojiMap` maps code→cid;
  // `emojiSize` maps code→pixel size (capped at the sticker max) for the inline render.
  const EMOJI_MAX_SIZE = 160;
  function parseEmojiName(name: string): { code: string; size: number } {
    const raw = name.replace(/\.[^.]+$/, "").toLowerCase();
    const tilde = raw.indexOf("~");
    if (tilde < 0) return { code: raw, size: 0 };
    const size = parseInt(raw.slice(tilde + 1), 10);
    return {
      code: raw.slice(0, tilde),
      size: Number.isFinite(size) ? Math.min(Math.max(size, 0), EMOJI_MAX_SIZE) : 0,
    };
  }
  let emojiMap = $derived.by(() => {
    const m: Record<string, string> = {};
    for (const f of files) {
      if (f.path === "emoji") {
        const { code } = parseEmojiName(f.name);
        if (code) m[code] = f.cid;
      }
    }
    return m;
  });
  let emojiSize = $derived.by(() => {
    const m: Record<string, number> = {};
    for (const f of files) {
      if (f.path === "emoji") {
        const { code, size } = parseEmojiName(f.name);
        if (code && size) m[code] = size;
      }
    }
    return m;
  });

  // The main pane shows one tab at a time.
  type Tab = "chat" | "files" | "status" | "wiki" | "profile" | "downloads" | "events";
  let view = $state<Tab>("chat");
  let wikiPages = $state<string[]>([]);
  let wikiFilter = $state("");
  let filteredWikiPages = $derived.by(() => {
    const q = wikiFilter.trim().toLowerCase();
    return q ? wikiPages.filter((p) => p.toLowerCase().includes(q)) : wikiPages;
  });
  let wikiMap = $state<Record<string, string>>({}); // name -> body (backlinks + link existence)
  let wikiMeta = $state<Record<string, string>>({}); // name -> "md" | "wiki" (per-page format, shared)
  let activeWikiPage = $state("");
  let wikiBody = $state("");
  let newWikiPage = $state("");
  let wikiDirty = $state(false); // unsaved edits in the open page (avoid clobbering on live updates)
  let wikiEdit = $state(false); // edit (textarea) vs read (rendered) mode
  let wikiEl = $state<HTMLDivElement | undefined>(undefined); // rendered-page container (media resolve)
  let showWikiHelp = $state(false);
  let wikiFormat = $derived(wikiMeta[activeWikiPage] === "wiki" ? "wiki" : "md");
  let wikiPreview = $state(false); // live side-by-side preview in edit mode
  let wikiPreviewEl = $state<HTMLDivElement | undefined>(undefined);
  let wikiTextarea = $state<HTMLTextAreaElement | undefined>(undefined);
  let wikiRedirectedFrom = $state(""); // the #REDIRECT page we auto-followed here from
  let wikiRenaming = $state(false);
  let wikiRenameTo = $state("");
  let wikiDeleteArmed = $state(false); // two-step delete confirm in the page header
  // --- wiki history + edit review (11x) ---
  type UiWikiRev = { id: string; author: string; ts: number; body: string; kind: string; actor: string; note: string };
  type UiWikiPending = { id: string; page: string; author: string; ts: number; expires_ts: number; body: string };
  let wikiReviewDays = $state(0); // server setting: 0 = edits publish immediately
  let wikiPending = $state<UiWikiPending[]>([]); // the live review queue (whole server)
  let wikiHistory = $state<UiWikiRev[]>([]); // the open page's revisions, oldest first
  let showWikiHistory = $state(false); // the history browser replaces the article body
  let wikiHistorySel = $state(""); // selected revision id in the history browser
  // The revision selected in the history browser, and the one before it (its diff base).
  let wikiSelRev = $derived(wikiHistory.find((r) => r.id === wikiHistorySel));
  let wikiSelPrev = $derived.by(() => {
    const i = wikiHistory.findIndex((r) => r.id === wikiHistorySel);
    return i > 0 ? wikiHistory[i - 1] : undefined;
  });
  let wikiSelDiff = $derived.by<DiffLine[]>(() => {
    if (!wikiSelRev) return [];
    // A rejected proposal never landed: its diff base is the revision before it too, which
    // is exactly what the reviewer declined it against.
    return diffLines(wikiSelPrev?.body ?? "", wikiSelRev.body);
  });
  // My own proposal(s) waiting on the open page, so the read view can say so.
  let myWikiPendingHere = $derived(wikiPending.filter((p) => p.page === activeWikiPage && p.author === myFp));
  // The admin review surface: when open it replaces the article area in the main wiki panel.
  let wikiReviewOpen = $state(false);
  // The sidebar's nested page tree: `/`-separated names organise into collapsible folders.
  let wikiCollapsed = $state<Set<string>>(new Set());
  let wikiTree = $derived(buildWikiTree(wikiPages));
  let wikiTreeRows = $derived(visibleRows(wikiTree, wikiCollapsed));
  function toggleWikiFolder(path: string) {
    const next = new Set(wikiCollapsed);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    wikiCollapsed = next;
  }
  // Expand every ancestor folder so the given page's row is visible in the tree.
  function revealWikiPage(name: string) {
    const anc = ancestorsOf(name).filter((a) => wikiCollapsed.has(a));
    if (anc.length === 0) return;
    const next = new Set(wikiCollapsed);
    for (const a of anc) next.delete(a);
    wikiCollapsed = next;
  }
  // The auto-contents box: built from the rendered page's headings (Wikipedia-style numbering).
  type WikiTocItem = { level: number; text: string; id: string; num: string; occ: number };
  let wikiToc = $state<WikiTocItem[]>([]);
  let wikiTocCollapsed = $state(false);
  let wikiTocDir = $derived(tocDirective(wikiBody));
  let showWikiToc = $derived(wikiTocDir !== "notoc" && (wikiToc.length >= 3 || (wikiTocDir === "force" && wikiToc.length > 0)));

  // Pages whose body links to the open page: [[Open Page]] or piped [[Open Page|label]].
  let backlinks = $derived.by(() => {
    if (!activeWikiPage) return [] as string[];
    const plain = `[[${activeWikiPage}]]`.toLowerCase();
    const piped = `[[${activeWikiPage}|`.toLowerCase();
    return Object.entries(wikiMap)
      .filter(([name, body]) => {
        const b = (body ?? "").toLowerCase();
        return name !== activeWikiPage && (b.includes(plain) || b.includes(piped));
      })
      .map(([name]) => name)
      .sort();
  });

  // Profile editor.
  let pName = $state("");
  let pColor = $state("#4f8cff");
  let pFont = $state("system");
  let pEffect = $state("none");
  let pDescription = $state("");
  let pBubble = $state("");
  let pAvatar = $state("");
  let pBanner = $state("");
  // The name-style picker's choices (font face / text effect / colour). Ids are the opaque
  // strings stored in the profile; the tiles preview each one live.
  const NAME_FONTS: { id: string; label: string }[] = [
    { id: "system", label: "System" },
    { id: "serif", label: "Serif" },
    { id: "mono", label: "Mono" },
    { id: "script", label: "Script" },
    { id: "caps", label: "Small caps" },
    { id: "rounded", label: "Rounded" },
    { id: "gothic", label: "Gothic" },
  ];
  // "gradient" (the old accent-mix effect) is deliberately absent from the picker: the
  // custom creator below covers it. Peers who still wear it keep rendering fine.
  const NAME_EFFECTS: { id: string; label: string }[] = [
    { id: "none", label: "Solid" },
    { id: "neon", label: "Neon" },
    { id: "rainbow", label: "Rainbow" },
    { id: "wave", label: "Wave" },
    { id: "pulse", label: "Pulse" },
    { id: "outline", label: "Outline" },
    { id: "retro", label: "Retro" },
    { id: "glitch", label: "Glitch" },
  ];
  // Rainbow/wave/pulse are ANIMATIONS: with motion off (the Appearance toggle or the
  // OS's reduced-motion) they freeze and look like Solid, so their tiles say so.
  const ANIM_FX = new Set(["rainbow", "wave", "pulse"]);
  const prefersStill = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  let fxMotionOff = $derived(appearance.motion === "off" || prefersStill);
  // Curated name colours that stay legible on the dark grounds (content, not theme).
  const NAME_COLORS = ["#977df2", "#6ca0d8", "#57c77a", "#d8a657", "#e0574b", "#e879c0", "#6ee7d8", "#c6c2d6"];
  // Preset message-bubble backgrounds (CSS) the profile editor offers; "" = the default. All chosen
  // dark enough for the white message text (and a text-shadow on custom bubbles backs it up).
  const BUBBLE_PRESETS: { label: string; value: string }[] = [
    { label: "Default", value: "" },
    { label: "Ocean", value: "linear-gradient(135deg,#1a2980,#26415e)" },
    { label: "Sunset", value: "linear-gradient(135deg,#c31432,#5c1020)" },
    { label: "Forest", value: "linear-gradient(135deg,#134e5e,#1c7a4d)" },
    { label: "Grape", value: "linear-gradient(135deg,#41295a,#5d2a6e)" },
    { label: "Ember", value: "linear-gradient(135deg,#8a3a12,#b34700)" },
    { label: "Rose", value: "linear-gradient(135deg,#7a1f3d,#3d1020)" },
    { label: "Slate", value: "#3a3f4b" },
  ];
  // Custom gradient creator. Name gradients pack 2..8 stops + an angle into the OPAQUE
  // effect string as `grad2-rrggbb(-rrggbb)+-deg`: the backend never interprets it, and
  // a build that predates gradients sees an unknown class and falls back to the member's
  // flat colour (graceful, never garbled). Bubble gradients reuse the bubble channel,
  // which already carries preset gradient strings.
  // Optional animation suffix `-a<speed>[r]`: the gradient scrolls along its own angle
  // (that IS the vector; `r` reverses it), speed 1..10 sets the pace. Encoded in the same
  // opaque string, so peers on this build animate it and older builds stay flat.
  const GRAD2_RE = /^grad2-((?:[0-9a-f]{6})(?:-[0-9a-f]{6}){1,7})-(\d{1,3})(?:-a(\d{1,2})(r?))?$/;
  const GRAD_MAX_STOPS = 8;
  let pGradStops = $state<string[]>(["#e879c0", "#977df2"]);
  let pGradDeg = $state(90);
  let pGradSpeed = $state(0);
  let pGradRev = $state(false);
  const grad2Id = () =>
    `grad2-${pGradStops.map((s) => s.slice(1).toLowerCase()).join("-")}-${Math.max(0, Math.min(360, pGradDeg))}` +
    (pGradSpeed > 0 ? `-a${Math.min(10, pGradSpeed)}${pGradRev ? "r" : ""}` : "");
  const BUB_GRAD_RE = /^linear-gradient\(135deg,(#[0-9a-fA-F]{6}),(#[0-9a-fA-F]{6})\)$/;
  let pBubA = $state("#41295a");
  let pBubB = $state("#1a2980");
  const customBubble = () => `linear-gradient(135deg,${pBubA},${pBubB})`;

  let cur = $derived(servers.find((s) => s.id === activeServerId) ?? null);
  let myFp = $derived(roster.find((r) => r.you)?.fingerprint ?? "");
  // My display name in the active server (per-server identity): drives @mention self-highlight
  // and detection of mentions aimed at me. `myMentionName` is the form that round-trips through the
  // `@[Name]` marker (see `mentionName`), so insertion + detection + self-highlight all agree.
  let myName = $derived(myFp ? nameOf(myFp) : "");
  let myMentionName = $derived(mentionName(myName));
  // Reserved fileshare folder for chat/status media embeds uploaded by this member.
  let myEmbedFolder = $derived(myFp ? `embed/${myFp}` : "embed");
  // Member roles (10h): fingerprint -> "owner"|"admin"|"member".
  let roles = $state<Record<string, string>>({});
  let myRole = $derived(roles[myFp] ?? "member");
  // Owners + admins may invite. Admin invites are owner-serialized end-to-end (the joiner is
  // admitted when the owner is next online), and revocation is replay-proof (THREAT-MODEL item 3),
  // so this is safe to surface to admins.
  let canInvite = $derived(myRole === "owner" || myRole === "admin");
  let canModerate = $derived(myRole === "owner" || myRole === "admin");
  let confirmRemoveFp = $state(""); // two-click confirm for member removal

  function activeName(): string {
    return cur?.channels.find((c) => c.id === cur?.active)?.name ?? "";
  }
  function nameOf(fp: string): string {
    // A companion device with no profile of its own renders under its origin's name
    // (attribution is per member; the device tag is added where devices are shown).
    const p = profiles[fp] ?? (deviceMap[fp] ? profiles[deviceMap[fp].origin] : undefined);
    return p?.name?.trim() || fp;
  }
  // Two-letter mono monogram for a rail circle (one letter for a one-character name).
  function monogram(name: string): string {
    return (name ?? "").trim().slice(0, 2).toUpperCase() || "?";
  }
  // A member's custom message-bubble background (CSS), or "" for the default. The value comes from an
  // untrusted profile, so allow only simple colors/gradients: no `url(...)`, `;`, `@`, `{` etc. that
  // could inject CSS.
  function bubbleStyle(fp: string): string {
    const b = (profiles[fp]?.bubble ?? "").trim();
    if (!b) return "";
    if (!/^[#a-z0-9 ,.%()-]+$/i.test(b) || /url|expression|image|var\(/i.test(b)) return "";
    return `background:${b}`;
  }
  // The profile card popover (opened by clicking a member's avatar/name).
  let profileCard = $state<string | null>(null);
  function showProfile(fp: string) {
    if (fp) profileCard = fp;
  }

  // Out-of-band identity verification. The dialog shows both fingerprints for comparison
  // over a trusted channel; "mark verified" is a LOCAL-ONLY note of your own judgement
  // (stored on this device, never gossiped, no cryptographic weight). Per server.
  let verifyFor = $state<string | null>(null);
  let verifiedFps = $state<Set<string>>(new Set());
  const verifiedKey = (id: number) => `catcoms.verified.${id}`;
  function loadVerified(id: number) {
    try {
      verifiedFps = new Set(JSON.parse(localStorage.getItem(verifiedKey(id)) ?? "[]") as string[]);
    } catch {
      verifiedFps = new Set();
    }
  }
  function setVerified(fp: string, v: boolean) {
    if (activeServerId === null) return;
    const next = new Set(verifiedFps);
    if (v) next.add(fp);
    else next.delete(fp);
    verifiedFps = next;
    try { localStorage.setItem(verifiedKey(activeServerId), JSON.stringify([...next])); } catch { /* best-effort */ }
  }
  // Fingerprint in 4-char groups for reading aloud ("A4F2 9C11 0B7D …").
  function fmtFp(fp: string): string {
    return (fp.match(/.{1,4}/g) ?? [fp]).join(" ");
  }

  // Server events (the calendar doc): any member creates; author or owner/admin deletes.
  type UiEvent = { id: string; title: string; body: string; start_ts: number; end_ts: number; author: string; image: string };
  let events = $state<UiEvent[]>([]);
  let evTitle = $state("");
  let evBody = $state("");
  let evStart = $state("");
  let evEnd = $state("");
  let evImage = $state(""); // cid of the poster image for the event being composed ("" = none)
  let evImageBusy = $state(false);
  let confirmDeleteEventId = $state("");
  async function refreshEvents() {
    if (activeServerId === null) {
      events = [];
      return;
    }
    try {
      const knownEvents = new Set(events.map((e) => e.id));
      const hadEvents = events.length > 0;
      const srv = activeServerId;
      events = await invoke<UiEvent[]>("get_events", { server: activeServerId });
      if (hadEvents) {
        for (const ev of events) {
          if (knownEvents.has(ev.id)) continue;
          pushTicker("event", `event:${srv}:${ev.id}`, ev.title, () => void goSurface(srv, "events"));
        }
      }
    } catch {
      events = [];
    }
  }
  async function createEvent() {
    if (activeServerId === null) return;
    const startTs = evStart ? new Date(evStart).getTime() : 0;
    const endTs = evEnd ? new Date(evEnd).getTime() : 0;
    if (!evTitle.trim() || !startTs) return;
    try {
      await invoke("create_event", { server: activeServerId, title: evTitle, body: evBody, startTs, endTs, image: evImage });
      evTitle = ""; evBody = ""; evStart = ""; evEnd = ""; evImage = "";
      await refreshEvents();
    } catch (e) {
      error = String(e);
    }
  }
  // The poster for the event being composed: uploaded like any other share (so it circulates the
  // same way), then referenced by address. Replacing it just points the draft at the new blob.
  async function pickEventImage(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file || activeServerId === null) return;
    evImageBusy = true;
    const tid = toast(`Uploading ${file.name}…`, "info", 0);
    try {
      evImage = await invoke<string>("add_file", {
        server: activeServerId,
        name: file.name,
        mime: file.type || "image/png",
        path: myEmbedFolder,
        data: await readBase64(file),
      });
      updateToast(tid, `Event image set: ${file.name}`, "ok");
      await refreshFiles();
    } catch (e) {
      updateToast(tid, `Upload of ${file.name} failed: ${e}`, "err", 9000);
    } finally {
      evImageBusy = false;
    }
  }

  async function deleteEvent(id: string) {
    if (activeServerId === null) return;
    try {
      await invoke("delete_event", { server: activeServerId, id });
      confirmDeleteEventId = "";
      await refreshEvents();
    } catch (e) {
      error = String(e);
    }
  }
  // "Still relevant": hasn't ended yet (or, with no end, started less than an hour ago).
  function eventLive(e: UiEvent, now: number): boolean {
    return (e.end_ts || e.start_ts + 3_600_000) >= now;
  }
  let upcomingEvents = $derived(events.filter((e) => eventLive(e, nowTick)));
  let pastEvents = $derived(events.filter((e) => !eventLive(e, nowTick)).slice().reverse());
  function fmtEventWhen(e: UiEvent): string {
    const s = new Date(e.start_ts);
    const day = dayLabel(e.start_ts);
    const t = s.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    const end = e.end_ts ? `–${new Date(e.end_ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}` : "";
    return `${day} · ${t}${end}`;
  }

  // News feed (inbox): recent status posts + upcoming events across every server.
  // Client-side aggregation over existing per-server invokes: nothing new on the wire.
  type NewsItem = { server: number; serverName: string; kind: "status" | "event"; ts: number; text: string; author: string };
  let inboxMode = $state<"mentions" | "news">("mentions");
  let newsItems = $state<NewsItem[]>([]);
  let newsLoading = $state(false);
  async function loadNews() {
    newsLoading = true;
    const items: NewsItem[] = [];
    const now = Date.now();
    await Promise.all(
      servers.filter((s) => !s.isDm).map(async (s) => {
        try {
          const [sts, evs] = await Promise.all([
            invoke<Msg[]>("get_statuses", { server: s.id }),
            invoke<UiEvent[]>("get_events", { server: s.id }).catch(() => [] as UiEvent[]),
          ]);
          for (const st of sts.slice(-5))
            items.push({ server: s.id, serverName: s.name, kind: "status", ts: st.ts, text: st.text, author: st.author });
          for (const ev of evs)
            if (eventLive(ev, now))
              items.push({ server: s.id, serverName: s.name, kind: "event", ts: ev.start_ts, text: ev.title, author: ev.author });
        } catch {
          /* unreachable server actor: skip it */
        }
      }),
    );
    newsItems = items;
    newsLoading = false;
  }
  let newsUpcoming = $derived(newsItems.filter((n) => n.kind === "event").sort((a, b) => a.ts - b.ts));
  let newsFeed = $derived(newsItems.filter((n) => n.kind === "status").sort((a, b) => b.ts - a.ts).slice(0, 30));
  function jumpToNews(n: NewsItem) {
    navStepStart(); // the server hop and the surface hop are one move, not two
    inboxView = false;
    switchServer(n.server)
      .then(() => switchView(n.kind === "event" ? "events" : "status"))
      .finally(navStepEnd);
  }

  // Companion devices (multi-device M4): the Devices doc maps a companion's fingerprint to
  // its origin + a device tag. Attribution renders the ORIGIN's identity plus the tag; the
  // tag is member-chosen text, so it renders as content, never chrome.
  type UiDevice = { origin: string; name: string };
  let deviceMap = $state<Record<string, UiDevice>>({});
  async function refreshDevices() {
    if (activeServerId === null) {
      deviceMap = {};
      return;
    }
    try {
      deviceMap = await invoke<Record<string, UiDevice>>("get_devices", { server: activeServerId });
    } catch {
      deviceMap = {};
    }
  }
  // Revoke one of YOUR OWN linked devices (M5 "lost phone"). Origin-only: the backend refuses a
  // device you don't originate, so the UI only offers it where origin === you.
  let confirmRevokeFp = $state("");
  async function revokeDevice(companionFp: string) {
    if (activeServerId === null) return;
    try {
      await invoke("revoke_device", { server: activeServerId, fp: companionFp });
      confirmRevokeFp = "";
      await refreshDevices();
    } catch (e) {
      error = String(e);
    }
  }
  // Resolve a fingerprint for display: a companion renders as its origin (+ tag).
  function identityOf(fp: string): { fp: string; tag: string } {
    const d = deviceMap[fp];
    return d ? { fp: d.origin, tag: d.name } : { fp, tag: "" };
  }
  // Roster grouping: origins (and never-mapped members) at the top level, companions
  // nested under their origin with their device tag.
  let rosterGrouped = $derived.by(() => {
    const companions = new Map<string, { m: Member; tag: string }[]>();
    const top: Member[] = [];
    for (const m of filteredRoster) {
      const d = deviceMap[m.fingerprint];
      if (d) {
        const list = companions.get(d.origin) ?? [];
        list.push({ m, tag: d.name });
        companions.set(d.origin, list);
      } else {
        top.push(m);
      }
    }
    return { top, companions };
  });
  // The origin's TURN strings for every non-DM server, for pairing_mint's passthrough
  // (they live in the frontend's per-server localStorage, invisible to the backend).
  function turnMapForMint(): Record<string, string> {
    const map: Record<string, string> = {};
    for (const s of servers) {
      if (s.isDm) continue;
      try {
        const t = localStorage.getItem(serverTurnKey(s.id));
        if (t) map[String(s.id)] = t;
      } catch {
        /* best-effort */
      }
    }
    return map;
  }
  // Redeem held grants (new device, after pairing_open): join every granted server and
  // register the successes in the rail exactly as an invite join would.
  type PairingJoinResult = { name: string; ok: boolean; error?: string; server?: number };
  let pairJoining = $state(false);
  let pairJoinResults = $state<PairingJoinResult[]>([]);
  async function pairJoinAll() {
    pairJoining = true;
    try {
      const results = await invoke<PairingJoinResult[]>("pairing_join", {});
      pairJoinResults = results;
      for (const r of results) {
        if (!r.ok || r.server === undefined || r.server === null) continue;
        if (servers.some((s) => s.id === r.server)) continue;
        // Same-name hashing means opening "general" lands every member in the same channel.
        const channel = await invoke<string>("open_channel", { server: r.server, name: "general" });
        servers = [
          ...servers,
          { id: r.server, name: r.name, channels: [{ id: channel, name: "general" }], active: channel, unread: [], invite: "", dot: false, isDm: false },
        ];
      }
      const first = results.find((r) => r.ok && r.server !== undefined && r.server !== null);
      if (first?.server !== undefined && first.server !== null) {
        syncIntent = false; // the first-run nudge has done its job; stop forcing the panel open
        switchServer(first.server);
      }
    } catch (e) {
      error = String(e);
    } finally {
      pairJoining = false;
    }
  }

  // Admin-assigned member badges (shared doc; untrusted on read like everything else).
  // Reserved role names are rejected by the backend AND ignored here in case one predates
  // that gate; colours must be #rrggbb or the badge renders in the default chrome colour.
  type MemberBadge = { label: string; color: string };
  const RESERVED_BADGES = new Set(["owner", "admin", "mod", "moderator"]);
  let badges = $state<Record<string, MemberBadge>>({});
  function sanitizeBadge(b: MemberBadge): MemberBadge | null {
    const label = (b.label ?? "").trim();
    if (!label || label.length > 24 || RESERVED_BADGES.has(label.toLowerCase())) return null;
    const color = HEX_COLOR.test(b.color ?? "") ? b.color : "";
    return { label, color };
  }
  async function refreshBadges() {
    if (activeServerId === null) {
      badges = {};
      return;
    }
    try {
      const raw = await invoke<Record<string, MemberBadge>>("get_badges", { server: activeServerId });
      const map: Record<string, MemberBadge> = {};
      for (const [fp, b] of Object.entries(raw)) {
        const ok = sanitizeBadge(b);
        if (ok) map[fp] = ok;
      }
      badges = map;
    } catch {
      badges = {};
    }
  }
  // The badge editor row in Server settings (admin-only affordance).
  let badgeEditFp = $state("");
  let badgeLabelDraft = $state("");
  let badgeColorDraft = $state("#6ca0d8");
  function openBadgeEditor(fp: string) {
    badgeEditFp = fp;
    badgeLabelDraft = badges[fp]?.label ?? "";
    badgeColorDraft = badges[fp]?.color || "#6ca0d8";
  }
  async function saveBadge(fp: string, label: string, color: string) {
    if (activeServerId === null) return;
    if (RESERVED_BADGES.has(label.trim().toLowerCase())) {
      error = `"${label.trim()}" is in use already (reserved for roles).`;
      return;
    }
    try {
      await invoke("set_member_badge", { server: activeServerId, fp, label: label.trim(), color });
      badgeEditFp = "";
      await refreshBadges();
    } catch (e) {
      error = String(e);
    }
  }
  // Font/effect ids are opaque strings in the profile document (the backend stores them
  // verbatim), so a value from a peer on a newer build is tolerated: an unknown font falls
  // back to the system face, an unknown effect to a class with no rule (i.e. plain text).
  function fontClass(font: string): string {
    return font === "serif"
      ? "font-serif"
      : font === "mono"
        ? "font-mono"
        : font === "script"
          ? "font-script"
          : font === "caps"
            ? "font-caps"
            : font === "rounded"
              ? "font-rounded"
              : font === "gothic"
                ? "font-gothic"
                : "";
  }
  // A file-type glyph from the MIME prefix (a small QoL cue in the file browser).
  function fileIcon(mime: string): string {
    if (mime.startsWith("image/")) return "🖼";
    if (mime.startsWith("video/")) return "🎬";
    if (mime.startsWith("audio/")) return "🎵";
    if (mime.startsWith("text/")) return "📝";
    if (mime.includes("pdf")) return "📕";
    if (mime.includes("zip") || mime.includes("compressed")) return "🗜";
    return "📄";
  }
  // Known: none | gradient | neon | rainbow | wave | pulse. Anything else still maps to
  // `fx-<id>` (harmless: no rule matches), but the id is stripped to [a-z0-9-] first so an
  // untrusted profile can't smuggle extra class names into the span.
  function fxClass(effect: string): string {
    if (GRAD2_RE.test(effect)) return "fx-grad2";
    const id = effect.toLowerCase().replace(/[^a-z0-9-]/g, "");
    return id && id !== "none" ? `fx-${id}` : "";
  }
  // The inline half of a custom name gradient: the image itself. The clip rules live in
  // .fx-grad2, so without this style (an old build, a non-matching effect) nothing clips.
  function fxStyle(effect: string): string {
    const m = GRAD2_RE.exec(effect);
    if (!m) return "";
    const stops = m[1].split("-").map((h) => "#" + h);
    const speed = m[3] ? Math.min(10, +m[3]) : 0;
    if (!speed) return `;background-image:linear-gradient(${m[2]}deg, ${stops.join(", ")})`;
    // Animated: the first stop repeats at the end so the 200% tile wraps seamlessly; the
    // keyframes scroll one full tile period. Motion-off rules freeze this with !important.
    const dur = ((11 - speed) * 1.2).toFixed(1);
    return (
      `;background-image:linear-gradient(${m[2]}deg, ${[...stops, stops[0]].join(", ")})` +
      `;background-size:200% 200%;animation:fx-grad2-scroll ${dur}s linear infinite${m[4] === "r" ? " reverse" : ""}`
    );
  }
  function colorStyle(color: string): string {
    return color ? `color:${color}` : "";
  }
  function fmtTime(ts: number): string {
    if (!ts) return "";
    // Settings can force a clock convention; "" keeps the locale's own habit.
    const opts: Intl.DateTimeFormatOptions = { hour: "2-digit", minute: "2-digit" };
    if (appearance.clock === "12") opts.hour12 = true;
    else if (appearance.clock === "24") opts.hourCycle = "h23";
    return new Date(ts).toLocaleTimeString([], opts);
  }
  // Coarse relative duration for presence ("45s", "5m", "3h", "2d").
  function relTime(ms: number): string {
    const s = Math.max(0, Math.round(ms / 1000));
    if (s < 60) return `${s}s`;
    const m = Math.round(s / 60);
    if (m < 60) return `${m}m`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h}h`;
    return `${Math.round(h / 24)}d`;
  }
  // The presence detail line for a member: "You" / "Online" / "Online · 5m" / "Last seen 5m ago" /
  // "Offline". Durations only appear for transitions we actually observed this session.
  function presenceText(fp: string, you: boolean): string {
    if (you) return "You";
    if (onlineMembers.has(fp)) {
      const since = onlineSince[fp];
      return since ? `Online · ${relTime(nowTick - since)}` : "Online";
    }
    const ls = lastSeen[fp];
    return ls ? `Last seen ${relTime(nowTick - ls)} ago` : "Offline";
  }
  function fmtSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  $effect(() => {
    void messages;
    if (messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });

  // Persist the rendezvous address as a reusable default (it's usually a stable infra node).
  $effect(() => {
    try {
      localStorage.setItem("catcoms.rendezvous", rendezvous);
    } catch {
      /* ignore */
    }
  });

  // The eclipse hint + presence are per-server; clear them when switching (the new server re-loads
  // presence via refreshMembers and re-asserts the eclipse hint via its next event).
  $effect(() => {
    void activeServerId;
    eclipseCaution = false;
    onlineMembers = new Set();
    onlineSince = {};
    lastSeen = {};
  });

  // Auto-dismiss a transient error after a few seconds (it's also manually dismissable via the ✕),
  // so a one-off "file not available" doesn't linger forever.
  $effect(() => {
    if (!error) return;
    const t = setTimeout(() => (error = ""), 8000);
    return () => clearTimeout(t);
  });

  // Resolve inline media embeds + custom emoji whenever content or the file index changes.
  // The `tick()` is essential: it waits for Svelte to commit the `{@html renderMessage(...)}`
  // DOM so the [data-embed-cid]/[data-emoji] placeholders exist before we query for them.
  // Without it, on a fresh mount (app restart / HMR / tab switch) this effect runs in the
  // same flush as the {@html} block, finds zero placeholders, and never re-runs: so embeds
  // render on first send but vanish after a restart.
  $effect(() => {
    void messages;
    void statuses;
    void files;
    void emojiUrls;
    void emojiSize;
    void events; // a card for an event that has only just synced
    void wikiPages;
    void wikiMap;
    void profiles; // cards name the author, so a renamed member re-reads
    void view; // switching tabs destroys + recreates this DOM (fresh, unresolved placeholders)
    void inboxView; // returning from the inbox recreates the chat DOM too
    tick().then(() => {
      resolveMedia(messagesEl);
      resolveEmoji(messagesEl);
      resolveRefCards(messagesEl);
      resolveMedia(statusEl);
      resolveEmoji(statusEl);
      resolveRefCards(statusEl);
    });
  });

  // Resolve embeds + emoji + mark missing [[links]] in the rendered wiki page (read mode),
  // build the auto-contents box, and keep the edit-mode live preview resolved too.
  $effect(() => {
    void wikiBody;
    void wikiPages;
    void wikiEdit;
    void wikiPreview;
    void wikiFormat;
    void files;
    void emojiUrls;
    void events;
    void statuses;
    void wikiMap;
    void profiles;
    void view; // re-resolve after a tab switch recreates the wiki DOM
    tick().then(() => {
      if (!wikiEdit) {
        resolveMedia(wikiEl);
        resolveEmoji(wikiEl);
        resolveWikiLinks(wikiEl);
        resolveRefCards(wikiEl);
        decorateWikiHeadings(wikiEl);
      } else if (wikiPreview) {
        resolveMedia(wikiPreviewEl);
        resolveEmoji(wikiPreviewEl);
        resolveWikiLinks(wikiPreviewEl);
        resolveRefCards(wikiPreviewEl);
      }
    });
  });

  // Give each rendered heading an anchor id + a hover "edit" jump, and derive the contents box
  // (hierarchical numbering relative to the smallest heading level on the page, like Wikipedia).
  function decorateWikiHeadings(container: HTMLElement | undefined) {
    if (!container) {
      wikiToc = [];
      return;
    }
    for (const b of Array.from(container.querySelectorAll(".wiki-sec-edit"))) b.remove();
    const hs = Array.from(container.querySelectorAll<HTMLElement>("h1, h2, h3, h4"));
    if (hs.length === 0) {
      wikiToc = [];
      return;
    }
    const min = Math.min(...hs.map((h) => Number(h.tagName[1])));
    const counts = [0, 0, 0, 0];
    const occ = new Map<string, number>(); // nth heading with this text (for the source lookup)
    const items: WikiTocItem[] = [];
    hs.forEach((h, i) => {
      const text = (h.textContent ?? "").trim();
      const depth = Math.min(3, Number(h.tagName[1]) - min);
      counts[depth]++;
      for (let d = depth + 1; d < 4; d++) counts[d] = 0;
      const num = counts.slice(0, depth + 1).join(".");
      const n = occ.get(text) ?? 0;
      occ.set(text, n + 1);
      const id = `wh-${i}`;
      h.id = id;
      const btn = document.createElement("button");
      btn.className = "wiki-sec-edit";
      btn.textContent = "edit";
      btn.title = "Edit this section";
      btn.dataset.secText = text;
      btn.dataset.secOcc = String(n);
      h.appendChild(btn);
      items.push({ level: depth, text, id, num, occ: n });
    });
    wikiToc = items;
  }

  function scrollToWikiHeading(id: string) {
    wikiEl?.querySelector(`#${CSS.escape(id)}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  // A heading's hover "edit": open the editor with the caret on that section's source line.
  function onWikiBodyClick(e: MouseEvent) {
    const btn = (e.target as HTMLElement | null)?.closest<HTMLElement>(".wiki-sec-edit");
    if (!btn) return;
    e.stopPropagation();
    void jumpToWikiSection(btn.dataset.secText ?? "", Number(btn.dataset.secOcc ?? 0));
  }

  function escapeWikiRe(s: string): string {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  async function jumpToWikiSection(text: string, occ: number) {
    // Best-effort: the rendered heading text may differ from its source line (inline markup),
    // so fall back to a plain-text search, then to the top of the page.
    const re =
      wikiFormat === "wiki"
        ? new RegExp(`^=+[ \\t]*${escapeWikiRe(text)}`, "gm")
        : new RegExp(`^#{1,6}[ \\t]*${escapeWikiRe(text)}`, "gm");
    let m: RegExpExecArray | null = null;
    for (let i = 0; (m = re.exec(wikiBody)) !== null; i++) if (i === occ) break;
    const pos = m ? m.index : Math.max(0, wikiBody.indexOf(text));
    const len = m ? m[0].length : 0;
    wikiEdit = true;
    await tick();
    const ta = wikiTextarea;
    if (!ta) return;
    ta.focus();
    ta.setSelectionRange(pos, pos + len);
    const line = wikiBody.slice(0, pos).split("\n").length - 1;
    const lh = parseFloat(getComputedStyle(ta).lineHeight) || 18;
    ta.scrollTop = Math.max(0, line * lh - ta.clientHeight / 3);
  }

  // Pre-resolve every custom-emoji image (small) when the file index changes, into emojiUrls.
  $effect(() => {
    void files;
    if (activeServerId === null) return;
    for (const [code, cid] of Object.entries(emojiMap)) {
      if (!emojiUrls[code]) void loadEmoji(code, cid);
    }
  });
  async function loadEmoji(code: string, cid: string) {
    if (activeServerId === null) return;
    try {
      let url = embedCache.get(cid);
      if (!url) {
        const base64 = await invoke<string>("download_file", { server: activeServerId, cid });
        const file = files.find((f) => f.cid === cid);
        url = `data:${safeMime(file?.mime ?? "") || "image/png"};base64,${base64}`;
        embedCache.set(cid, url);
      }
      emojiUrls = { ...emojiUrls, [code]: url };
    } catch {
      /* leave unresolved; retry on next files change */
    }
  }

  function resolveEmoji(container: HTMLElement | undefined) {
    if (!container) return;
    for (const span of Array.from(container.querySelectorAll<HTMLElement>("[data-emoji]:not([data-resolved])"))) {
      const code = (span.getAttribute("data-emoji") ?? "").toLowerCase();
      const url = emojiUrls[code];
      if (!url) continue; // unknown / not loaded yet: leave :code: text, retry on update
      span.setAttribute("data-resolved", "1");
      const img = document.createElement("img");
      img.src = url;
      img.className = "emoji";
      img.alt = `:${code}:`;
      img.title = `:${code}:`;
      img.dataset.emojiCode = code; // so we can re-apply the size if it loads later
      const sz = emojiSize[code];
      if (sz) {
        img.style.width = `${sz}px`;
        img.style.height = `${sz}px`;
      }
      span.replaceWith(img);
    }
    // Re-apply sizes to already-resolved emoji: the size (from `files`) may have loaded after the
    // image was first resolved (e.g. after returning from the inbox), which would otherwise leave a
    // sticker stuck at the default small size.
    for (const img of Array.from(container.querySelectorAll<HTMLImageElement>("img.emoji[data-emoji-code]"))) {
      const sz = emojiSize[img.dataset.emojiCode ?? ""];
      if (sz) {
        img.style.width = `${sz}px`;
        img.style.height = `${sz}px`;
      }
    }
  }

  function resolveWikiLinks(container: HTMLElement | undefined) {
    if (!container) return;
    for (const a of Array.from(container.querySelectorAll<HTMLElement>("[data-wikilink]"))) {
      a.classList.toggle("missing", !wikiPages.includes(a.getAttribute("data-wikilink") ?? ""));
    }
  }

  // Upload an image as a custom emoji `:code:` (stored under the "emoji" folder).
  async function addEmoji(fileList: FileList | null) {
    const code = newEmojiCode.trim().toLowerCase().replace(/[^a-z0-9_+-]/g, "");
    const file = fileList?.[0];
    if (!code || !file || activeServerId === null) return;
    uploading = true;
    try {
      // Encode an optional size into the file name (`code~px`) so it's shared with everyone.
      const sz = Math.min(Math.max(newEmojiSize, 0), EMOJI_MAX_SIZE);
      await invoke("add_file", {
        server: activeServerId,
        name: sz ? `${code}~${sz}` : code,
        mime: file.type || "image/png",
        path: "emoji",
        data: await readBase64(file),
      });
      newEmojiCode = "";
      await refreshFiles();
    } catch (e) {
      error = String(e);
    } finally {
      uploading = false;
    }
  }
  function insertEmoji(code: string) {
    draft = draft ? `${draft} :${code}:` : `:${code}:`;
    showEmoji = false;
  }

  // Default unicode emoji, shown under the server's custom set (Discord-style). Curated,
  // not exhaustive: cats lead, obviously.
  const EMOJI_SETS: { label: string; list: string[] }[] = [
    { label: "cats & critters", list: ["🐱", "🐈", "🐈‍⬛", "😺", "😸", "😹", "😻", "😼", "😽", "🙀", "😿", "😾", "🐾", "🐶", "🦊", "🐺", "🐻", "🐼", "🐸", "🦉", "🦄", "🐝", "🦋", "🐢", "🐍", "🐙", "🦀", "🐬", "🦈"] },
    { label: "smileys", list: ["😀", "😄", "😁", "😆", "😅", "😂", "🤣", "🙂", "😉", "😊", "😇", "🥰", "😍", "🤩", "😘", "😋", "😜", "🤪", "😎", "🥳", "😏", "😒", "😞", "😢", "😭", "😤", "😠", "🤯", "😳", "🥺", "😱", "😨", "😴", "🤤", "🫠", "🤔", "🤨", "🫡", "🤗", "🤫", "🤭", "🙄", "😬", "😶"] },
    { label: "gestures", list: ["👍", "👎", "👌", "🤌", "✌️", "🤞", "🤘", "🤙", "👏", "🙌", "🫶", "🤝", "🙏", "💪", "👉", "👈", "👋", "✊", "👊", "🖖"] },
    { label: "hearts", list: ["❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "💕", "💞", "💗", "💖", "💘", "💔", "❤️‍🔥"] },
    { label: "food & drink", list: ["☕", "🍵", "🧋", "🥤", "🍺", "🍕", "🍔", "🌮", "🍜", "🍣", "🍙", "🍪", "🍰", "🧁", "🍫", "🍿", "🥐"] },
    { label: "things", list: ["🔥", "✨", "⭐", "⚡", "🌈", "🌙", "☀️", "❄️", "🎉", "🎊", "🎁", "🏆", "🎮", "🎲", "🎧", "🎵", "💻", "⌨️", "📱", "🔒", "🔑", "🛠️", "📌", "📎", "🔍", "💡", "📖", "✏️", "💀", "👻", "🤖", "👾", "🚀", "💯"] },
  ];
  function insertUnicodeEmoji(e: string) {
    draft = draft + e;
    showEmoji = false;
  }

  // Unlock the vault with the entered passphrase and reload persisted servers (9f). A wrong
  // passphrase fails (the vault won't decrypt) and we stay locked, showing the error.
  async function unlock(secret = unlockSecret()) {
    if (!secret) return;
    unlocking = true;
    error = "";
    // The summon starts alongside the KDF call and never gates it: the animation fills
    // latency that exists anyway, and a failed unlock aborts it below.
    if (unlockMethod === "sigil") startSummon();
    try {
      const reloaded = await invoke<Reloaded[]>("unlock", { passphrase: secret });
      passphrase = "";
      sigilStrokes = [];
      sigilDrawing = [];
      sigilColors = Array(19).fill(0);
      sigilEmojis = [];
      sigilWord = "";
      stopPlayback();
      melodySeq = [];
      for (const r of reloaded) {
        servers = [
          ...servers,
          { id: r.server, name: r.name, channels: [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: r.invite, dot: false, isDm: r.is_dm },
        ];
      }
      locked = false;
      passphrase = "";
      const firstServer = servers.find((s) => !s.isDm) ?? servers[0];
      if (firstServer) switchServer(firstServer.id);
      loadInbox(); // populate the inbox badge once the reloaded servers are live
      refreshAllServerIcons(); // rail icons come from each server's livery doc
    } catch (e) {
      abortSummon(); // wrong secret ⇒ no cat: the failure must read as a failure
      error = String(e);
    } finally {
      unlocking = false;
    }
  }

  // Ctrl+L: drop back to the unlock gate. This is a session lock, not a teardown: the node stays
  // online and keeps gossiping (so nothing you sent is stranded and peers don't see you drop),
  // but every window onto the vault's contents is cleared and getting back in costs the
  // passphrase again. Re-entering calls `unlock`, which no-ops on an already-open vault and hands
  // back the registered servers, so no actor or transport is duplicated.
  function lockScreen() {
    if (locked) return;
    if (inCall) leaveVoice(); // never leave a hot mic behind a lock screen
    spaceOpen = false; // and no server names floating behind it either
    showSettings = false;
    showServerSettings = false;
    showFeedback = false;
    showAdd = false;
    showLinkDevice = false;
    showQuickSwitch = false;
    showEmoji = false;
    showInsert = false;
    showPinned = false;
    profileCard = null;
    verifyFor = null;
    fileInfo = null;
    menu = null;
    closeSearch();
    navStack = []; // where you have been is part of what the lock screen takes off the screen
    navAt = -1;
    tickerItems = []; // and so is anything the ticker was naming
    servers = [];
    activeServerId = null;
    dmHome = false;
    inboxView = false;
    messages = [];
    roster = [];
    profiles = {};
    files = [];
    statuses = [];
    events = [];
    wikiPages = [];
    inboxItems = [];
    newsItems = [];
    serverIcons = {};
    delivery = {};
    draft = "";
    passphrase = "";
    error = "";
    syncIntent = false;
    locked = true;
  }

  async function found() {
    busy = true;
    error = "";
    try {
      const r = await invoke<Found>("found_server", { displayName, advertise, relay, rendezvous, isDm: false });
      addServer(r, displayName);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function join() {
    busy = true;
    error = "";
    try {
      const { hex, turn } = unwrapInvite(joinInvite);
      const r = await invoke<Found>("join_server", { inviteHex: hex, displayName, isDm: false });
      if (turn) storeServerTurn(r.server, turn); // inherit the operator's shared TURN
      addServer(r, displayName);
      joinInvite = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  // Start a new DM: found a 1:1 group named after the friend; the invite that comes back is the
  // "friend code" to share. Your own profile name is unchanged (a DM's label is the friend's name).
  async function newDm() {
    const name = dmName.trim();
    if (!name) return;
    busy = true;
    error = "";
    try {
      const r = await invoke<Found>("found_server", { displayName: name, advertise, relay, rendezvous, isDm: true });
      addServer(r, name);
      dmName = "";
      showNewDm = false;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  // Accept a friend code: join the friend's 1:1 group, flagged as a DM.
  async function addFriend() {
    const name = dmName.trim();
    if (!name) return;
    busy = true;
    error = "";
    try {
      const r = await invoke<Found>("join_server", { inviteHex: dmInvite.trim(), displayName: name, isDm: true });
      addServer(r, name);
      dmName = "";
      dmInvite = "";
      showAddFriend = false;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function addServer(r: Found, name: string) {
    servers = [
      ...servers,
      { id: r.server, name, channels: [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: "", dot: false, isDm: r.is_dm },
    ];
    showAdd = false;
    // A server adopts the name as your profile (existing behaviour); a DM's label is the friend's
    // name, so leave your profile alone.
    if (!r.is_dm) pName = name;
    switchServer(r.server);
  }

  // Add a member you share a server with as a friend: found a 1:1 DM and deliver its invite to them
  // IN-BAND over the shared server (they get a pending friend request). Stays on the current server.
  async function startDmWithMember(fp: string) {
    const sourceServer = activeServerId;
    if (sourceServer === null) return;
    const name = nameOf(fp);
    busy = true;
    error = "";
    notice = "";
    menu = null;
    try {
      const r = await invoke<Found>("found_server", { displayName: name, advertise, relay, rendezvous, isDm: true });
      // Add the DM to the list without switching away from the current server.
      servers = [
        ...servers,
        { id: r.server, name, channels: [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: "", dot: false, isDm: true },
      ];
      const invite = (await invoke<string | null>("get_invite", { server: r.server })) ?? "";
      const sent = invite
        ? await invoke<boolean>("send_dm_invite", { server: sourceServer, targetFp: fp, inviteHex: invite })
        : false;
      notice = sent
        ? `Friend request sent to ${name}: they'll see it in their DMs.`
        : `Couldn't reach ${name} right now. Open DMs to share a friend code instead.`;
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  // Pull pending friend requests for one server, merging them into the aggregated list.
  async function refreshDmRequests(server: number) {
    try {
      const reqs = await invoke<{ from_fp: string; from_name: string; invite: string }[]>("get_dm_requests", { server });
      const others = dmRequests.filter((r) => r.server !== server);
      dmRequests = [...others, ...reqs.map((r) => ({ server, ...r }))];
    } catch {
      /* a server that's gone / mid-shutdown: ignore */
    }
  }

  // Accept a friend request: join the DM group, then clear the request on the carrying server.
  async function acceptDmRequest(req: DmRequest) {
    busy = true;
    error = "";
    try {
      const r = await invoke<Found>("join_server", { inviteHex: req.invite, displayName: req.from_name, isDm: true });
      addServer(r, req.from_name);
      await invoke("dismiss_dm_request", { server: req.server, fromFp: req.from_fp });
      dmRequests = dmRequests.filter((x) => !(x.server === req.server && x.from_fp === req.from_fp));
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function declineDmRequest(req: DmRequest) {
    try {
      await invoke("dismiss_dm_request", { server: req.server, fromFp: req.from_fp });
    } catch {
      /* ignore */
    }
    dmRequests = dmRequests.filter((x) => !(x.server === req.server && x.from_fp === req.from_fp));
  }

  // Pull per-DM activity stats for the friends-list sortings (one round-trip for all DMs).
  async function refreshDmStats() {
    try {
      const list = await invoke<DmStat[]>("dm_stats", {});
      const map: Record<number, DmStat> = {};
      for (const s of list) map[s.server] = s;
      dmStats = map;
    } catch (e) {
      error = String(e);
    }
  }

  // Open the DMs area (the friends/DM list); land on the first DM if there is one, else an empty
  // DM-home (no active group) ready for a New DM / Add friend.
  function enterDmHome() {
    dmHome = true;
    inboxView = false;
    menu = null;
    showNewDm = false;
    showAddFriend = false;
    refreshDmStats();
    if (dmList.length) switchServer(dmList[0].id);
    else {
      activeServerId = null;
      clearServerView(); // no active group: drop the previous server's stale messages/roster/etc.
    }
  }
  // Reset the per-server display collections (used when there is no active group, so nothing from a
  // previously-active server lingers behind the empty DM-home placeholder).
  function clearServerView() {
    view = "chat";
    messages = [];
    roster = [];
    members = 0;
    files = [];
    statuses = [];
    onlineMembers = new Set();
  }
  function openNewDm() {
    showNewDm = true;
    showAddFriend = false;
    dmName = "";
  }
  function openAddFriend() {
    showAddFriend = true;
    showNewDm = false;
    dmName = "";
    dmInvite = "";
  }

  async function switchServer(id: number) {
    saveDraftFor(chanKey()); // stash the current channel's draft before switching servers
    activeServerId = id;
    inboxView = false;
    spaceOpen = false; // navigating anywhere leaves the orbit view behind
    const s = servers.find((x) => x.id === id);
    if (s) s.dot = false;
    dmHome = s?.isDm ?? false; // a DM keeps us in DM-home; a server leaves it
    showNewDm = false;
    showAddFriend = false;
    if (showSearch) closeSearch();
    notice = "";
    refreshDmRequests(id); // pick up any friend request that arrived over this server
    // Each server has its own wiki + fileshare; reset per-server view state.
    view = "chat";
    activeWikiPage = "";
    wikiBody = "";
    wikiDirty = false;
    wikiEdit = false;
    wikiMeta = {};
    wikiToc = [];
    wikiPreview = false;
    wikiRedirectedFrom = "";
    wikiRenaming = false;
    wikiDeleteArmed = false;
    wikiDrafts.clear(); // wiki drafts are per-server (page names collide across servers)
    folder = "";
    newFolder = "";
    reactionPickerFor = "";
    replyingTo = "";
    mentionQuery = null;
    showPinned = false;
    mentionChannels = new Set(); // mention badges are scoped to the active server
    acceptCallsHere = loadAccept(id); // this server's call-notification preference
    loadSrvTurn(id); // this server's operator-set TURN (for the Server-settings editor)
    loadLiveryOptOut(id); // whether the user opted out of this server's livery
    loadVerified(id); // this server's locally-verified members
    loadDraftFor(chanKey()); // restore this server's active-channel draft
    captureDivider(); // snapshot the read boundary for this server's active channel
    await Promise.all([
      refresh(),
      refreshMembers(),
      refreshProfiles(),
      refreshFiles(),
      refreshStatuses(),
      refreshInvite(),
      refreshRoles(),
      refreshLivery(),
      refreshTopic(),
      refreshDelivery(),
      refreshBadges(),
      refreshEvents(),
      refreshDevices(),
    ]);
    syncProfileEditor();
  }

  // Populate the Profile tab's editor from this member's own saved profile (so the tab shows
  // current values for the server you just switched to).
  function syncProfileEditor() {
    const me = profiles[myFp];
    if (me) {
      pName = me.name || pName;
      pColor = me.color || pColor;
      pFont = me.font || pFont;
      pEffect = me.effect || pEffect;
      pDescription = me.description ?? "";
      pBubble = me.bubble ?? "";
      pAvatar = me.avatar || "";
      pBanner = me.banner || "";
      // Re-seat the gradient creators on whatever the saved profile carries, so opening
      // the editor shows the stops you actually published rather than the defaults.
      const g = GRAD2_RE.exec(pEffect);
      if (g) {
        pGradStops = g[1].split("-").map((h) => "#" + h);
        pGradDeg = +g[2];
        pGradSpeed = g[3] ? Math.min(10, +g[3]) : 0;
        pGradRev = g[4] === "r";
      }
      const bg = BUB_GRAD_RE.exec(pBubble);
      if (bg && !BUBBLE_PRESETS.some((b) => b.value === pBubble)) {
        pBubA = bg[1];
        pBubB = bg[2];
      }
    }
  }

  async function leaveServer(id: number) {
    try {
      await invoke("leave_server", { server: id });
    } catch (e) {
      error = String(e);
    }
    servers = servers.filter((s) => s.id !== id);
    if (activeServerId === id) {
      if (servers.length) switchServer(servers[0].id);
      else activeServerId = null;
    }
  }

  async function addChannel() {
    const name = newChannel.trim().replace(/^#/, "");
    if (!name || activeServerId === null) return;
    newChannel = "";
    try {
      const id = await invoke<string>("open_channel", { server: activeServerId, name });
      if (cur && !cur.channels.some((c) => c.id === id)) cur.channels.push({ id, name });
      switchTo(id);
    } catch (e) {
      error = String(e);
    }
  }

  // `keepSearch` is set when the search itself is driving the move (jumping to a hit in another
  // channel): everything else closes the search bar, as before.
  async function switchTo(id: string, keepSearch = false) {
    if (!cur) return;
    saveDraftFor(chanKey()); // stash the current channel's draft before leaving it
    cur.active = id;
    loadDraftFor(chanKey()); // restore the target channel's draft
    cur.unread = cur.unread.filter((c) => c !== id);
    if (showSearch && !keepSearch) closeSearch();
    reactionPickerFor = "";
    replyingTo = "";
    mentionQuery = null;
    showPinned = false;
    if (mentionChannels.has(id)) {
      mentionChannels = new Set(mentionChannels);
      mentionChannels.delete(id); // reading the channel clears its mention badge
    }
    captureDivider(); // snapshot the read boundary before refresh advances the mark
    await refresh(); // awaited so a search jump can address the target channel's loaded messages
    refreshTopic();
    refreshDelivery();
  }

  // The active channel's topic (a shared LWW scalar in the channel doc; any member may set it).
  let channelTopic = $state("");
  let editingTopic = $state(false);
  let topicDraft = $state("");
  async function refreshTopic() {
    if (activeServerId === null || !cur?.active) {
      channelTopic = "";
      return;
    }
    try {
      channelTopic = await invoke<string>("get_channel_topic", { server: activeServerId, channel: cur.active });
    } catch {
      channelTopic = "";
    }
  }
  async function saveTopic() {
    if (activeServerId === null || !cur?.active) return;
    const t = topicDraft.trim();
    editingTopic = false;
    if (t === channelTopic) return;
    try {
      await invoke("set_channel_topic", { server: activeServerId, channel: cur.active, topic: t });
      channelTopic = t;
    } catch (e) {
      error = String(e);
    }
  }

  async function refresh() {
    if (!cur || !cur.active || activeServerId === null) return;
    try {
      messages = await invoke<Msg[]>("get_messages", { server: activeServerId, channel: cur.active });
      advanceReadMark();
    } catch (e) {
      error = String(e);
    }
  }

  // Delivery states for OWN messages (docs/design-delivery-states.md). Evidence-based lower
  // bounds: a member is counted only once it has provably built on the message, so counts
  // only rise and 0 means "no proof yet", never "failed". Red is reserved for the one true
  // negative signal we have: no peers reachable at all.
  type DeliveryState = { id: string; delivered: number; reachable: number };
  let delivery = $state<Record<string, DeliveryState>>({});
  async function refreshDelivery() {
    if (activeServerId === null || !cur?.active) {
      delivery = {};
      return;
    }
    try {
      const list = await invoke<DeliveryState[]>("get_delivery", { server: activeServerId, channel: cur.active });
      const map: Record<string, DeliveryState> = {};
      for (const s of list) map[s.id] = s;
      delivery = map;
    } catch {
      delivery = {}; // older backend or closed actor: ticks simply don't render
    }
  }
  // The gutter tick for one of your messages: ✕ no peers · ◌ no proof yet · ~ partial ·
  // ✓ all reachable confirmed · ✓✓ the whole roster confirmed.
  function deliveryTick(m: Msg): { g: string; cls: string; tip: string } | null {
    if (m.author !== myFp || !m.id) return null;
    const total = Math.max(members - 1, 0);
    if (total === 0) return null; // alone here: nothing to deliver to
    const d = delivery[m.id];
    const del = d?.delivered ?? 0;
    const reach = d?.reachable ?? Math.max(onlineCount - 1, 0);
    if (del >= total)
      return { g: "✓✓", cls: "d-all", tip: `Delivered to everyone: all ${total} other member${total === 1 ? "" : "s"} proved they hold this message.` };
    if (reach === 0)
      return { g: "✕", cls: "d-none", tip: "No peers reachable: queued; it gossips automatically when members reconnect. Not lost." };
    if (del >= reach)
      return { g: "✓", cls: "d-ok", tip: `Delivered to all ${reach} reachable member${reach === 1 ? "" : "s"} (${del}/${total} confirmed overall). Confirmation is proof-based: silent receivers may also have it.` };
    if (del > 0)
      return { g: "~", cls: "d-part", tip: `Delivering: ${del} of ${reach} reachable confirmed (${total} members in total). Members confirm by building on the message.` };
    return { g: "◌", cls: "d-wait", tip: `Sent: no confirmations yet from ${reach} reachable member${reach === 1 ? "" : "s"}. Silent receipt isn't visible; the count only rises.` };
  }
  // Index of your most recent message in the log (-1 if none): the receipt line's anchor.
  let lastOwnIdx = $derived(messages.reduce((acc, m, i) => (m.author === myFp ? i : acc), -1));
  // The spelled-out receipt under one of your messages. Same evidence as the gutter tick, in
  // words. Shown on your latest message (the state you actually care about) and on any older
  // one that hasn't settled yet; a delivered-and-superseded message stays quiet.
  function deliveryReceipt(m: Msg, mi: number): { g: string; label: string; cls: string; tip: string } | null {
    const t = deliveryTick(m);
    if (!t || !m.id) return null;
    if ((t.cls === "d-all" || t.cls === "d-ok") && mi !== lastOwnIdx) return null;
    const d = delivery[m.id];
    const total = Math.max(members - 1, 0);
    const del = d?.delivered ?? 0;
    const reach = d?.reachable ?? Math.max(onlineCount - 1, 0);
    const label =
      t.cls === "d-none"
        ? "queued · no peers reachable"
        : t.cls === "d-wait"
          ? "sending…"
          : t.cls === "d-part"
            ? `delivering · ${del}/${reach} peers`
            : t.cls === "d-ok"
              ? `delivered · ${del} peer${del === 1 ? "" : "s"}`
              : `delivered · all ${total} member${total === 1 ? "" : "s"}`;
    return { g: t.g, label, cls: t.cls, tip: t.tip };
  }
  async function refreshMembers() {
    const id = activeServerId;
    if (id === null) return;
    try {
      const r = await invoke<Member[]>("get_members", { server: id });
      const online = await invoke<string[]>("get_online_members", { server: id });
      if (activeServerId !== id) return; // server switched mid-fetch: drop stale results
      roster = r;
      members = r.length;
      onlineMembers = new Set(online);
    } catch (e) {
      error = String(e);
    }
  }
  async function refreshProfiles() {
    if (activeServerId === null) return;
    try {
      const list = await invoke<Prof[]>("get_profiles", { server: activeServerId });
      const map: Record<string, Prof> = {};
      for (const p of list) map[p.fingerprint] = p;
      profiles = map;
    } catch (e) {
      error = String(e);
    }
  }
  async function refreshFiles() {
    if (activeServerId === null) return;
    try {
      const v = await invoke<{ files: UiFile[]; has_peers: boolean }>("get_files", {
        server: activeServerId,
      });
      files = v.files;
      hasPeers = v.has_peers;
      // Wiki-pinned content addresses, derived fresh from the wiki on the backend each call: a
      // file embedded in a live page never drops out of circulation, whatever its expiry says.
      // One extra round-trip per refresh, so the Files tab and Properties can say so instantly.
      wikiPinned = new Set(
        await invoke<string[]>("get_wiki_pinned_cids", { server: activeServerId })
      );
    } catch (e) {
      error = String(e);
    }
  }
  // Lowercase-hex cids embedded in a live wiki page (the never-decay set).
  let wikiPinned = $state<Set<string>>(new Set());
  const isPinned = (cid: string) => wikiPinned.has(cid.toLowerCase());

  // The availability of a file for the browser indicator: held locally / partially downloaded /
  // fetchable from peers / no peers online: or actively downloading. Reactive (reads files,
  // downloads, hasPeers). The colour conveys it; `label` is the status text.
  type Avail = { cls: string; icon: string; label: string };
  function availOf(f: UiFile): Avail {
    const dl =
      activeServerId !== null ? downloads[dlKey(activeServerId, f.cid)] : undefined;
    if (dl && dl.status === "downloading")
      return { cls: "downloading", icon: "↓", label: `Downloading ${Math.round(dl.progress * 100)}%` };
    if (dl && dl.status === "queued")
      return { cls: "downloading", icon: "↓", label: "Queued" };
    if (f.total > 0 && f.held >= f.total)
      return { cls: "local", icon: "●", label: "On this device" };
    if (f.held > 0)
      return { cls: "partial", icon: "◐", label: `Partial ${f.held}/${f.total}` };
    if (hasPeers) return { cls: "remote", icon: "○", label: "Downloadable" };
    return { cls: "offline", icon: "○", label: "No peers online" };
  }
  async function refreshStatuses() {
    if (activeServerId === null) return;
    try {
      const knownStatuses = new Set(statuses.map((s) => s.id));
      const hadStatuses = statuses.length > 0;
      const srv = activeServerId;
      statuses = await invoke<Msg[]>("get_statuses", { server: activeServerId });
      if (hadStatuses) {
        for (const st of statuses) {
          if (knownStatuses.has(st.id)) continue;
          pushTicker("status", `status:${srv}:${st.id}`, `${nameOf(st.author)}: ${msgSnippet(st.text, 60)}`, () =>
            void goSurface(srv, "status"),
          );
        }
      }
    } catch (e) {
      error = String(e);
    }
  }
  async function refreshRoles() {
    if (activeServerId === null) return;
    try {
      roles = await invoke<Record<string, string>>("get_roles", { server: activeServerId });
    } catch (e) {
      error = String(e);
    }
  }
  async function setAdmin(fp: string, admin: boolean) {
    if (activeServerId === null) return;
    try {
      await invoke("set_admin", { server: activeServerId, fp, admin });
      await refreshRoles();
    } catch (e) {
      error = String(e);
    }
  }
  async function removeMember(fp: string) {
    if (activeServerId === null) return;
    confirmRemoveFp = "";
    try {
      await invoke("remove_member", { server: activeServerId, fp });
      await Promise.all([refreshMembers(), refreshRoles()]);
    } catch (e) {
      error = String(e);
    }
  }
  async function postStatus() {
    const text = statusDraft.trim();
    if (!text || activeServerId === null) return;
    statusDraft = "";
    try {
      await invoke("post_status", { server: activeServerId, text });
    } catch (e) {
      error = String(e);
    }
  }

  function switchView(v: Tab) {
    menu = null;
    view = v;
    if (v === "wiki") refreshWiki();
    if (v === "files") refreshFiles(); // re-evaluate availability each time the tab opens
  }

  // Delegated click handler for rendered rich text: [[wiki links]] navigate to the wiki tab.
  async function handleRichClick(e: MouseEvent) {
    const target = e.target as HTMLElement | null;
    // A spoiler: first click reveals it (don't also follow any link inside).
    const sp = target?.closest("[data-spoiler]") as HTMLElement | null;
    if (sp && !sp.classList.contains("revealed")) {
      e.preventDefault();
      sp.classList.add("revealed");
      return;
    }
    // An inline image: fill the screen with it. Skipped when the image is wrapped in a link, so
    // the link still wins.
    const im = target?.closest("img.embed-image") as HTMLImageElement | null;
    if (im && !im.closest("a[href],[data-wikilink]")) {
      e.preventDefault();
      openLightbox(im);
      return;
    }
    const el = target?.closest("[data-wikilink]") as HTMLElement | null;
    if (el) {
      e.preventDefault();
      const page = el.getAttribute("data-wikilink") ?? "";
      if (page) {
        view = "wiki";
        await openWikiPage(page);
      }
      return;
    }
    // Reference chips inserted by the "+" picker.
    const fl = target?.closest("[data-file-cid]") as HTMLElement | null;
    if (fl) {
      e.preventDefault();
      await openFileRef((fl.getAttribute("data-file-cid") ?? "").toLowerCase());
      return;
    }
    const el2 = target?.closest("[data-event-id]") as HTMLElement | null;
    if (el2) {
      e.preventDefault();
      openEventRef(el2.getAttribute("data-event-id") ?? "");
      return;
    }
    const sl = target?.closest("[data-status-id]") as HTMLElement | null;
    if (sl) {
      e.preventDefault();
      await openStatusRef(sl.getAttribute("data-status-id") ?? "");
    }
  }

  // Svelte action: delegate clicks inside a rendered-rich-text container (attaches the
  // listener imperatively, so no a11y warning for a click on a non-interactive container).
  function richClicks(node: HTMLElement) {
    const h = (e: Event) => handleRichClick(e as MouseEvent);
    const c = (e: Event) => handleRichContext(e as MouseEvent);
    // A link card is focusable and reads as a button, so it has to open from the keyboard too;
    // the synthesized click goes through the same delegated handler as a real one.
    const k = (e: Event) => {
      const ev = e as KeyboardEvent;
      if (ev.key !== "Enter" && ev.key !== " ") return;
      const card = (ev.target as HTMLElement | null)?.closest(".ref-card") as HTMLElement | null;
      if (!card) return;
      ev.preventDefault();
      card.click();
    };
    node.addEventListener("click", h);
    node.addEventListener("contextmenu", c);
    node.addEventListener("keydown", k);
    return {
      destroy: () => {
        node.removeEventListener("click", h);
        node.removeEventListener("contextmenu", c);
        node.removeEventListener("keydown", k);
      },
    };
  }

  // --- right-click context menu --------------------------------------------------------------
  function openMenu(e: MouseEvent, items: MenuItem[]) {
    if (items.length === 0) return;
    e.preventDefault();
    e.stopPropagation();
    menu = { x: e.clientX, y: e.clientY, items };
  }

  // Svelte action: open a context menu on right-click, with items built fresh at click time
  // (so they capture current roles/draft/etc.).
  function contextMenu(node: HTMLElement, factory: () => MenuItem[]) {
    let make = factory;
    const h = (e: MouseEvent) => {
      // An inline embed and a link card each build their own menu in handleRichContext, which is
      // delegated on the container above this node. Let the event bubble there instead of opening
      // (and stopping at) the row's menu; that handler folds this row's items in below its own.
      if ((e.target as HTMLElement | null)?.closest("[data-embed-cid],.ref-card")) return;
      openMenu(e, make());
    };
    node.addEventListener("contextmenu", h);
    return {
      update(f: () => MenuItem[]) {
        make = f;
      },
      destroy() {
        node.removeEventListener("contextmenu", h);
      },
    };
  }

  // Keep the menu on-screen + focus it once rendered (clamp against the viewport).
  $effect(() => {
    if (!menu || !menuEl) return;
    const w = menuEl.offsetWidth;
    const h = menuEl.offsetHeight;
    const x = Math.max(4, Math.min(menu.x, window.innerWidth - w - 8));
    const y = Math.max(4, Math.min(menu.y, window.innerHeight - h - 8));
    menuEl.style.left = `${x}px`;
    menuEl.style.top = `${y}px`;
    menuEl.focus();
  });

  function onMenuKey(e: KeyboardEvent) {
    if (!menuEl) return;
    const items = Array.from(menuEl.querySelectorAll<HTMLButtonElement>(".ctx-item:not([disabled])"));
    if (items.length === 0) return;
    const idx = items.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      items[(idx + 1) % items.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[(idx - 1 + items.length) % items.length]?.focus();
    }
  }

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* clipboard may be unavailable in the webview */
    }
  }

  // Re-arm the open menu as a confirm/cancel prompt for a destructive action.
  function confirmInMenu(label: string, action: () => void) {
    if (!menu) return true;
    menu = {
      ...menu,
      items: [
        { label, icon: "⚠", danger: true, onSelect: action },
        { label: "Cancel", onSelect: () => {} },
      ],
    };
    return true; // keep the menu open to show the confirm
  }

  // Append text to the chat composer draft (used by "post to chat" actions). Awaits a tick so
  // the composer is mounted before focusing (these can fire from the wiki/files tab).
  async function appendToDraft(text: string) {
    draft = draft ? `${draft} ${text}` : text;
    view = "chat";
    await tick();
    composerEl?.focus();
  }

  // Auto-grow the composer textarea to fit its content (bounded), so multi-line messages
  // (Shift+Enter) expand the box instead of scrolling inside one row.
  const COMPOSER_MAX = 140;
  function autoGrowComposer() {
    const el = composerEl;
    if (!el) return;
    // Collapse to zero before measuring, not to "auto": scrollHeight is max(content, clientHeight),
    // so measuring against any inherited box (a stretched flex item, or the previous larger height)
    // measures the box rather than the text and the composer never shrinks back. Zero has no such
    // floor, and the CSS min-height keeps one empty row visible.
    el.style.height = "0px";
    el.style.height = `${Math.min(el.scrollHeight, COMPOSER_MAX)}px`;
  }
  $effect(() => {
    void draft;
    if (!composerEl) return;
    autoGrowComposer();
    // Measure again after the first paint: on mount the row can be measured before fonts and the
    // final column width settle, and nothing re-measures until you type, so a bad first reading
    // is what left the box stuck at its maximum for the whole session.
    const raf = requestAnimationFrame(autoGrowComposer);
    window.addEventListener("resize", autoGrowComposer);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", autoGrowComposer);
    };
  });

  function messageMenu(m: Msg): MenuItem[] {
    const items: MenuItem[] = [
      { label: "Copy text", icon: "⧉", onSelect: () => copyText(m.text) },
      { label: "Quote in reply", icon: "❝", onSelect: () => appendToDraft(`> ${nameOf(m.author)}: ${m.text}`) },
      { divider: true },
      { label: "Copy sender fingerprint", icon: "#", onSelect: () => copyText(m.author) },
    ];
    if (m.id) {
      const top: MenuItem[] = [
        { label: "Reply", icon: "↰", onSelect: () => startReply(m) },
        { label: "React…", icon: "☺", onSelect: () => (reactionPickerFor = m.id) },
      ];
      // Owner/admin can pin/unpin any message (not in DMs).
      if (canModerate && !cur?.isDm) {
        top.push({ label: m.pinned ? "Unpin" : "Pin message", icon: "📌", onSelect: () => togglePin(m) });
      }
      top.push({ divider: true });
      items.splice(0, 0, ...top);
    }
    // Edit / delete your own messages (legacy ones without an id can't be targeted).
    if (m.author === myFp && m.id) {
      items.push({ divider: true });
      items.push({ label: "Edit", icon: "✎", onSelect: () => startEdit(m) });
      items.push({
        label: "Delete",
        icon: "🗑",
        danger: true,
        onSelect: () => confirmInMenu("Delete this message?", () => deleteMessage(m)),
      });
    } else if (m.id && canModerate && !cur?.isDm) {
      // Owner/admin moderation: remove another member's message (not in DMs).
      items.push({ divider: true });
      items.push({
        label: "Delete (moderator)",
        icon: "🗑",
        danger: true,
        onSelect: () => confirmInMenu(`Delete ${nameOf(m.author)}'s message?`, () => deleteMessage(m)),
      });
    }
    return items;
  }

  function memberMenu(m: Member): MenuItem[] {
    const isOnline = m.you || onlineMembers.has(m.fingerprint);
    const items: MenuItem[] = [
      {
        label: presenceText(m.fingerprint, m.you),
        icon: isOnline ? "●" : "○",
        disabled: true,
        onSelect: () => {},
      },
      { divider: true },
      { label: "Copy fingerprint", icon: "#", onSelect: () => copyText(m.fingerprint) },
    ];
    if (!m.you) {
      items.push({
        label: verifiedFps.has(m.fingerprint) ? "Verified: review…" : "Verify identity…",
        icon: "✓",
        onSelect: () => (verifyFor = m.fingerprint),
      });
    }
    // Add a friend in-band (only for an online member of a server: not in a DM, not yourself).
    if (!m.you && !cur?.isDm && isOnline) {
      items.push({ divider: true });
      items.push({ label: "Add friend (DM)", icon: "👋", onSelect: () => startDmWithMember(m.fingerprint) });
    }
    const r = roles[m.fingerprint] ?? "member";
    if (myRole === "owner" && !m.you && r !== "owner") {
      items.push({ divider: true });
      items.push(
        r === "admin"
          ? { label: "Demote from admin", icon: "▾", onSelect: () => setAdmin(m.fingerprint, false) }
          : { label: "Make admin", icon: "▴", onSelect: () => setAdmin(m.fingerprint, true) },
      );
      items.push({
        label: "Remove from server",
        icon: "⨯",
        danger: true,
        onSelect: () => confirmInMenu(`Remove ${nameOf(m.fingerprint)}`, () => removeMember(m.fingerprint)),
      });
    }
    return items;
  }

  function fileMenu(f: UiFile): MenuItem[] {
    const items: MenuItem[] = [
      { label: "Open details", icon: "ⓘ", onSelect: () => openFileInfo(f) },
      { label: "Download", icon: "↓", onSelect: () => downloadFile(f) },
      { label: "Post to chat", icon: "➦", onSelect: () => appendToDraft(`![${f.name}](cid:${f.cid})`) },
      { divider: true },
      { label: "Copy address (CID)", icon: "#", onSelect: () => copyText(f.cid) },
    ];
    if (myRole === "owner" || myRole === "admin") {
      items.push({
        label: "Delete file",
        icon: "🗑",
        danger: true,
        onSelect: () => confirmInMenu(`Delete ${f.name}`, () => removeFile(f)),
      });
    }
    return items;
  }

  function wikiPageMenu(p: string): MenuItem[] {
    return [
      { label: "Open page", icon: "⊞", onSelect: () => openWikiPage(p) },
      { label: "Post link to chat", icon: "➦", onSelect: () => appendToDraft(`[[${p}]]`) },
      { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[[${p}]]`) },
      {
        label: "Rename page…",
        icon: "✎",
        onSelect: () => void openWikiPage(p).then(() => startWikiRename()),
      },
      {
        label: "Delete page…",
        icon: "✕",
        onSelect: () => void openWikiPage(p).then(() => armWikiDelete()), // confirmed in the page header
      },
    ];
  }

  function serverMenu(s: ServerState): MenuItem[] {
    const items: MenuItem[] = [];
    if (s.invite) items.push({ label: "Copy invite", icon: "⧉", onSelect: () => copyText(s.invite) });
    items.push({ label: "Server settings", icon: "⚙", onSelect: () => openServerSettings(s.id) });
    items.push({ divider: true });
    items.push({
      label: "Leave server",
      icon: "⤴",
      danger: true,
      onSelect: () => confirmInMenu(`Leave ${s.name}`, () => leaveServer(s.id)),
    });
    return items;
  }

  // A reference chip's own label, without the leading icon glyph the renderer prepends.
  function chipLabel(el: HTMLElement): string {
    const c = el.cloneNode(true) as HTMLElement;
    c.querySelector(".reflink-ico")?.remove();
    return c.textContent?.trim() ?? "";
  }

  // The chat row an element sits in, if any. A card or embed can cover its whole message, so its
  // menu carries the row's own actions (reply, edit, delete) below its target-specific ones.
  function rowActions(el: HTMLElement): MenuItem[] {
    const row = el.closest("li[data-mi]") as HTMLElement | null;
    const m = row ? messages[Number(row.getAttribute("data-mi"))] : undefined;
    return m ? [{ divider: true }, ...messageMenu(m)] : [];
  }

  // Context menu on rendered rich text: copy/post a [[wikilink]], copy a :emoji:, copy an embed,
  // open/copy a file or status reference chip.
  function handleRichContext(e: MouseEvent) {
    const el = (e.target as HTMLElement | null)?.closest(
      "[data-wikilink],[data-emoji],[data-embed-cid],[data-file-cid],[data-status-id],[data-event-id]",
    ) as HTMLElement | null;
    if (!el) return;
    if (el.hasAttribute("data-file-cid")) {
      const cid = (el.getAttribute("data-file-cid") ?? "").toLowerCase();
      const label = chipLabel(el) || "file";
      openMenu(e, [
        { label: "Properties", icon: "📄", onSelect: () => openFileRef(cid) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](file:${cid})`) },
        { label: "Copy address (CID)", icon: "#", onSelect: () => copyText(cid) },
        ...rowActions(el),
      ]);
    } else if (el.hasAttribute("data-status-id")) {
      const id = el.getAttribute("data-status-id") ?? "";
      const label = chipLabel(el) || "status";
      openMenu(e, [
        { label: "Open status", icon: "⊞", onSelect: () => openStatusRef(id) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](status:${id})`) },
        ...rowActions(el),
      ]);
    } else if (el.hasAttribute("data-event-id")) {
      const id = el.getAttribute("data-event-id") ?? "";
      const label = chipLabel(el) || "event";
      openMenu(e, [
        { label: "Open event", icon: "⧗", onSelect: () => openEventRef(id) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](event:${id})`) },
        ...rowActions(el),
      ]);
    } else if (el.hasAttribute("data-wikilink")) {
      const page = el.getAttribute("data-wikilink") ?? "";
      openMenu(e, [
        { label: "Open page", icon: "⊞", onSelect: () => { view = "wiki"; openWikiPage(page); } },
        { label: "Post link to chat", icon: "➦", onSelect: () => appendToDraft(`[[${page}]]`) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[[${page}]]`) },
        ...rowActions(el),
      ]);
    } else if (el.hasAttribute("data-emoji")) {
      const code = (el.getAttribute("data-emoji") ?? "").replace(/:/g, "");
      openMenu(e, [{ label: `Copy :${code}:`, icon: "⧉", onSelect: () => copyText(`:${code}:`) }]);
    } else {
      // An inline embed (`![alt](cid:HEX)`) in chat, status or a wiki page : all three render
      // through this one context-menu path, so Properties works on every surface. Before the
      // blob resolves this is the placeholder span; after, it is the <img>/<video> itself.
      const cid = (el.getAttribute("data-embed-cid") ?? "").toLowerCase();
      const items: MenuItem[] = [];
      if (el instanceof HTMLImageElement) {
        items.push({ label: "View image", icon: "⛶", onSelect: () => openLightbox(el) });
      }
      items.push({ label: "Properties", icon: "📄", onSelect: () => openFileRef(cid) });
      const embedded = files.find((f) => f.cid === cid);
      if (embedded) items.push({ label: "Download", icon: "↓", onSelect: () => downloadFile(embedded) });
      items.push({ label: "Copy address (CID)", icon: "#", onSelect: () => copyText(cid) });
      // An image can cover its whole message row, so keep the message actions reachable here:
      // right-clicking the picture offers the same Reply/Edit/Delete as right-clicking the text.
      items.push(...rowActions(el));
      openMenu(e, items);
    }
  }
  async function refreshWiki() {
    if (activeServerId === null) return;
    try {
      const knownPages = wikiPages;
      const srv = activeServerId;
      wikiPages = await invoke<string[]>("get_wiki_pages", { server: activeServerId });
      // A page list arriving for the first time is not news, it is just the list; only pages that
      // appear against a list we already had get announced.
      if (knownPages.length) {
        for (const pg of wikiPages) {
          if (knownPages.includes(pg)) continue;
          pushTicker("wiki", `wiki:${srv}:${pg}`, pg, () => void goWikiPage(srv, pg));
        }
      }
      wikiMap = await invoke<Record<string, string>>("get_wiki_map", { server: activeServerId });
      wikiMeta = await invoke<Record<string, string>>("get_wiki_meta", { server: activeServerId });
      wikiReviewDays = await invoke<number>("get_wiki_review_days", { server: activeServerId });
      wikiPending = await invoke<UiWikiPending[]>("get_wiki_pending", { server: activeServerId });
      // Reload the open page only if it still exists and the user isn't mid-edit.
      if (activeWikiPage && !wikiDirty && wikiPages.includes(activeWikiPage)) {
        wikiBody = await invoke<string>("get_wiki_page", { server: activeServerId, name: activeWikiPage });
      }
      // Keep an open history browser current (an approval elsewhere adds a revision).
      if (showWikiHistory && activeWikiPage) {
        wikiHistory = await invoke<UiWikiRev[]>("get_wiki_history", { server: activeServerId, page: activeWikiPage });
      }
    } catch (e) {
      error = String(e);
    }
  }

  // --- wiki history browser: list revisions, diff each against its predecessor, restore ---

  async function openWikiHistory() {
    if (activeServerId === null || !activeWikiPage) return;
    try {
      wikiHistory = await invoke<UiWikiRev[]>("get_wiki_history", { server: activeServerId, page: activeWikiPage });
      wikiHistorySel = wikiHistory.length ? wikiHistory[wikiHistory.length - 1].id : "";
      showWikiHistory = true;
      wikiEdit = false;
    } catch (e) {
      toast(`Loading history failed: ${e}`, "err", 9000);
    }
  }

  async function restoreWikiRev(revId: string) {
    if (activeServerId === null || !activeWikiPage) return;
    try {
      const queued = await invoke<boolean>("restore_wiki_page", { server: activeServerId, page: activeWikiPage, rev: revId });
      showWikiHistory = false;
      await refreshWiki();
      if (queued) {
        toast(`Restore submitted for review: it publishes when approved, or automatically in ${wikiReviewDays} day${wikiReviewDays === 1 ? "" : "s"}`, "info", 6000);
      } else {
        toast(`Restored "${activeWikiPage}" to the selected revision`, "ok", 4000);
      }
    } catch (e) {
      toast(`Restore failed: ${e}`, "err", 9000);
    }
  }

  // What one revision reads as in the list: who did what, resolved at render time.
  function wikiRevLabel(kind: string): string {
    switch (kind) {
      case "approve": return "approved edit";
      case "auto": return "auto-accepted edit";
      case "reject": return "declined proposal";
      case "rollback": return "rollback";
      case "delete": return "page deleted";
      case "rename": return "renamed";
      default: return "edit";
    }
  }

  // --- admin edit review: approve / decline pending proposals, tune the review window ---

  async function approveWikiEdit(p: UiWikiPending) {
    if (activeServerId === null) return;
    try {
      await invoke("approve_wiki_edit", { server: activeServerId, id: p.id });
      toast(`Approved the edit to "${p.page}"`, "ok", 4000);
      await refreshWiki();
    } catch (e) {
      toast(`Approve failed: ${e}`, "err", 9000);
    }
  }

  async function declineWikiEdit(p: UiWikiPending) {
    if (activeServerId === null) return;
    try {
      await invoke("reject_wiki_edit", { server: activeServerId, id: p.id });
      toast(`Declined the edit to "${p.page}"`, "info", 4000);
      await refreshWiki();
    } catch (e) {
      // Declining races the deadline: once a proposal auto-accepts the backend refuses it.
      toast(`Decline failed: ${e}`, "err", 9000);
      await refreshWiki();
    }
  }

  async function setWikiReviewWindow(days: number) {
    if (activeServerId === null) return;
    try {
      await invoke("set_wiki_review_days", { server: activeServerId, days });
      toast(
        days === 0
          ? "Edits now publish immediately"
          : `Member edits now wait up to ${days} day${days === 1 ? "" : "s"} for review`,
        "ok",
        4000,
      );
      await refreshWiki();
    } catch (e) {
      toast(`Changing the review window failed: ${e}`, "err", 9000);
      await refreshWiki(); // snap the control back to the value the server actually holds
    }
  }

  // Unsaved page bodies survive navigation (in-memory, like per-channel chat drafts): following
  // a [[link]] away from a half-edited page stashes the draft; coming back restores it.
  const wikiDrafts = new Map<string, string>();
  async function openWikiPage(name: string, opts: { noRedirect?: boolean } = {}) {
    if (activeServerId === null) return;
    if (wikiDirty && activeWikiPage && activeWikiPage !== name) wikiDrafts.set(activeWikiPage, wikiBody);
    if (showInsert && insertTarget === "wiki") closeInsert();
    try {
      let body = await invoke<string>("get_wiki_page", { server: activeServerId, name });
      // Follow #REDIRECT [[Target]] pages Wikipedia-style (bounded; only to pages that exist),
      // remembering where we came from so the notice can link back to the redirect itself.
      let from = "";
      if (!opts.noRedirect) {
        for (let hops = 0; hops < 3; hops++) {
          const target = parseRedirect(body);
          if (!target || target === name || !wikiPages.includes(target)) break;
          from = from || name;
          name = target;
          body = await invoke<string>("get_wiki_page", { server: activeServerId, name });
        }
      }
      wikiRedirectedFrom = from;
      wikiBody = body;
      activeWikiPage = name;
      wikiDirty = false;
      wikiRenaming = false;
      wikiDeleteArmed = false;
      showWikiHistory = false; // the history browser is per page
      wikiReviewOpen = false; // opening a page leaves the review surface
      wikiHistorySel = "";
      revealWikiPage(name); // expand ancestor folders so the tree shows where you are
      view = "wiki";
      // Existing pages open in read mode; a not-yet-created page (e.g. a [[link]] target) in edit.
      wikiEdit = !wikiPages.includes(name);
      const draft = wikiDrafts.get(name);
      if (draft !== undefined && draft !== body) {
        wikiBody = draft;
        wikiDirty = true;
        wikiEdit = true;
      } else if (draft !== undefined) {
        wikiDrafts.delete(name); // draft caught up with the saved page
      }
    } catch (e) {
      error = String(e);
    }
  }

  // The per-page Markdown/Wikitext toggle: a shared page property (CRDT meta), not a local view.
  async function setWikiPageFormat(fmt: "md" | "wiki") {
    if (activeServerId === null || !activeWikiPage || wikiFormat === fmt) return;
    try {
      // A brand-new page (opened via a red link) has nothing saved yet; save it so the meta
      // has a page to describe.
      if (!wikiPages.includes(activeWikiPage)) await saveWikiPage();
      await invoke("set_wiki_format", { server: activeServerId, name: activeWikiPage, format: fmt });
      wikiMeta = { ...wikiMeta, [activeWikiPage]: fmt };
      toast(`Page format set to ${fmt === "wiki" ? "wikitext" : "markdown"} for everyone`, "info", 2500);
    } catch (e) {
      toast(`Setting the page format failed: ${e}`, "err", 9000);
    }
  }

  function startWikiRename() {
    wikiRenameTo = activeWikiPage;
    wikiRenaming = true;
  }
  async function commitWikiRename() {
    const to = wikiRenameTo.trim();
    wikiRenaming = false;
    if (!to || to === activeWikiPage || activeServerId === null || !activeWikiPage) return;
    try {
      await invoke("rename_wiki_page", { server: activeServerId, from: activeWikiPage, to });
      await refreshWiki();
      await openWikiPage(to, { noRedirect: true });
      toast(`Renamed to "${to}" · links to the old name go red`, "ok", 5000);
    } catch (e) {
      toast(`Rename failed: ${e}`, "err", 9000);
    }
  }

  function armWikiDelete() {
    wikiDeleteArmed = true;
    setTimeout(() => (wikiDeleteArmed = false), 4000); // disarm if not confirmed promptly
  }
  async function deleteWikiPage(name: string) {
    if (activeServerId === null) return;
    try {
      await invoke("delete_wiki_page", { server: activeServerId, name });
      wikiDeleteArmed = false;
      if (activeWikiPage === name) {
        activeWikiPage = "";
        wikiBody = "";
        wikiDirty = false;
        wikiEdit = false;
        wikiRedirectedFrom = "";
      }
      await refreshWiki();
      toast(`Deleted "${name}"`, "ok");
    } catch (e) {
      toast(`Deleting "${name}" failed: ${e}`, "err", 9000);
    }
  }

  // --- editor toolbar: format-aware syntax insertion at the textarea selection ---

  async function wikiWrap(before: string, after = before, placeholder = "text") {
    const ta = wikiTextarea;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const sel = wikiBody.slice(start, end) || placeholder;
    wikiBody = wikiBody.slice(0, start) + before + sel + after + wikiBody.slice(end);
    wikiDirty = true;
    await tick();
    ta.focus();
    ta.setSelectionRange(start + before.length, start + before.length + sel.length);
  }

  async function wikiHeading(level: number) {
    const ta = wikiTextarea;
    if (!ta) return;
    const start = ta.selectionStart;
    const ls = wikiBody.lastIndexOf("\n", start - 1) + 1;
    let le = wikiBody.indexOf("\n", start);
    if (le < 0) le = wikiBody.length;
    const line =
      wikiBody
        .slice(ls, le)
        .replace(/^#{1,6}\s*/, "")
        .replace(/^=+\s*/, "")
        .replace(/\s*=+\s*$/, "")
        .trim() || "Heading";
    const rep = wikiFormat === "wiki" ? `${"=".repeat(level)} ${line} ${"=".repeat(level)}` : `${"#".repeat(level)} ${line}`;
    wikiBody = wikiBody.slice(0, ls) + rep + wikiBody.slice(le);
    wikiDirty = true;
    await tick();
    ta.focus();
    ta.setSelectionRange(ls, ls + rep.length);
  }

  async function wikiList(ordered: boolean) {
    const ta = wikiTextarea;
    if (!ta) return;
    const start = ta.selectionStart;
    const end = ta.selectionEnd;
    const ls = wikiBody.lastIndexOf("\n", start - 1) + 1;
    const to = Math.max(end, ls);
    const seg = wikiBody.slice(ls, to) || "item";
    const mark = wikiFormat === "wiki" ? (ordered ? "# " : "* ") : ordered ? "1. " : "- ";
    const rep = seg
      .split("\n")
      .map((l) => mark + l)
      .join("\n");
    wikiBody = wikiBody.slice(0, ls) + rep + wikiBody.slice(to);
    wikiDirty = true;
    await tick();
    ta.focus();
    ta.setSelectionRange(ls, ls + rep.length);
  }

  async function insertWikiTable() {
    const ta = wikiTextarea;
    if (!ta) return;
    const tpl =
      wikiFormat === "wiki"
        ? "\n{|\n|+ Caption\n! Header !! Header\n|-\n| cell || cell\n|-\n| cell || cell\n|}\n"
        : "\n| Header | Header |\n| --- | --- |\n| cell | cell |\n| cell | cell |\n";
    const at = ta.selectionEnd;
    wikiBody = wikiBody.slice(0, at) + tpl + wikiBody.slice(at);
    wikiDirty = true;
    await tick();
    ta.focus();
    ta.setSelectionRange(at + 1, at + tpl.length - 1);
  }

  // The infobox: one block per page, so this drops the skeleton at the TOP rather than at the
  // caret, and refuses when the page already has one (the second block would stay literal text).
  async function insertWikiInfobox() {
    const ta = wikiTextarea;
    if (extractInfobox(wikiBody).box) {
      toast("This page already has an infobox: edit the block at the top", "info", 4000);
      return;
    }
    const tpl = infoboxTemplate(activeWikiPage);
    wikiBody = tpl + wikiBody;
    wikiDirty = true;
    await tick();
    if (!ta) return;
    ta.focus();
    // Select the placeholder row, which is the first thing the author will want to replace.
    const at = tpl.indexOf("| Label   = value");
    ta.setSelectionRange(at, at + "| Label   = value".length);
  }

  function onWikiEditKey(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey)) return;
    const k = e.key.toLowerCase();
    if (k === "b") {
      e.preventDefault();
      void wikiWrap(wikiFormat === "wiki" ? "'''" : "**");
    } else if (k === "i") {
      e.preventDefault();
      void wikiWrap(wikiFormat === "wiki" ? "''" : "*");
    } else if (k === "s") {
      e.preventDefault();
      void saveWikiPage();
    }
  }
  async function createWikiPage() {
    const name = newWikiPage.trim();
    if (!name || activeServerId === null) return;
    newWikiPage = "";
    try {
      // With review on, a member's save queues as a proposal; creating the page eagerly here
      // would queue an EMPTY proposal that eventually auto-creates a blank page. So members
      // under review just open the editor; their first real save becomes the proposal.
      if (!wikiPages.includes(name) && (wikiReviewDays === 0 || canModerate)) {
        await invoke("save_wiki_page", { server: activeServerId, name, body: "" });
        await refreshWiki();
      }
      await openWikiPage(name);
      wikiEdit = true;
    } catch (e) {
      error = String(e);
    }
  }

  // Insert into the wiki textarea at the caret (attach, drop, and the "+" picker all use this),
  // switching into edit mode first if needed so the insertion is visible.
  function insertIntoWikiBody(insert: string) {
    wikiEdit = true;
    const ta = wikiTextarea;
    const start = ta?.selectionStart ?? wikiBody.length;
    const end = ta?.selectionEnd ?? wikiBody.length;
    const { text, caret } = insertInto(wikiBody, start, end, insert);
    wikiBody = text;
    wikiDirty = true;
    tick().then(() => {
      const t = wikiTextarea;
      if (t) {
        t.focus();
        t.selectionStart = t.selectionEnd = caret;
      }
    });
  }

  // Embed media into the open wiki page: upload under wiki/<page>/, insert a marker at the caret.
  // Each file gets a live toast (uploading -> embedded / failed) so nothing happens silently.
  async function wikiEmbed(fileList: FileList | null) {
    if (!fileList || fileList.length === 0 || activeServerId === null || !activeWikiPage) return;
    uploading = true;
    try {
      for (const file of Array.from(fileList)) {
        const tid = toast(`Uploading ${file.name}…`, "info", 0);
        try {
          const cid = await invoke<string>("add_file", {
            server: activeServerId,
            name: file.name,
            mime: file.type || "application/octet-stream",
            path: `wiki/${activeWikiPage}`,
            data: await readBase64(file),
          });
          const alt = file.name.replace(/[[\]]/g, " ");
          insertIntoWikiBody(`![${alt}](cid:${cid})`);
          updateToast(tid, `Embedded ${file.name} · save the page to publish`, "ok", 5000);
        } catch (e) {
          updateToast(tid, `Upload of ${file.name} failed: ${e}`, "err", 9000);
        }
      }
      await refreshFiles();
    } finally {
      uploading = false;
    }
  }
  function onWikiDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    wikiEmbed(e.dataTransfer?.files ?? null);
  }
  async function saveWikiPage() {
    if (!activeWikiPage || activeServerId === null) return;
    try {
      const queued = await invoke<boolean>("save_wiki_page", { server: activeServerId, name: activeWikiPage, body: wikiBody });
      wikiDirty = false;
      wikiDrafts.delete(activeWikiPage);
      if (queued) {
        // Review mode: the save became a proposal. The page itself is unchanged until an
        // admin approves it (or the window lapses), so reload rather than pretend.
        await refreshWiki();
        wikiEdit = false;
        toast(`Edit submitted for review: it publishes when approved, or automatically in ${wikiReviewDays} day${wikiReviewDays === 1 ? "" : "s"}`, "info", 6000);
      } else {
        toast(`Saved "${activeWikiPage}"`, "ok", 2500);
      }
    } catch (e) {
      toast(`Saving "${activeWikiPage}" failed: ${e}`, "err", 9000);
    }
  }
  async function refreshInvite() {
    if (!cur || activeServerId === null) return;
    try {
      cur.invite = (await invoke<string | null>("get_invite", { server: activeServerId })) ?? "";
    } catch (e) {
      error = String(e);
    }
  }

  // Centre-crop an image file to a size×size JPEG, returned as raw base64 (no data: prefix).
  // Shared by the avatar editor and the server-icon uploader.
  async function fileToSquareJpegB64(file: File, size: number): Promise<string> {
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      await new Promise((resolve, reject) => {
        img.onload = () => resolve(null);
        img.onerror = () => reject(new Error("could not load image"));
        img.src = url;
      });
      const canvas = document.createElement("canvas");
      canvas.width = size;
      canvas.height = size;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        const scale = Math.max(size / img.width, size / img.height);
        const w = img.width * scale;
        const h = img.height * scale;
        ctx.drawImage(img, (size - w) / 2, (size - h) / 2, w, h);
      }
      return canvas.toDataURL("image/jpeg", 0.8).split(",")[1] ?? "";
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  // Sniff a stored image's format from its base64 prefix (the first magic bytes survive
  // base64 alignment). Profiles store opaque bytes, so an animated GIF or WebP a member
  // uploaded plays back as itself instead of being branded a JPEG.
  function imgSrc(b64: string): string {
    const mime = b64.startsWith("R0lGOD")
      ? "image/gif"
      : b64.startsWith("iVBOR")
        ? "image/png"
        : b64.startsWith("UklGR")
          ? "image/webp"
          : "image/jpeg";
    return `data:${mime};base64,${b64}`;
  }
  // A file's raw bytes as base64, no re-encode: keeps animation and alpha.
  async function fileToRawB64(file: File): Promise<string> {
    const buf = new Uint8Array(await file.arrayBuffer());
    let s = "";
    const CHUNK = 0x8000; // String.fromCharCode argument limit
    for (let i = 0; i < buf.length; i += CHUNK) s += String.fromCharCode(...buf.subarray(i, i + CHUNK));
    return btoa(s);
  }

  async function loadAvatar(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      // Animated formats are kept byte-for-byte when they fit the backend's 64KiB cap:
      // the canvas path would freeze them to one JPEG frame. Everything else gets the
      // usual 128px-square normalization.
      if ((file.type === "image/gif" || file.type === "image/webp") && file.size <= 64 * 1024) {
        pAvatar = await fileToRawB64(file);
      } else {
        pAvatar = await fileToSquareJpegB64(file, 128);
      }
    } catch (err) {
      error = String(err);
    }
  }

  // Downscale an image file to a wide banner JPEG (max 640px wide, aspect kept), raw base64.
  // Animated GIF/WebP under the backend's 256KiB banner cap ride byte-for-byte instead.
  async function loadBanner(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      if ((file.type === "image/gif" || file.type === "image/webp") && file.size <= 256 * 1024) {
        pBanner = await fileToRawB64(file);
        return;
      }
      const url = URL.createObjectURL(file);
      try {
        const img = new Image();
        await new Promise((resolve, reject) => {
          img.onload = () => resolve(null);
          img.onerror = () => reject(new Error("could not load image"));
          img.src = url;
        });
        const w = Math.min(640, img.width);
        const h = Math.max(1, Math.round((img.height / img.width) * w));
        const canvas = document.createElement("canvas");
        canvas.width = w;
        canvas.height = h;
        canvas.getContext("2d")?.drawImage(img, 0, 0, w, h);
        pBanner = canvas.toDataURL("image/jpeg", 0.8).split(",")[1] ?? "";
      } finally {
        URL.revokeObjectURL(url);
      }
    } catch (err) {
      error = String(err);
    }
  }

  async function saveProfile() {
    if (activeServerId === null) return;
    const prevName = myName; // my name before this edit, to detect a rail that was tracking it
    const newName = pName.trim() || displayName;
    try {
      await invoke("set_profile", {
        server: activeServerId,
        name: newName,
        color: pColor,
        font: pFont,
        effect: pEffect,
        description: pDescription.trim(),
        bubble: pBubble,
        avatar: pAvatar,
        banner: pBanner,
      });
      // Keep the server's rail label in sync ONLY when it was still tracking my name (i.e. never
      // deliberately renamed in Server settings), so the name I set shows everywhere without
      // clobbering a chosen server name like "Team Chat".
      if (cur && !cur.isDm && cur.name === prevName && newName) {
        await invoke("rename_server", { server: activeServerId, name: newName });
        cur.name = newName;
      }
      await refreshProfiles();
    } catch (e) {
      error = String(e);
    }
  }

  // Read a File as raw base64 (strips the data: prefix), for the add_file command.
  function readBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(new Error("could not read file"));
      reader.onload = () => {
        const r = reader.result;
        resolve(typeof r === "string" ? (r.split(",")[1] ?? "") : "");
      };
      reader.readAsDataURL(file);
    });
  }

  // Share a file into the Files-tab's current folder.
  async function uploadFile(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file || activeServerId === null) return;
    uploading = true;
    const tid = toast(`Sharing ${file.name}…`, "info", 0);
    try {
      await invoke("add_file", {
        server: activeServerId,
        name: file.name,
        mime: file.type || "application/octet-stream",
        path: folder,
        data: await readBase64(file),
      });
      updateToast(tid, `Shared ${file.name}`, "ok");
      await refreshFiles();
    } catch (e) {
      updateToast(tid, `Sharing ${file.name} failed: ${e}`, "err", 9000);
    } finally {
      uploading = false;
    }
  }

  // Embed media (image/video/audio) into the chat/status composer: upload under this
  // member's embed folder, then insert a `![name](cid:HEX)` marker into the draft for the
  // shared renderer to resolve inline. Non-media files are shared as plain attachments.
  async function embedFiles(target: "chat" | "status", fileList: FileList | null) {
    if (!fileList || fileList.length === 0 || activeServerId === null) return;
    uploading = true;
    try {
      for (const file of Array.from(fileList)) {
        const tid = toast(`Uploading ${file.name}…`, "info", 0);
        try {
          const cid = await invoke<string>("add_file", {
            server: activeServerId,
            name: file.name,
            mime: file.type || "application/octet-stream",
            path: myEmbedFolder,
            data: await readBase64(file),
          });
          // Brackets in the alt would break the `![alt](cid:…)` marker parse: strip them.
          const alt = file.name.replace(/[[\]]/g, " ");
          const marker = `![${alt}](cid:${cid})`;
          if (target === "chat") draft = draft ? `${draft} ${marker}` : marker;
          else statusDraft = statusDraft ? `${statusDraft} ${marker}` : marker;
          updateToast(tid, `Attached ${file.name}`, "ok");
        } catch (e) {
          updateToast(tid, `Upload of ${file.name} failed: ${e}`, "err", 9000);
        }
      }
      await refreshFiles();
    } finally {
      uploading = false;
    }
  }

  function onComposerDrop(target: "chat" | "status", e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    embedFiles(target, e.dataTransfer?.files ?? null);
  }

  // Only embeddable media types render inline; anything else is shown as a download chip.
  function safeMime(mime: string): string {
    return /^(image|video|audio)\/[a-z0-9.+-]+$/i.test(mime || "") ? mime.toLowerCase() : "";
  }

  function buildMediaEl(mime: string, url: string, alt: string, cid: string): HTMLElement {
    let el: HTMLImageElement | HTMLVideoElement | HTMLAudioElement;
    if (mime.startsWith("video/")) {
      el = document.createElement("video");
      el.controls = true;
      el.className = "embed-media";
    } else if (mime.startsWith("audio/")) {
      el = document.createElement("audio");
      el.controls = true;
      el.className = "embed-audio";
    } else {
      el = document.createElement("img");
      (el as HTMLImageElement).alt = alt;
      el.className = "embed-media embed-image";
      el.title = "Click to view full size, right-click for properties";
    }
    el.src = url;
    // Keep the address on the built element so the delegated click/context handlers can find the
    // file again; `data-resolved` stops resolveMedia treating it as a fresh placeholder to fill.
    el.setAttribute("data-embed-cid", cid);
    el.setAttribute("data-resolved", "1");
    return el;
  }

  function downloadChip(file: UiFile): HTMLElement {
    const b = document.createElement("button");
    b.className = "embed-chip";
    b.textContent = `📎 ${file.name}`;
    b.onclick = () => downloadFile(file);
    return b;
  }

  // The decrypted blob behind `cid` as a `data:` URL, from the cache or over the wire. Throws if
  // the file cannot be fetched right now (not held locally and no peer sharing it), which every
  // caller treats as "show something else" rather than as an error worth surfacing.
  async function loadBlobUrl(cid: string, mime: string, server: number): Promise<string> {
    const hit = embedCache.get(cid);
    if (hit) return hit;
    const base64 = await invoke<string>("download_file", { server, cid });
    const url = `data:${mime};base64,${base64}`;
    embedCache.set(cid, url);
    // Bound the cache (each entry is a full decrypted blob): FIFO-evict the oldest.
    if (embedCache.size > 48) {
      const oldest = embedCache.keys().next().value;
      if (oldest !== undefined) embedCache.delete(oldest);
    }
    return url;
  }

  // Blobs for markup that binds a `src` (the events tab's poster images) rather than building its
  // own element the way the embed/card resolvers do. Keyed by cid, filled in the background.
  let mediaUrls = $state<Record<string, string>>({});
  const mediaLoading = new Set<string>();
  async function ensureMedia(cid: string) {
    if (!cid || mediaUrls[cid] || mediaLoading.has(cid) || activeServerId === null) return;
    const file = files.find((f) => f.cid === cid);
    const mime = safeMime(file?.mime ?? "");
    if (!file || !mime) return; // not in the file index yet: retried when `files` updates
    mediaLoading.add(cid);
    try {
      mediaUrls = { ...mediaUrls, [cid]: await loadBlobUrl(cid, mime, activeServerId) };
    } catch {
      /* nobody is sharing it right now: the event just shows without its picture */
    } finally {
      mediaLoading.delete(cid);
    }
  }
  $effect(() => {
    const wanted = [...events.map((e) => e.image), evImage].filter(Boolean);
    void files; // a poster may only become fetchable once the index lists it
    untrack(() => {
      for (const cid of wanted) void ensureMedia(cid);
    });
  });

  // Replace `[data-embed-cid]` placeholders (from the renderer) with media built in code from
  // the group's own content-addressed blobs: never via untrusted innerHTML, so a peer's text
  // can't inject a live tag or remote URL. Only media MIME types embed; others get a chip.
  async function resolveMedia(container: HTMLElement | undefined) {
    if (!container || activeServerId === null) return;
    const server = activeServerId;
    const spans = container.querySelectorAll<HTMLElement>("[data-embed-cid]:not([data-resolved])");
    for (const span of Array.from(spans)) {
      const cid = span.getAttribute("data-embed-cid") ?? "";
      if (!cid) {
        span.setAttribute("data-resolved", "1");
        continue;
      }
      const file = files.find((f) => f.cid === cid);
      if (!file) continue; // not in the index yet: retry when `files` updates
      span.setAttribute("data-resolved", "1");
      const mime = safeMime(file.mime);
      const alt = span.getAttribute("data-alt") || file.name || "";
      if (!mime) {
        span.replaceWith(downloadChip(file));
        continue;
      }
      try {
        span.replaceWith(buildMediaEl(mime, await loadBlobUrl(cid, mime, server), alt, cid));
      } catch {
        span.replaceWith(downloadChip(file));
      }
    }
  }

  // --- image lightbox: click an inline image to fill the screen with it ----------------------
  // The viewer reuses the data: URL the embed already holds, so opening it never refetches the
  // blob; `cid` is kept so Properties/Download can look the file up in the index.
  let lightbox = $state<{ cid: string; url: string; alt: string } | null>(null);
  let lightboxZoom = $state(false); // false = fit the window, true = 1:1 and scrollable
  const lightboxFile = $derived.by(() => {
    const lb = lightbox;
    return lb ? (files.find((f) => f.cid === lb.cid) ?? null) : null;
  });

  function openLightbox(el: HTMLImageElement) {
    lightbox = {
      cid: (el.getAttribute("data-embed-cid") ?? "").toLowerCase(),
      url: el.currentSrc || el.src,
      alt: el.alt,
    };
    lightboxZoom = false;
  }
  function closeLightbox() {
    lightbox = null;
    lightboxZoom = false;
  }

  // --- link cards: a standalone reference renders as an information box ------------------------
  //
  // The renderer emits an inert chip for every in-app reference, because it is a pure function
  // with no access to this server's content. The upgrade happens here instead, the same way media
  // embeds are filled in: a chip is replaced by a card built from the file index, status feed,
  // calendar or wiki this device already holds.
  //
  // Only a chip that STANDS ALONE on its line becomes a card. A reference written into a sentence
  // is part of the prose and stays a chip; a box in the middle of a paragraph would break it.

  type CardSpec = {
    kind: "file" | "status" | "event" | "wiki";
    icon: string;
    kicker: string; // the small line above the title: what kind of thing this is, and when
    title: string;
    sub?: string;
    body?: string;
    thumb?: string; // cid of an image to show alongside the text
    missing?: boolean; // the target does not exist (yet): a red link, wiki-style
  };

  /** The first `![alt](cid:HEX)` embed in a body, so a card can show what the page/post shows. */
  function firstEmbedCid(text: string): string {
    return (/!\[[^\]\n]*\]\(cid:([0-9a-fA-F]{2,64})\)/.exec(text ?? "")?.[1] ?? "").toLowerCase();
  }

  // Structure that is already a list of links, a table cell or a heading: a card there would
  // wreck the layout the author chose, so those keep their inline chips however they are written.
  const NEVER_A_CARD = /^(LI|TD|TH|DT|DD|H1|H2|H3|H4)$/;

  /** Whether `el` is the only thing on its line: blank text, `<br>`s and the edited tag aside. */
  function standsAlone(el: HTMLElement): boolean {
    const parent = el.parentElement;
    if (!parent || NEVER_A_CARD.test(parent.tagName)) return false;
    const sibs = Array.from(parent.childNodes);
    const at = sibs.indexOf(el);
    if (at < 0) return false;
    // Comment nodes matter here: Svelte anchors every block it renders with one, so a message
    // body is `<!>chip<!>` even when the chip is the only thing in it. Treating those as content
    // is what kept chat references from ever unfurling.
    const ignorable = (nd: ChildNode) =>
      nd.nodeType === Node.COMMENT_NODE ||
      (nd.nodeType === Node.TEXT_NODE && !(nd.textContent ?? "").trim()) ||
      (nd as HTMLElement).classList?.contains("edited-tag");
    for (let i = at - 1; i >= 0 && sibs[i].nodeName !== "BR"; i--) if (!ignorable(sibs[i])) return false;
    for (let i = at + 1; i < sibs.length && sibs[i].nodeName !== "BR"; i++) if (!ignorable(sibs[i])) return false;
    return true;
  }

  function fileCardSpec(cid: string): CardSpec | null {
    const f = files.find((x) => x.cid === cid);
    if (!f) return null; // not in the index on this device: leave the chip, retry next pass
    const where = f.path ? ` in ${f.path}` : "";
    return {
      kind: "file",
      icon: "\u{1F4C4}",
      kicker: `File · ${fmtSize(f.size)}${f.mime ? ` · ${f.mime}` : ""}`,
      title: f.name,
      sub: `shared by ${nameOf(f.author)}${where}`,
      thumb: cid,
    };
  }

  function statusCardSpec(id: string): CardSpec | null {
    const post = statuses.find((x) => x.id === id);
    if (!post) return null;
    return {
      kind: "status",
      icon: "◈",
      kicker: `Status · ${relDay(post.ts, Date.now())}`,
      title: nameOf(post.author),
      body: plainSummary(post.text, 200) || "(no text)",
      thumb: firstEmbedCid(post.text),
    };
  }

  function eventCardSpec(id: string): CardSpec | null {
    const ev = events.find((x) => x.id === id);
    if (!ev) return null;
    const now = Date.now();
    const when = ev.start_ts <= now && eventLive(ev, now) ? "happening now" : relDay(ev.start_ts, now);
    return {
      kind: "event",
      icon: "⧗",
      kicker: `Event · ${when}`,
      title: ev.title,
      sub: `${fmtEventWhen(ev)} · by ${nameOf(ev.author)}`,
      body: plainSummary(ev.body, 160),
      thumb: ev.image || firstEmbedCid(ev.body),
    };
  }

  function wikiCardSpec(page: string): CardSpec | null {
    const exists = wikiPages.includes(page);
    const body = wikiMap[page] ?? "";
    // A page nobody has written yet is still worth a card: it says so, and clicking it starts one.
    if (!exists && !body) {
      return { kind: "wiki", icon: "⊞", kicker: "Wiki page", title: page, sub: "not created yet", missing: true };
    }
    const target = parseRedirect(body);
    return {
      kind: "wiki",
      icon: "⊞",
      kicker: "Wiki page",
      title: page,
      sub: target ? `redirects to ${target}` : undefined,
      body: target ? undefined : plainSummary(body, 220) || "(empty page)",
      thumb: firstEmbedCid(body),
    };
  }

  /** Hang an image on a card once its blob arrives; a card without one just reads as text. */
  function attachCardThumb(card: HTMLElement, cid: string, server: number) {
    const mime = safeMime(files.find((f) => f.cid === cid)?.mime ?? "");
    if (!mime.startsWith("image/")) return;
    void loadBlobUrl(cid, mime, server)
      .then((url) => {
        if (!card.isConnected) return; // the surface re-rendered while the blob was in flight
        const img = document.createElement("img");
        img.className = "ref-card-thumb";
        img.src = url;
        img.alt = "";
        card.appendChild(img);
        card.classList.add("has-thumb");
      })
      .catch(() => {
        /* nobody is sharing it: no picture, same card */
      });
  }

  /**
   * Build the card element. Every value is set as text, never as markup, and the card keeps the
   * chip's own `data-` attribute so the existing click and context-menu handlers still address it.
   */
  function buildCard(spec: CardSpec, attr: string, value: string, server: number): HTMLElement {
    const card = document.createElement("a");
    card.className = `ref-card ${spec.kind}-card${spec.missing ? " missing" : ""}`;
    card.setAttribute(attr, value);
    card.setAttribute("role", "button");
    card.tabIndex = 0;
    const text = document.createElement("span");
    text.className = "ref-card-text";
    const kicker = document.createElement("span");
    kicker.className = "ref-card-kicker";
    const ico = document.createElement("span");
    ico.className = "ref-card-ico";
    ico.setAttribute("aria-hidden", "true");
    ico.textContent = spec.icon;
    kicker.append(ico, document.createTextNode(spec.kicker));
    text.appendChild(kicker);
    for (const [cls, val] of [
      ["ref-card-title", spec.title],
      ["ref-card-sub", spec.sub],
      ["ref-card-body", spec.body],
    ] as const) {
      if (!val) continue;
      const line = document.createElement("span");
      line.className = cls;
      line.textContent = val;
      text.appendChild(line);
    }
    card.appendChild(text);
    if (spec.thumb) attachCardThumb(card, spec.thumb, server);
    return card;
  }

  const CARD_CHIPS =
    "a.reflink[data-file-cid],a.reflink[data-status-id],a.reflink[data-event-id],a.wikilink[data-wikilink]";

  function resolveRefCards(container: HTMLElement | undefined) {
    if (!container || activeServerId === null) return;
    const server = activeServerId;
    for (const el of Array.from(container.querySelectorAll<HTMLElement>(CARD_CHIPS))) {
      if (!standsAlone(el)) continue;
      const cid = (el.getAttribute("data-file-cid") ?? "").toLowerCase();
      const sid = el.getAttribute("data-status-id") ?? "";
      const eid = el.getAttribute("data-event-id") ?? "";
      const page = el.getAttribute("data-wikilink") ?? "";
      const [spec, attr, value] = cid
        ? [fileCardSpec(cid), "data-file-cid", cid]
        : sid
          ? [statusCardSpec(sid), "data-status-id", sid]
          : eid
            ? [eventCardSpec(eid), "data-event-id", eid]
            : [wikiCardSpec(page), "data-wikilink", page];
      // No spec means the target is not loaded on this device yet (a status feed not fetched, an
      // event from a calendar still syncing): keep the chip, and try again when that state lands.
      if (spec) el.replaceWith(buildCard(spec, attr, value, server));
    }
  }

  function enterFolder(seg: string) {
    folder = folder === "" ? seg : `${folder}/${seg}`;
  }
  function gotoCrumb(i: number) {
    folder = breadcrumbs.slice(0, i + 1).join("/");
  }

  async function downloadFile(f: UiFile) {
    if (activeServerId === null) return;
    const key = dlKey(activeServerId, f.cid);
    downloads[key] = {
      server: activeServerId,
      cid: f.cid,
      name: f.name,
      author: f.author,
      status: "queued",
      progress: 0,
      ts: Date.now(),
    };
    try {
      const base64 = await invoke<string>("download_file", { server: activeServerId, cid: f.cid });
      const a = document.createElement("a");
      a.href = `data:${f.mime || "application/octet-stream"};base64,${base64}`;
      a.download = f.name;
      document.body.appendChild(a);
      a.click();
      a.remove();
      if (downloads[key]) downloads[key].status = "done";
      refreshFiles(); // the file's chunks are now held locally: update its availability
    } catch (e) {
      error = String(e);
      if (downloads[key]) downloads[key].status = "failed";
    }
  }

  // The file info pane: click a file to inspect it (preview, availability, uploader, delete).
  let fileInfo = $state<UiFile | null>(null);
  let fileInfoAvail = $state<boolean | null>(null); // null = still checking
  let fileInfoPreview = $state<string>(""); // a data: URL for image/video/audio previews
  let fileInfoPreviewError = $state(false); // the preview fetch failed (so "Loading…" doesn't hang)
  let fileInfoBusy = $state(false);
  let confirmDeleteCid = $state(""); // two-click delete confirm in the info pane
  let fileInfoUsage = $state<UiFileUsage | null>(null); // null = still checking
  let fileInfoExpiryBusy = $state(false);

  // The default circulation lifetime, mirroring `catcoms_app::FILE_EXPIRY_DEFAULT_MS`. Used only
  // to compute the deadline "Restore 30-day expiry" writes back; new shares are stamped by the
  // backend off its injected clock.
  const FILE_EXPIRY_DEFAULT_MS = 30 * 24 * 60 * 60 * 1000;
  const RELATIVE_FMT = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  /** An absolute day in the viewer's locale, e.g. "Sep 14, 2026". */
  function fmtDay(ms: number): string {
    return new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  }
  /** A coarse signed distance from `now`, e.g. "in 30 days" / "3 days ago" / "tomorrow". */
  function relDay(ms: number, now: number): string {
    const delta = ms - now;
    const abs = Math.abs(delta);
    if (abs < 3_600_000) return RELATIVE_FMT.format(Math.round(delta / 60_000), "minute");
    if (abs < 86_400_000) return RELATIVE_FMT.format(Math.round(delta / 3_600_000), "hour");
    if (abs < 45 * 86_400_000) return RELATIVE_FMT.format(Math.round(delta / 86_400_000), "day");
    return RELATIVE_FMT.format(Math.round(delta / (30 * 86_400_000)), "month");
  }

  // The "Circulates until" row. Precedence: a wiki embed pins the file regardless of any recorded
  // deadline; then an explicit keep-forever; then a date; then a legacy listing that never
  // recorded one. `kind` drives the styling, `text` is the row's wording.
  type ExpiryView = { kind: "pinned" | "forever" | "date" | "unknown"; text: string };
  let fileInfoExpiry = $derived.by((): ExpiryView => {
    const f = fileInfo;
    if (!f) return { kind: "unknown", text: "" };
    if (isPinned(f.cid) || fileInfoUsage?.pinned)
      return { kind: "pinned", text: "pinned: embedded in the wiki, never drops from sharing" };
    if (!f.expires_known) return { kind: "unknown", text: "not recorded (older share)" };
    if (f.expires === null) return { kind: "forever", text: "forever" };
    return { kind: "date", text: `${fmtDay(f.expires)} · ${relDay(f.expires, nowTick)}` };
  });
  // Expiry is per listing and the gate is uploader / owner / admin, matching the backend.
  let canSetExpiry = $derived(
    !!fileInfo && (fileInfo.author === myFp || myRole === "owner" || myRole === "admin")
  );
  let keptForever = $derived(!!fileInfo && fileInfo.expires_known && fileInfo.expires === null);

  /** Toggle this listing between "keep forever" and a fresh 30-day circulation window. */
  async function toggleKeepForever() {
    if (activeServerId === null || !fileInfo) return;
    const f = fileInfo;
    const forever = !keptForever; // the state we're moving TO
    const expires = forever ? null : Date.now() + FILE_EXPIRY_DEFAULT_MS;
    fileInfoExpiryBusy = true;
    const tid = toast(forever ? "Keeping forever…" : "Restoring 30-day expiry…", "info", 0);
    try {
      await invoke("set_file_expiry", { server: activeServerId, cid: f.cid, path: f.path, expires });
      await refreshFiles();
      // Re-point the open pane at the refreshed listing so the row repaints.
      const fresh = files.find((x) => x.cid === f.cid && x.path === f.path);
      if (fresh && fileInfo?.cid === f.cid) fileInfo = fresh;
      updateToast(
        tid,
        forever
          ? `${f.name} will be kept in circulation forever`
          : `${f.name} circulates until ${fmtDay(expires as number)}`,
        "ok"
      );
    } catch (e) {
      updateToast(tid, `Couldn't change the expiry: ${e}`, "err", 9000);
    } finally {
      fileInfoExpiryBusy = false;
    }
  }
  // Tracked downloads keyed by file cid, for the Downloads tab + the file-info progress bar. Driven
  // by 'download-progress' events (per-chunk) from the actor. Only EXPLICIT downloads (the Download
  // button) are tracked here: background embed/preview fetches emit progress but create no entry.
  type DownloadInfo = {
    server: number;
    cid: string;
    name: string;
    author: string; // the uploader (the file's source)
    provider?: string; // the live serving peer's fingerprint, when bytes came over the network
    status: "queued" | "downloading" | "done" | "failed";
    progress: number; // 0..1
    ts: number;
  };
  // Keyed by `${server}:${cid}` so a download is scoped to its server (the same content cid can
  // exist on two servers, and switching servers must not show the other's transfers).
  let downloads = $state<Record<string, DownloadInfo>>({});
  const dlKey = (server: number, cid: string) => `${server}:${cid}`;
  // The active server's downloads, newest first.
  let downloadList = $derived(
    Object.values(downloads)
      .filter((d) => d.server === activeServerId)
      .sort((a, b) => b.ts - a.ts)
  );
  let activeDownloads = $derived(
    downloadList.filter((d) => d.status === "queued" || d.status === "downloading").length
  );
  function clearFinishedDownloads() {
    for (const [k, d] of Object.entries(downloads)) {
      if (d.server === activeServerId && (d.status === "done" || d.status === "failed"))
        delete downloads[k];
    }
  }
  // Advisory eclipse hint for the active server (the node may be isolated: verify a member out of
  // band). Never gates anything; driven by 'eclipse-changed'. Reset when switching servers.
  let eclipseCaution = $state(false);

  async function openFileInfo(f: UiFile) {
    if (activeServerId === null) return;
    fileInfo = f;
    fileInfoAvail = null;
    fileInfoPreview = "";
    fileInfoPreviewError = false;
    confirmDeleteCid = "";
    fileInfoUsage = null;
    const id = activeServerId;
    // Where the file is used (wiki pages + status/chat counts). Async like the availability row,
    // and guarded against the pane being switched while the scan is in flight.
    invoke<UiFileUsage>("get_file_usage", { server: id, cid: f.cid })
      .then((u) => {
        if (fileInfo?.cid === f.cid) fileInfoUsage = u;
      })
      .catch(() => {
        if (fileInfo?.cid === f.cid) fileInfoUsage = { wiki_pages: [], status_count: 0, chat_count: 0, event_count: 0, pinned: false };
      });
    // Report whether the blob is held locally *before* a preview fetch would pull it. Guard the
    // assignment against a race where the user clicked another file while this was in flight.
    try {
      const ok = await invoke<boolean>("file_available", { server: id, cid: f.cid });
      if (fileInfo?.cid === f.cid) fileInfoAvail = ok;
    } catch {
      if (fileInfo?.cid === f.cid) fileInfoAvail = false;
    }
    // Fetch an inline preview for media types (bounded by the backend's max file size).
    if (safeMime(f.mime) && fileInfo?.cid === f.cid) {
      try {
        const base64 = await invoke<string>("download_file", { server: id, cid: f.cid });
        // Guard against a race where the pane was closed/switched while fetching.
        if (fileInfo?.cid === f.cid) fileInfoPreview = `data:${f.mime};base64,${base64}`;
      } catch {
        // The fetch failed (not held locally + no peer sharing it): surface that instead of
        // leaving "Loading preview…" up forever.
        if (fileInfo?.cid === f.cid) fileInfoPreviewError = true;
      }
    }
  }

  function closeFileInfo() {
    fileInfo = null;
    fileInfoPreview = "";
    fileInfoPreviewError = false;
    fileInfoAvail = null;
    confirmDeleteCid = "";
    fileInfoUsage = null;
  }

  /** From Properties → "Used in": close the pane and open the wiki page that embeds the file. */
  async function openUsageWikiPage(name: string) {
    closeFileInfo();
    switchView("wiki");
    await openWikiPage(name);
  }

  async function removeFile(f: UiFile) {
    if (activeServerId === null) return;
    fileInfoBusy = true;
    try {
      await invoke("delete_file", { server: activeServerId, cid: f.cid });
      closeFileInfo();
      await refreshFiles();
    } catch (e) {
      error = String(e);
    } finally {
      fileInfoBusy = false;
    }
  }

  const previewKind = $derived.by(() => {
    const m = safeMime(fileInfo?.mime ?? "");
    if (m.startsWith("image/")) return "image";
    if (m.startsWith("video/")) return "video";
    if (m.startsWith("audio/")) return "audio";
    return "";
  });

  // Index the loaded messages by id once per change, so reply-parent lookups (the quote on every
  // reply + the composer banner) are O(1) instead of a linear scan per render.
  let msgById = $derived(new Map(messages.map((m) => [m.id, m] as const)));
  // Reply target: the id of the message the composer is replying to ("" = a plain message).
  let replyingTo = $state("");
  let replyTarget = $derived(replyingTo ? msgById.get(replyingTo) : undefined);
  // Briefly flash a message you jumped to (e.g. from a reply quote), keyed by id so it survives
  // index shifts; cleared after the pulse.
  let flashId = $state("");
  function jumpToMessageId(id: string) {
    const idx = messages.findIndex((m) => m.id === id);
    if (idx < 0) return; // parent not in the loaded list (deleted / scrolled out of history)
    scrollToMatch(idx);
    flashId = id;
    setTimeout(() => {
      if (flashId === id) flashId = "";
    }, 1300);
  }
  // Reply counts per parent message id (for the "N replies" thread affordance).
  let replyCounts = $derived.by(() => {
    const m = new Map<string, number>();
    for (const msg of messages) {
      if (msg.reply_to) m.set(msg.reply_to, (m.get(msg.reply_to) ?? 0) + 1);
    }
    return m;
  });
  function jumpToFirstReply(parentId: string) {
    const first = messages.find((m) => m.reply_to === parentId);
    if (first) jumpToMessageId(first.id);
  }
  function startReply(m: Msg) {
    replyingTo = m.id;
    composerEl?.focus();
  }
  function cancelReply() {
    replyingTo = "";
  }
  function msgSnippet(text: string, n = 70): string {
    const t = text.replace(/\s+/g, " ").trim();
    return t.length > n ? t.slice(0, n) + "…" : t;
  }

  // --- toasts: visible feedback for otherwise-silent work (uploads, saves, renames) --------------
  type Toast = { id: number; kind: "info" | "ok" | "err"; text: string };
  let toasts = $state<Toast[]>([]);
  let toastSeq = 0;
  const toastTimers = new Map<number, ReturnType<typeof setTimeout>>();
  /** Show a toast; `ms` 0 = sticky until updated/dismissed. Returns an id for updateToast. */
  function toast(text: string, kind: Toast["kind"] = "info", ms = 3500): number {
    const id = ++toastSeq;
    toasts = [...toasts, { id, kind, text }];
    if (ms > 0) toastTimers.set(id, setTimeout(() => dismissToast(id), ms));
    return id;
  }
  /** Morph an existing toast in place (e.g. "Uploading…" -> "Embedded"). */
  function updateToast(id: number, text: string, kind: Toast["kind"], ms = 3500) {
    const timer = toastTimers.get(id);
    if (timer) clearTimeout(timer);
    toastTimers.delete(id);
    if (!toasts.some((t) => t.id === id)) {
      toast(text, kind, ms);
      return;
    }
    toasts = toasts.map((t) => (t.id === id ? { ...t, text, kind } : t));
    if (ms > 0) toastTimers.set(id, setTimeout(() => dismissToast(id), ms));
  }
  function dismissToast(id: number) {
    const timer = toastTimers.get(id);
    if (timer) clearTimeout(timer);
    toastTimers.delete(id);
    toasts = toasts.filter((t) => t.id !== id);
  }

  // --- "+" insert picker: link/embed this server's own content into the message -----------------
  // Everything the group already holds is addressable from the composer: a fileshare file (inline
  // embed for media, a link chip otherwise), one of YOUR status posts, or a wiki page. Each inserts
  // a marker the shared renderer resolves: nothing here leaves the group or touches the network.
  type InsertTab = "files" | "status" | "wiki" | "events";
  let showInsert = $state(false);
  let insertTarget = $state<"chat" | "wiki">("chat"); // which editor the picker inserts into
  let insertTab = $state<InsertTab>("files");
  let insertQuery = $state("");
  let insertInput = $state<HTMLInputElement | undefined>(undefined);
  let insertLoading = $state(false); // the open-time refresh is in flight

  // The custom-emoji folder has its own picker (the 🐱 button), so it's noise here.
  let insertFiles = $derived.by(() => {
    const q = insertQuery.trim().toLowerCase();
    return files
      .filter((f) => f.path !== "emoji" && !f.path.startsWith("emoji/"))
      .filter((f) => !q || f.name.toLowerCase().includes(q) || f.path.toLowerCase().includes(q))
      .slice(0, 80);
  });
  // "recent statuses we posted": your own posts, newest first. A status with no id predates the
  // stable-id slice and can't be addressed, so it's skipped rather than offered and broken.
  let insertStatuses = $derived.by(() => {
    const q = insertQuery.trim().toLowerCase();
    return statuses
      .filter((s) => s.author === myFp && s.id && (!q || s.text.toLowerCase().includes(q)))
      .slice()
      .reverse()
      .slice(0, 50);
  });
  let insertWikiPages = $derived(
    wikiPages.filter((p) => !insertQuery.trim() || p.toLowerCase().includes(insertQuery.trim().toLowerCase())).slice(0, 80),
  );
  // Upcoming first (soonest at the top), then recent past: anything addressable.
  let insertEvents = $derived.by(() => {
    const q = insertQuery.trim().toLowerCase();
    const hit = (e: UiEvent) => !q || e.title.toLowerCase().includes(q);
    return [...upcomingEvents.filter(hit), ...pastEvents.filter(hit)].slice(0, 50);
  });
  let insertCount = $derived(
    insertTab === "files"
      ? insertFiles.length
      : insertTab === "status"
        ? insertStatuses.length
        : insertTab === "events"
          ? insertEvents.length
          : insertWikiPages.length,
  );

  async function toggleInsert(target: "chat" | "wiki" = "chat") {
    if (showInsert && insertTarget === target) {
      closeInsert();
      return;
    }
    insertTarget = target;
    showInsert = true;
    showEmoji = false;
    insertQuery = "";
    // A DM has no Status/Wiki tab, so don't strand the picker on one you can't see.
    const dmOnly = !!cur?.isDm;
    if (dmOnly) insertTab = "files";
    await tick();
    insertInput?.focus();
    // The picker opens from the Chat tab, where these lists may never have been loaded (each is
    // otherwise fetched when its own tab is opened), so pull them now. These are actor round-trips
    // that can lag behind a busy server, so say so rather than showing a bare "nothing here".
    insertLoading = true;
    try {
      await Promise.all(dmOnly ? [refreshFiles()] : [refreshFiles(), refreshStatuses(), refreshWiki(), refreshEvents()]);
    } finally {
      insertLoading = false;
    }
  }
  function closeInsert() {
    showInsert = false;
    insertQuery = "";
    insertLoading = false;
  }

  // Insert at the target editor's caret (mirrors pickMention), leaving the caret just after it.
  // The string maths lives in refs.ts so it can be unit-tested away from the DOM. The picker
  // serves two editors: the chat composer and the wiki page editor (insertTarget).
  function insertAtCaret(insert: string) {
    if (insertTarget === "wiki") {
      insertIntoWikiBody(insert);
      return;
    }
    const start = composerEl?.selectionStart ?? draft.length;
    const end = composerEl?.selectionEnd ?? draft.length;
    const { text, caret } = insertInto(draft, start, end, insert);
    draft = text;
    queueMicrotask(() => {
      if (composerEl) {
        composerEl.focus();
        composerEl.selectionStart = composerEl.selectionEnd = caret;
      }
    });
  }
  function insertFileRef(f: UiFile, asEmbed: boolean) {
    insertAtCaret(fileMarker(f.name, f.cid, asEmbed));
    closeInsert();
  }
  function insertStatusRef(s: Msg) {
    insertAtCaret(statusMarker(s.text, s.id));
    closeInsert();
  }
  function insertWikiRef(page: string) {
    insertAtCaret(wikiMarker(page));
    closeInsert();
  }
  function insertEventRef(e: UiEvent) {
    insertAtCaret(eventMarker(e.title, e.id));
    closeInsert();
  }
  // An event chip in a message: jump to the Events surface and briefly flash the event.
  let flashEventId = $state("");
  function openEventRef(id: string) {
    switchView("events");
    flashEventId = id;
    setTimeout(() => {
      if (flashEventId === id) flashEventId = "";
    }, 1600);
  }

  // Follow a `[…](file:CID)` chip: the index may be stale (the file was added after this tab last
  // loaded), so refresh once before giving up.
  async function openFileRef(cid: string) {
    let f = files.find((x) => x.cid.toLowerCase() === cid);
    if (!f) {
      await refreshFiles();
      f = files.find((x) => x.cid.toLowerCase() === cid);
    }
    if (f) openFileInfo(f);
    else toast("That file is no longer in this server's file index.", "err", 6000);
  }
  // Follow a `[…](status:ID)` chip: switch to the Status tab and flash the post.
  let flashStatusId = $state("");
  async function openStatusRef(id: string) {
    if (!id) return;
    view = "status";
    if (!statuses.some((s) => s.id === id)) await refreshStatuses();
    await tick();
    if (!statuses.some((s) => s.id === id)) {
      error = "That status post is no longer in this server's feed.";
      return;
    }
    statusEl?.querySelector(`[data-sid="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
    flashStatusId = id;
    setTimeout(() => {
      if (flashStatusId === id) flashStatusId = "";
    }, 1300);
  }

  // @-mention autocomplete: when the caret sits right after an "@partial", offer matching members;
  // selecting one inserts the `@[Name]` marker the renderer highlights.
  let mentionQuery = $state<string | null>(null);
  let mentionStart = $state(0); // index of the '@' in the draft
  let mentionIdx = $state(0); // highlighted candidate
  let mentionCandidates = $derived.by(() => {
    if (mentionQuery === null) return [] as { fp: string; name: string }[];
    const q = mentionQuery.toLowerCase();
    return roster
      .map((r) => ({ fp: r.fingerprint, name: nameOf(r.fingerprint) }))
      .filter((c) => c.name.toLowerCase().includes(q))
      .slice(0, 6);
  });
  function onComposerInput(e: Event & { currentTarget: HTMLTextAreaElement }) {
    const caret = e.currentTarget.selectionStart ?? draft.length;
    const m = /@([^\s@[\]]{0,30})$/.exec(draft.slice(0, caret));
    if (m) {
      mentionStart = caret - m[0].length;
      mentionQuery = m[1];
      mentionIdx = 0;
    } else {
      mentionQuery = null;
    }
  }
  // Normalize a display name into the form carried by an `@[Name]` marker: no `[`/`]`/newline (which
  // would break the bracketed marker / the tokenizer regex) and bounded to the tokenizer's 40 chars.
  // Insertion and detection both go through this, so a mention round-trips even for odd names.
  function mentionName(name: string): string {
    return name
      .replace(/[[\]\n]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 40);
  }
  function pickMention(c: { fp: string; name: string }) {
    const caret = composerEl?.selectionStart ?? draft.length;
    const before = draft.slice(0, mentionStart);
    const insert = `@[${mentionName(c.name)}] `;
    draft = before + insert + draft.slice(caret);
    mentionQuery = null;
    const pos = before.length + insert.length;
    queueMicrotask(() => {
      if (composerEl) {
        composerEl.focus();
        composerEl.selectionStart = composerEl.selectionEnd = pos;
      }
    });
  }
  function onComposerKeydown(e: KeyboardEvent) {
    if (mentionQuery !== null && mentionCandidates.length) {
      const n = mentionCandidates.length;
      if (e.key === "ArrowDown") { e.preventDefault(); mentionIdx = (mentionIdx + 1) % n; return; }
      if (e.key === "ArrowUp") { e.preventDefault(); mentionIdx = (mentionIdx - 1 + n) % n; return; }
      if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); pickMention(mentionCandidates[mentionIdx]); return; }
      if (e.key === "Escape") { e.preventDefault(); mentionQuery = null; return; }
    }
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey) {
      if (e.key === "b") { e.preventDefault(); e.stopPropagation(); wrapSelection("**"); return; }
      if (e.key === "i") { e.preventDefault(); e.stopPropagation(); wrapSelection("*"); return; }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); }
  }

  // Mentions/replies aimed at me. `mentionChannels` holds the active server's channels with an
  // unseen message that @-mentions me or replies to one of my messages (drives the sidebar badge).
  let mentionChannels = $state<Set<string>>(new Set());
  // Best-effort, name-based: collisions (two members with the same display name both match) and
  // renames (old markers orphan) are accepted for advisory metadata: the `@[Name]` wire form keeps
  // the renderer member-list-free. `myMentionName` matches what `pickMention` would have inserted.
  function mentionsMe(text: string): boolean {
    return !!myMentionName && text.includes(`@[${myMentionName}]`);
  }
  // Does `msgs` contain a message newer than the channel's read mark that targets me (and isn't
  // mine)? Used to flag a channel and to decide whether an arrival deserves a mention chime.
  function targetsMe(channel: string, msgs: Msg[]): boolean {
    if (!myFp) return false;
    const seen = readMarks[`${activeServerId}:${channel}`] ?? 0;
    const byId = new Map(msgs.map((m) => [m.id, m] as const));
    return msgs.some(
      (m) =>
        m.ts > seen &&
        m.author !== myFp &&
        (mentionsMe(m.text) || (!!m.reply_to && byId.get(m.reply_to)?.author === myFp)),
    );
  }

  // Cross-server inbox: the backend scans every server's channels for messages addressed to me.
  async function loadInbox() {
    inboxLoading = true;
    try {
      inboxItems = await invoke<InboxEntry[]>("get_inbox");
    } catch (e) {
      error = String(e);
    } finally {
      inboxLoading = false;
    }
  }
  let inboxTimer: ReturnType<typeof setTimeout> | undefined;
  function scheduleInboxReload() {
    clearTimeout(inboxTimer);
    inboxTimer = setTimeout(loadInbox, 1500); // debounce the cross-server scan
  }
  // An inbox entry is "unseen" until you've read past it in that channel (the same read marks that
  // drive jump-to-unread); resolved against the entry's own server, not the active one.
  function inboxUnseen(it: InboxEntry): boolean {
    return it.ts > (readMarks[`${it.server}:${it.channel}`] ?? 0);
  }
  let inboxUnseenCount = $derived(inboxItems.filter(inboxUnseen).length);
  // The entry's channel name, resolved from the server's known channel list (names are a UI concern).
  function inboxChannelName(it: InboxEntry): string {
    return servers.find((s) => s.id === it.server)?.channels.find((c) => c.id === it.channel)?.name ?? "channel";
  }
  function openInbox() {
    inboxView = true;
    dmHome = false;
    loadInbox();
  }
  // Open the server + channel an inbox entry points at and scroll to the message.
  async function jumpToInbox(it: InboxEntry) {
    // The server hop, the channel hop and the scroll are one move, so Back returns to the inbox
    // rather than walking you back out of it a step at a time.
    navStepStart();
    try {
      inboxView = false;
      if (it.server !== activeServerId) await switchServer(it.server);
      view = "chat";
      // The entry's channel was scanned (so it's open in the backend) but may not be in this UI's
      // sidebar list: register it so it renders + selects, then switch to it.
      if (cur && !cur.channels.some((c) => c.id === it.channel)) {
        cur.channels = [...cur.channels, { id: it.channel, name: inboxChannelName(it) }];
      }
      if (cur && cur.active !== it.channel) switchTo(it.channel);
      await refresh();
      jumpToMessageId(it.message_id);
    } finally {
      navStepEnd();
    }
  }

  // --- Voice calls (full-mesh WebRTC; E2E via authenticated signalling + DTLS-SRTP) -------------
  // Each pair of participants connects directly (no server in the media path), so SRTP is end-to-end;
  // the SDP/ICE is exchanged over the members-only, signed KIND_CALL_SIGNAL push, so the DTLS
  // fingerprints can't be MITM'd. A future MLS-keyed frame layer (SFrame) is only needed for an SFU.
  // polite/makingOffer/ignoreOffer implement the "perfect negotiation" pattern: with video, either
  // end can (re)negotiate at any moment, and two simultaneous offers must resolve deterministically.
  // The lexicographically-smaller fingerprint is the polite end and yields on collision.
  type CallPeer = {
    fp: string;
    pc: RTCPeerConnection;
    dc: RTCDataChannel | null;
    polite: boolean;
    makingOffer: boolean;
    ignoreOffer: boolean;
  };
  let inCall = $state(false);
  let callMuted = $state(false);
  let callParticipants = $state<string[]>([]); // peer fingerprints, for the call UI
  let callPeerStates = $state<Record<string, string>>({}); // fp -> RTCPeerConnectionState
  // A voice room is per-CHANNEL: the channel id doubles as the call id (for signalling + the media
  // key). You join a channel's room; others see it via presence (below) and join the same room.
  let callChannel = $state(""); // the channel id of my active voice room ("" = not in a call)
  let callChannelName = $state(""); // for the call bar
  let callServer: number | null = null; // the server the room is on
  let localStream: MediaStream | null = null;
  const callPeers: Record<string, CallPeer> = {};

  // Voice-room presence: `${server}:${channel}` -> { fp: lastSeenMs }, from periodic pings members in
  // a room broadcast. Drives the per-channel "in voice" indicators + the room-active notification.
  let voiceRooms = $state<Record<string, Record<string, number>>>({});
  let voiceAlert = $state<{ server: number; channel: string; name: string } | null>(null);
  let pingTimer: ReturnType<typeof setInterval> | undefined;
  const alertedRooms = new Set<string>(); // rooms already notified, so a call doesn't re-ring
  const VOICE_STALE_MS = 14000; // a presence entry older than this is dropped
  function roomKey(server: number, channel: string) {
    return `${server}:${channel}`;
  }
  // Fingerprints currently in a room (presence fresher than VOICE_STALE_MS).
  function roomMembers(server: number, channel: string): string[] {
    const r = voiceRooms[roomKey(server, channel)];
    if (!r) return [];
    const cut = Date.now() - VOICE_STALE_MS;
    return Object.entries(r).filter(([, t]) => t > cut).map(([fp]) => fp);
  }
  // Per-server "notify me of calls on this server" preference (default on).
  function loadAccept(server: number): boolean {
    try { return localStorage.getItem(`catcoms.call.accept.${server}`) !== "off"; } catch { return true; }
  }
  let acceptCallsHere = $state(true); // the active server's setting (for the Server-settings toggle)
  function toggleAcceptCalls() {
    if (activeServerId === null) return;
    acceptCallsHere = !acceptCallsHere;
    try { localStorage.setItem(`catcoms.call.accept.${activeServerId}`, acceptCallsHere ? "on" : "off"); } catch { /* ignore */ }
  }

  // The active server's operator-set TURN (Server settings). Editable here; saved locally and folded
  // into invites so members inherit it. Loaded on server switch.
  let srvTurn = $state("");
  let srvTurnUser = $state("");
  let srvTurnCred = $state("");
  function loadSrvTurn(id: number | null) {
    const t = loadServerTurn(id);
    srvTurn = t?.urls ?? "";
    srvTurnUser = t?.username ?? "";
    srvTurnCred = t?.credential ?? "";
  }
  function saveSrvTurn() {
    if (activeServerId === null) return;
    storeServerTurn(activeServerId, srvTurn.trim() ? { urls: srvTurn.trim(), username: srvTurnUser, credential: srvTurnCred } : null);
  }

  // ICE configuration (NAT traversal), user-editable in Settings → Calls and persisted locally.
  // STUN lets peers discover their public address + hole-punch (works for most home NATs); TURN
  // relays the (still SRTP-encrypted) media when hole-punching fails (symmetric NAT / strict
  // firewalls). Default to a public STUN so calls work cross-network out of the box; blank it for
  // LAN-only (no third party learns you used STUN).
  function loadCallSetting(k: string, def: string): string {
    try { return localStorage.getItem("catcoms.call." + k) ?? def; } catch { return def; }
  }
  let callStun = $state(loadCallSetting("stun", "stun:stun.l.google.com:19302"));
  let callTurn = $state(loadCallSetting("turn", ""));
  let callTurnUser = $state(loadCallSetting("turnUser", ""));
  let callTurnCred = $state(loadCallSetting("turnCred", ""));
  function saveCallSettings() {
    try {
      localStorage.setItem("catcoms.call.stun", callStun);
      localStorage.setItem("catcoms.call.turn", callTurn);
      localStorage.setItem("catcoms.call.turnUser", callTurnUser);
      localStorage.setItem("catcoms.call.turnCred", callTurnCred);
    } catch {
      /* storage unavailable */
    }
  }
  // Server-provided TURN: the operator sets one TURN endpoint (Server settings) that rides along
  // the invite string, so members don't each have to configure a relay. It's only a hint: media is
  // E2E (DTLS-SRTP), so a hostile/foreign TURN relays ciphertext at worst: hence no signing needed.
  type TurnCfg = { urls: string; username: string; credential: string };
  function serverTurnKey(id: number): string {
    return `catcoms.server.turn.${id}`;
  }
  function loadServerTurn(id: number | null): TurnCfg | null {
    if (id === null || typeof localStorage === "undefined") return null;
    const raw = localStorage.getItem(serverTurnKey(id));
    if (!raw) return null;
    try {
      const t = JSON.parse(raw) as TurnCfg;
      return t.urls?.trim() ? t : null;
    } catch {
      return null;
    }
  }
  function storeServerTurn(id: number, t: TurnCfg | null) {
    if (typeof localStorage === "undefined") return;
    if (t && t.urls.trim()) localStorage.setItem(serverTurnKey(id), JSON.stringify(t));
    else localStorage.removeItem(serverTurnKey(id));
  }
  // Append the server TURN (if any) to an invite so a joiner inherits it; joiners strip it back off.
  function wrapInvite(hex: string, id: number | null): string {
    const t = loadServerTurn(id);
    return t ? `${hex}.turn.${b64enc(JSON.stringify(t))}` : hex;
  }
  function unwrapInvite(s: string): { hex: string; turn: TurnCfg | null } {
    const i = s.indexOf(".turn.");
    if (i < 0) return { hex: s.trim(), turn: null };
    let turn: TurnCfg | null = null;
    try {
      turn = JSON.parse(b64dec(s.slice(i + 6))) as TurnCfg;
    } catch {
      turn = null;
    }
    return { hex: s.slice(0, i).trim(), turn };
  }

  function iceServers(): RTCIceServer[] {
    const out: RTCIceServer[] = [];
    for (const u of callStun.split(/[\s,]+/).filter(Boolean)) out.push({ urls: u });
    if (callTurn.trim()) {
      out.push({ urls: callTurn.trim(), username: callTurnUser, credential: callTurnCred });
    }
    // The active call's server-provided TURN (fallback for symmetric NAT), if the operator set one.
    const st = loadServerTurn(callServer ?? activeServerId);
    if (st) out.push({ urls: st.urls.trim(), username: st.username, credential: st.credential });
    return out;
  }

  function b64enc(s: string): string {
    return btoa(String.fromCharCode(...new TextEncoder().encode(s)));
  }
  function b64dec(b: string): string {
    return new TextDecoder().decode(Uint8Array.from(atob(b), (c) => c.charCodeAt(0)));
  }
  async function sendSignal(targetFp: string, msg: Record<string, unknown>) {
    if (callServer === null) return;
    try {
      await invoke("send_call_signal", { server: callServer, targetFp, payload: b64enc(JSON.stringify(msg)) });
    } catch {
      /* peer unreachable: ignore (mesh tolerates a missing edge) */
    }
  }
  // Send a signal to every online member of the call's server.
  function broadcast(msg: Record<string, unknown>) {
    for (const m of roster) {
      if (m.fingerprint !== myFp && onlineMembers.has(m.fingerprint)) void sendSignal(m.fingerprint, msg);
    }
  }
  // --- Audio devices ----------------------------------------------------------------------------
  // Which mic/speaker this install uses. Remembered locally (per machine, not per server), applied
  // when a call starts and hot-swappable mid-call via replaceTrack, so nothing ever renegotiates.
  let audioIns = $state<{ id: string; label: string }[]>([]);
  let audioOuts = $state<{ id: string; label: string }[]>([]);
  let micDev = $state(loadCallSetting("micDev", ""));
  let spkDev = $state(loadCallSetting("spkDev", ""));
  // setSinkId is Chromium-only and can be absent in the host webview; without it an OUT picker
  // would be a lie, so the stage hides it entirely rather than offering a dead control.
  const sinkSupported =
    typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype;
  type SinkAudio = HTMLAudioElement & { setSinkId?: (id: string) => Promise<void> };
  // Device labels stay blank until a mic permission exists, so this is only useful once in a call.
  async function refreshAudioDevices() {
    try {
      const list = await navigator.mediaDevices.enumerateDevices();
      audioIns = list
        .filter((d) => d.kind === "audioinput")
        .map((d, i) => ({ id: d.deviceId, label: d.label || `Input ${i + 1}` }));
      audioOuts = list
        .filter((d) => d.kind === "audiooutput")
        .map((d, i) => ({ id: d.deviceId, label: d.label || `Output ${i + 1}` }));
    } catch {
      audioIns = [];
      audioOuts = [];
    }
  }
  const onDeviceChange = () => void refreshAudioDevices();
  async function applySink(fp: string) {
    if (!sinkSupported || !spkDev) return;
    const el = document.getElementById(`call-audio-${fp}`) as SinkAudio | null;
    try { await el?.setSinkId?.(spkDev); } catch { /* device vanished: stays on the default */ }
  }
  async function setSpkDevice(id: string) {
    spkDev = id;
    try { localStorage.setItem("catcoms.call.spkDev", id); } catch { /* ignore */ }
    for (const fp of Object.keys(callPeers)) await applySink(fp);
  }
  async function setMicDevice(id: string) {
    micDev = id;
    try { localStorage.setItem("catcoms.call.micDev", id); } catch { /* ignore */ }
    if (!inCall) return;
    let next: MediaStream;
    try {
      next = await navigator.mediaDevices.getUserMedia({
        audio: id ? { deviceId: { exact: id } } : true,
        video: false,
      });
    } catch {
      error = "Couldn't switch to that microphone.";
      return;
    }
    const track = next.getAudioTracks()[0];
    if (!track) return;
    track.enabled = !callMuted; // a hot swap must never quietly un-mute you
    for (const p of Object.values(callPeers)) {
      const s = p.pc.getSenders().find((x) => x.track?.kind === "audio") ?? p.pc.getSenders()[0];
      if (s) { try { await s.replaceTrack(track); } catch { /* edge gone */ } }
    }
    if (localStream) for (const t of localStream.getTracks()) t.stop();
    localStream = next;
    addAnalyser("me", next); // the meter was watching the track that just went away
  }
  async function ensureMic(): Promise<boolean> {
    if (localStream) return true;
    // Try the remembered input first; a device that has since vanished must not block the call.
    const tries: (MediaTrackConstraints | boolean)[] = micDev
      ? [{ deviceId: { exact: micDev } }, true]
      : [true];
    for (const audio of tries) {
      try {
        localStream = await navigator.mediaDevices.getUserMedia({ audio, video: false });
        void refreshAudioDevices();
        return true;
      } catch {
        /* remembered device gone: fall back to the system default */
      }
    }
    error = "Couldn't access the microphone (permission denied or no device).";
    return false;
  }
  function attachRemote(fp: string, stream: MediaStream) {
    let el = document.getElementById(`call-audio-${fp}`) as HTMLAudioElement | null;
    if (!el) {
      el = document.createElement("audio");
      el.id = `call-audio-${fp}`;
      el.autoplay = true;
      document.body.appendChild(el);
    }
    el.srcObject = stream;
    el.muted = callDeafened || !!voiceMutedPeers[fp];
    const v = loadPeerVol(fp);
    el.volume = v;
    if (peerVolumes[fp] !== v) peerVolumes = { ...peerVolumes, [fp]: v };
    void applySink(fp);
    addAnalyser(fp, stream); // speaking detection taps the stream, never the element
  }
  // --- In-call instruments (the jam layer) ----------------------------------------------------
  // Notes are EVENTS, not audio: tiny JSON frames on a per-peer data channel, synthesized locally
  // at every ear by the same synth the melody lock uses. Near-zero bandwidth, and muting
  // instruments is a receive-side choice (global or per peer) that never touches the voice track.
  // Every note is attributable to the channel it arrived on. Full-mesh latency makes this a
  // campfire piano, not a DAW.
  const INST_WAVES: OscillatorType[] = ["sine", "triangle", "square", "sawtooth"];
  let instOpen = $state(false); // the stage's instrument drawer
  let instOctave = $state(4); // drawer piano register (C4 base, like the lock)
  let callHeld = $state<number[]>([]); // notes I am sounding into the call
  let remoteHeld = $state<Record<string, number[]>>({}); // fp -> notes they are sounding
  const remoteWave: Record<string, OscillatorType> = {}; // fp -> their last announced timbre
  let peerMeta = $state<Record<string, { mic: boolean; inst: boolean; vid: number }>>({}); // their broadcast states (vid: 0 none, 1 camera, 2 screen)
  let instMutedPeers = $state<Record<string, boolean>>({}); // my per-peer instrument mutes
  let callDeafened = $state(false);
  let myTimbre = $state<OscillatorType>(((): OscillatorType => {
    const t = loadCallSetting("timbre", "triangle") as OscillatorType;
    return INST_WAVES.includes(t) ? t : "triangle";
  })());
  let instRxMuted = $state(loadCallSetting("instrx", "on") === "off"); // true = not hearing instruments
  function setTimbre(w: OscillatorType) {
    myTimbre = w;
    try { localStorage.setItem("catcoms.call.timbre", w); } catch { /* ignore */ }
  }
  // Per-peer voice volume (0..1), remembered per fingerprint.
  let peerVolumes = $state<Record<string, number>>({});
  function loadPeerVol(fp: string): number {
    try {
      const raw = localStorage.getItem(`catcoms.call.vol.${fp}`);
      if (raw === null) return 1;
      const v = Number(raw);
      return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 1;
    } catch { return 1; }
  }
  function setPeerVolume(fp: string, v: number) {
    peerVolumes = { ...peerVolumes, [fp]: v };
    const el = document.getElementById(`call-audio-${fp}`) as HTMLAudioElement | null;
    if (el) el.volume = v;
    try { localStorage.setItem(`catcoms.call.vol.${fp}`, String(v)); } catch { /* ignore */ }
  }
  // Local per-peer voice mute: purely receive side (their <audio> element), so it needs no signal
  // and they are never told. Deafen still wins over it, hence the OR at every assignment.
  let voiceMutedPeers = $state<Record<string, boolean>>({});
  function toggleVoicePeer(fp: string) {
    const muted = !voiceMutedPeers[fp];
    voiceMutedPeers = { ...voiceMutedPeers, [fp]: muted };
    const el = document.getElementById(`call-audio-${fp}`) as HTMLAudioElement | null;
    if (el) el.muted = muted || callDeafened;
  }
  function toggleDeafen() {
    callDeafened = !callDeafened;
    for (const fp of Object.keys(callPeers)) {
      const el = document.getElementById(`call-audio-${fp}`) as HTMLAudioElement | null;
      if (el) el.muted = callDeafened || !!voiceMutedPeers[fp];
    }
    if (jukeAudio) jukeAudio.muted = callDeafened; // the deck is part of "everyone", not an exception
    if (callDeafened && !callMuted) toggleMute(); // deafened implies not transmitting either
  }
  // Note-on flood control: a token bucket per peer (~30 events/s with a small burst). Only
  // note-ONS spend tokens; note-offs always land, so a throttled peer can never strand a drone.
  const instBudget: Record<string, { tokens: number; last: number }> = {};
  function instAllow(fp: string): boolean {
    const now = performance.now();
    const b = (instBudget[fp] ??= { tokens: 60, last: now });
    b.tokens = Math.min(60, b.tokens + ((now - b.last) / 1000) * 30);
    b.last = now;
    if (b.tokens < 1) return false;
    b.tokens -= 1;
    return true;
  }
  function instState(): string {
    return JSON.stringify({
      t: "s",
      mic: callMuted ? 1 : 0,
      inst: instRxMuted ? 1 : 0,
      vid: myVideo === "cam" ? 1 : myVideo === "screen" ? 2 : 0,
    });
  }
  function pushInstState() {
    for (const p of Object.values(callPeers)) {
      if (p.dc?.readyState === "open") { try { p.dc.send(instState()); } catch { /* edge gone */ } }
    }
  }
  function handleInstMsg(fp: string, raw: unknown) {
    if (typeof raw !== "string" || raw.length > 200) return;
    let m: Record<string, unknown>;
    try { m = JSON.parse(raw) as Record<string, unknown>; } catch { return; }
    if (m.t === "s") {
      peerMeta = { ...peerMeta, [fp]: { mic: m.mic === 1, inst: m.inst === 1, vid: typeof m.vid === "number" ? m.vid : 0 } };
      return;
    }
    if (m.t !== "n") return;
    const note = m.n;
    if (typeof note !== "number" || !Number.isInteger(note) || note < 0 || note > 127) return;
    const held = remoteHeld[fp] ?? [];
    if (m.on === 1) {
      // Polyphony cap: past 16 held notes this is spam, not music.
      if (held.includes(note) || held.length >= 16 || !instAllow(fp)) return;
      remoteHeld = { ...remoteHeld, [fp]: [...held, note] };
      const w = INST_WAVES.includes(m.w as OscillatorType) ? (m.w as OscillatorType) : "triangle";
      remoteWave[fp] = w;
      if (!instRxMuted && !instMutedPeers[fp]) startTone(note, fp, w, 0.12);
    } else {
      if (!held.includes(note)) return;
      remoteHeld = { ...remoteHeld, [fp]: held.filter((n) => n !== note) };
      stopTone(note, fp);
    }
  }
  // My side: sound locally, then fan the event out to every open channel.
  function instSend(note: number, on: boolean) {
    const msg = JSON.stringify(on ? { t: "n", on: 1, n: note, w: myTimbre } : { t: "n", on: 0, n: note });
    for (const p of Object.values(callPeers)) {
      if (p.dc?.readyState === "open") { try { p.dc.send(msg); } catch { /* edge gone */ } }
    }
  }
  function instNoteOn(note: number) {
    if (!inCall || callHeld.includes(note)) return;
    callHeld = [...callHeld, note];
    startTone(note, "me", myTimbre);
    instSend(note, true);
  }
  function instNoteOff(note: number) {
    if (!callHeld.includes(note)) return;
    callHeld = callHeld.filter((n) => n !== note);
    stopTone(note);
    instSend(note, false);
  }
  function instReleaseAll() {
    for (const n of [...callHeld]) instNoteOff(n);
  }
  function stopAllFrom(src: string) {
    for (const k of [...voices.keys()]) {
      if (k.startsWith(src + ":")) stopTone(Number(k.slice(src.length + 1)), src);
    }
  }
  function toggleInstRx() {
    instRxMuted = !instRxMuted;
    try { localStorage.setItem("catcoms.call.instrx", instRxMuted ? "off" : "on"); } catch { /* ignore */ }
    for (const [fp, notes] of Object.entries(remoteHeld)) {
      if (instRxMuted) stopAllFrom(fp);
      else if (!instMutedPeers[fp]) for (const n of notes) startTone(n, fp, remoteWave[fp] ?? "triangle", 0.12);
    }
    pushInstState();
  }
  function toggleInstPeer(fp: string) {
    const muted = !instMutedPeers[fp];
    instMutedPeers = { ...instMutedPeers, [fp]: muted };
    if (muted) stopAllFrom(fp);
    else if (!instRxMuted) for (const n of remoteHeld[fp] ?? []) startTone(n, fp, remoteWave[fp] ?? "triangle", 0.12);
  }

  // --- Drawer surface: register, key tinting, edge markers, now-playing ---------------------
  // The four timbres in tile order (waveform glyph + a three-letter label): the same set the
  // wire carries, so what a peer hears is what the tile says.
  const INST_TILES: { wave: OscillatorType; label: string; d: string }[] = [
    { wave: "triangle", label: "TRI", d: "M1 10 4.5 2 8.5 10 12.5 2 16.5 10 20.5 2 24.5 10" },
    { wave: "sine", label: "SIN", d: "M1 6q3-5 6 0t6 0 6 0 6 0" },
    { wave: "square", label: "SQR", d: "M1 10h3V2h6v8h6V2h6v8h3" },
    { wave: "sawtooth", label: "SAW", d: "M1 10 7 2v8l6-8v8l6-8v8l5-6.7" },
  ];
  // Same clamp + audible confirmation the lock's setOctave gives: a shift you cannot hear would
  // silently transpose what everyone else in the call is about to receive.
  function setInstOctave(oct: number) {
    instOctave = Math.min(7, Math.max(1, oct));
    playBlip((instOctave + 1) * 12);
  }
  // The 25 keys on screen; recomputed on every register shift, so nothing may capture the base.
  let instKeys = $derived(Array.from({ length: 25 }, (_, i) => (instOctave + 1) * 12 + i));
  // note -> the peer holding it. One key can only wear one colour, so the last writer wins.
  let instHolder = $derived.by(() => {
    const m = new Map<number, string>();
    for (const [fp, notes] of Object.entries(remoteHeld)) for (const n of notes) m.set(n, fp);
    return m;
  });
  // A peer's chosen name colour, or the accent when their profile never set one.
  function instColor(fp: string): string {
    return profiles[fp]?.color?.trim() || "var(--accent-hi)";
  }
  // Remote notes outside the visible register: you must be able to see someone is playing
  // without hunting for the octave they played it in. Nearest the visible edge sorts first.
  let instEdges = $derived.by(() => {
    const base = (instOctave + 1) * 12;
    const below: { note: number; fp: string }[] = [];
    const above: { note: number; fp: string }[] = [];
    for (const [note, fp] of instHolder) {
      if (note < base) below.push({ note, fp });
      else if (note > base + 24) above.push({ note, fp });
    }
    below.sort((a, b) => b.note - a.note);
    above.sort((a, b) => a.note - b.note);
    return { below, above };
  });
  // Ascending, because a chord reads bottom-up and chordName names it over its bass.
  let instNowMine = $derived([...callHeld].sort((a, b) => a - b));
  let instNowPeers = $derived(
    callParticipants
      .filter((fp) => (remoteHeld[fp] ?? []).length > 0)
      .map((fp) => ({ fp, notes: [...remoteHeld[fp]].sort((a, b) => a - b) })),
  );
  // key -> note pinned at press time. Deliberately NOT the lock's `keyNotes`: the two surfaces
  // are never live at once, and sharing the map would let one strand a note in the other.
  const instKeyNotes = new Map<string, number>();
  // The home row is only a piano when no text field wants it.
  function typingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el?.tagName) return false;
    const tag = el.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
  }

  // ======================= 360 server space (the orbit view) =======================
  // A memory palace over the rail: servers hang as billboards on a sphere around a
  // fixed camera (yaw + clamped pitch, rotation only). The math lives in space.ts;
  // this block owns the camera, the gestures (drag-look, hold-lasso, tray), and the
  // per-device placement store. The view is an optional overlay: the rail stays the
  // default, Ctrl+O (or the rail's orbit button) toggles in and out.
  const SPACE_KEY = "catcoms.space";
  function loadSpace(): SpaceState {
    try {
      return parseSpace(localStorage.getItem(SPACE_KEY));
    } catch {
      return defaultSpace();
    }
  }
  let spaceState = $state<SpaceState>(loadSpace());
  function saveSpace() {
    try {
      localStorage.setItem(SPACE_KEY, JSON.stringify(spaceState));
    } catch {
      error = "Could not save the space layout (storage full?)";
    }
  }
  let spaceOpen = $state(false);
  let spaceCam = $state<Placement>({ yaw: 0, pitch: 0 });
  let spaceVw = $state(1200);
  let spaceVh = $state(700);
  // Focal length in px: shared by the CSS cube (perspective) and the JS projection,
  // so the backdrop and the icons never drift apart. Scales with the window so the
  // cube's 90-degree faces always cover the visible field (no seams at the edges).
  let spaceF = $derived(Math.max(560, spaceVw * 0.55));
  // Cursor as px offsets from the viewport centre (the projection's origin).
  let spaceCursor = $state({ x: 0, y: 0 });
  let spaceRoot = $state<HTMLElement | undefined>();
  // One drag at a time: "maybe" until the pointer commits to a look-drag or the
  // hold timer commits it to a lasso. Pointer capture starts only at that commit,
  // so plain clicks still reach the server buttons underneath.
  let spaceDrag: { id: number; sx: number; sy: number; yaw0: number; pitch0: number; mode: "maybe" | "look" } | null = null;
  let spaceHoldTimer = 0;
  let spaceLasso = $state<{ x: number; y: number; r: number; t0: number } | null>(null);
  // Captured servers ride as angular offsets around the aim point until dropped.
  let spaceCarried = $state<Record<number, Placement> | null>(null);
  let spaceSwallowClick = false; // a drop's trailing click must not open a server
  let spaceTrayPinned = $state(false);
  let spaceTrayHeld = $state(false);
  let spaceTray = $derived(spaceTrayPinned || spaceTrayHeld);
  // Per-server livery accents, so a hovered server can glow in its own colour.
  let spaceAccents = $state<Record<number, string>>({});
  async function refreshSpaceAccents() {
    for (const s of railServers) {
      try {
        const l = sanitizeLivery(await invoke<Livery>("get_livery", { server: s.id }));
        const a = l.accent || (l.preset ? (PRESETS.find((p) => p.id === l.preset)?.sw ?? "") : "");
        if (a) spaceAccents[s.id] = a;
        else delete spaceAccents[s.id];
      } catch {
        /* unreachable server actor: no livery accent, the user accent covers it */
      }
    }
  }
  function toggleSpace() {
    spaceOpen = !spaceOpen;
    spaceLasso = null;
    spaceCarried = null;
    spaceTrayPinned = false;
    spaceTrayHeld = false;
    spaceDrag = null;
    if (spaceOpen) refreshSpaceAccents();
  }
  // Where the placed servers land on screen this frame. While carrying, the group's
  // placements are overridden by re-anchoring the stored offsets at the cursor's aim
  // point, so the whole constellation follows the pointer without committing anything.
  let spacePlaced = $derived.by(() => {
    if (!spaceOpen) return [] as { s: ServerState; x: number; y: number; scale: number; carried: boolean }[];
    const carriedIds = new Set(Object.keys(spaceCarried ?? {}).map(Number));
    let eff = spaceState.placements;
    if (spaceCarried) {
      const aim = unproject(spaceCam, spaceCursor.x, spaceCursor.y, spaceF);
      eff = { ...eff, ...applyOffsets(spaceCarried, aim) };
    }
    const out: { s: ServerState; x: number; y: number; scale: number; carried: boolean }[] = [];
    for (const s of railServers) {
      const p = eff[s.id];
      if (!p) continue;
      const pr = project(spaceCam, p, spaceF);
      if (!pr.visible) continue;
      out.push({ s, x: pr.x, y: pr.y, scale: pr.scale, carried: carriedIds.has(s.id) });
    }
    return out;
  });
  // Servers with no place yet (new joins) wait in the tray until hung.
  let spaceUnplaced = $derived(spaceOpen ? railServers.filter((s) => !spaceState.placements[s.id]) : []);
  // "custom" without an uploaded panorama falls back to the default room.
  let spaceBackdropEff = $derived(spaceState.backdrop === "custom" && !spaceState.custom ? "den" : spaceState.backdrop);
  function spaceCursorFrom(e: PointerEvent) {
    const r = spaceRoot?.getBoundingClientRect();
    if (!r) return;
    spaceCursor = { x: e.clientX - r.left - r.width / 2, y: e.clientY - r.top - r.height / 2 };
  }
  function spaceLassoLoop() {
    if (!spaceLasso) return;
    // The circle grows while held (66px/s after a snappy start) and caps well short
    // of the viewport, so "hold longer" reads as "reach further" without ever lassoing
    // the whole sky by accident.
    spaceLasso = { ...spaceLasso, r: Math.min(300, 46 + (performance.now() - spaceLasso.t0) * 0.066) };
    requestAnimationFrame(spaceLassoLoop);
  }
  function onSpaceDown(e: PointerEvent) {
    if (e.button !== 0) return;
    spaceCursorFrom(e);
    spaceDrag = { id: e.pointerId, sx: e.clientX, sy: e.clientY, yaw0: spaceCam.yaw, pitch0: spaceCam.pitch, mode: "maybe" };
    clearTimeout(spaceHoldTimer);
    // Holding still grows a lasso from the cursor, which also covers the single-server
    // move (a lasso of one). While already carrying, the next press is a drop, not a grab.
    if (!spaceCarried) {
      spaceHoldTimer = window.setTimeout(() => {
        if (!spaceDrag || spaceDrag.mode !== "maybe" || !spaceOpen) return;
        spaceRoot?.setPointerCapture(spaceDrag.id);
        spaceLasso = { x: spaceCursor.x, y: spaceCursor.y, r: 46, t0: performance.now() };
        requestAnimationFrame(spaceLassoLoop);
      }, 350);
    }
  }
  function onSpaceMove(e: PointerEvent) {
    spaceCursorFrom(e);
    if (spaceLasso) {
      spaceLasso = { ...spaceLasso, x: spaceCursor.x, y: spaceCursor.y };
      return;
    }
    if (!spaceDrag || e.pointerId !== spaceDrag.id) return;
    const dx = e.clientX - spaceDrag.sx;
    const dy = e.clientY - spaceDrag.sy;
    if (spaceDrag.mode === "maybe") {
      if (Math.hypot(dx, dy) < 6) return; // still a click or a hold
      clearTimeout(spaceHoldTimer);
      spaceDrag.mode = "look";
      spaceRoot?.setPointerCapture(spaceDrag.id);
    }
    // Grab semantics: the world follows the hand, small-angle px-to-degrees via f.
    const k = (180 / Math.PI) / spaceF;
    spaceCam = { yaw: wrapYaw(spaceDrag.yaw0 - dx * k), pitch: clampPitch(spaceDrag.pitch0 + dy * k) };
  }
  function onSpaceUp(e: PointerEvent) {
    clearTimeout(spaceHoldTimer);
    if (!spaceDrag || e.pointerId !== spaceDrag.id) return;
    const mode = spaceDrag.mode;
    spaceDrag = null;
    if (spaceLasso) {
      const caught = lassoCapture(spaceState.placements, spaceCam, spaceLasso.x, spaceLasso.y, spaceLasso.r, spaceF);
      if (caught.length) {
        const aim = unproject(spaceCam, spaceLasso.x, spaceLasso.y, spaceF);
        spaceCarried = angularOffsets(caught, spaceState.placements, aim);
      }
      spaceLasso = null;
      spaceSwallowClick = true;
      return;
    }
    if (mode === "maybe" && spaceCarried) {
      // A plain click while carrying: drop the constellation where the cursor aims.
      const aim = unproject(spaceCam, spaceCursor.x, spaceCursor.y, spaceF);
      spaceState.placements = { ...spaceState.placements, ...applyOffsets(spaceCarried, aim) };
      spaceCarried = null;
      saveSpace();
      spaceSwallowClick = true;
    }
  }
  // Drops and lasso releases produce a trailing click on whatever sat under the
  // pointer; capture-phase swallow keeps that click from opening a server.
  function onSpaceClickCapture(e: MouseEvent) {
    if (!spaceSwallowClick) return;
    spaceSwallowClick = false;
    e.stopPropagation();
    e.preventDefault();
  }
  function spaceIconClick(id: number) {
    if (spaceCarried || spaceSwallowClick) return;
    switchServer(id); // switchServer also folds the space away
  }
  function spaceServerMenu(s: ServerState): MenuItem[] {
    return [
      { label: "Open", onSelect: () => spaceIconClick(s.id) },
      {
        label: "Return to tray",
        onSelect: () => {
          const { [s.id]: _gone, ...rest } = spaceState.placements;
          spaceState.placements = rest;
          saveSpace();
        },
      },
    ];
  }
  // Tray tap: the server flies to wherever the camera is aiming (the reticle).
  function placeFromTray(id: number) {
    spaceState.placements = { ...spaceState.placements, [id]: { yaw: spaceCam.yaw, pitch: spaceCam.pitch } };
    saveSpace();
  }
  function setSpaceBackdrop(b: string) {
    spaceState.backdrop = b as SpaceState["backdrop"];
    saveSpace();
  }
  // A custom panorama: one equirectangular 2:1 image, downscaled and stored locally
  // (it is a per-device preference, exactly like the placements).
  async function loadSpacePano(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      const url = URL.createObjectURL(file);
      const img = new Image();
      await new Promise<void>((res, rej) => {
        img.onload = () => res();
        img.onerror = () => rej(new Error("not an image"));
        img.src = url;
      });
      const w = Math.min(2048, img.naturalWidth);
      const h = Math.round((img.naturalHeight / img.naturalWidth) * w);
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      c.getContext("2d")?.drawImage(img, 0, 0, w, h);
      URL.revokeObjectURL(url);
      spaceState.custom = c.toDataURL("image/jpeg", 0.82);
      spaceState.backdrop = "custom";
      saveSpace();
    } catch (err) {
      error = String(err);
    }
  }
  // Background-position for one 90-degree wall slice of an equirect 2:1 panorama
  // (v1 shows equirect quarters flat on the cube: near-field distortion accepted).
  function panoPos(faceYaw: number): string {
    return `${(((faceYaw + 180) / 90 - 0.5) / 3) * 100}% 50%`;
  }
  const SPACE_BACKDROP_TILES = [
    { id: "den", name: "The Den" },
    { id: "ridge", name: "Nightfall Ridge" },
    { id: "void", name: "Void Deck" },
  ];
  // Panic release: folding the drawer (or the whole stage) away must not leave a note sounding
  // in everyone else's ears. untrack keeps the release out of this effect's dependency set.
  $effect(() => {
    if (instOpen && (stageOpen || focusOpen)) return; // the drawer is live on either surface
    untrack(() => {
      instKeyNotes.clear();
      instReleaseAll();
    });
  });

  // --- Jukebox (the room listens together) -----------------------------------------------------
  // Tied to the voice room, not the channel you are viewing. Nothing streams: a track is a cid in
  // the server's file share, every listener fetches the whole blob and plays it through ONE hidden
  // element. Transport (what / where / paused) rides the call signalling as "juke" frames, and
  // whoever pressed last is the DJ. Receivers never trust a wall clock: they anchor the DJ's offset
  // to their own performance.now() reading, so nobody has to agree on the time of day.
  type JukeEntry = { id: string; cid: string; name: string; author: string; added_ms: number };
  let jukeQueue = $state<JukeEntry[]>([]);
  let jukeNow = $state<{ entry: string; cid: string; name: string; paused: boolean; dj: string } | null>(null); // dj: "" is me
  let jukeStale = $state(false); // the DJ went quiet: the deck is frozen until someone presses
  let jukeDur = $state(0); // 0 until loadedmetadata knows
  let jukeFetching = $state(""); // the cid currently being pulled off the share
  let jukeReady = $state<Record<string, boolean>>({}); // cid -> already fetched
  let jukeVol = $state(loadJukeVol());
  const jukeUrls: Record<string, string> = {}; // cid -> the fetched blob's url
  const jukeFailed = new Set<string>(); // cids nobody would serve: the DJ's auto-advance skips them
  // The transport we currently follow. `seq`/`fromFp` decide who wins a race, `off`/`at` anchor the
  // position to the local clock. Plain `let`: identity, not reactivity, is what the races need.
  let jukeSeq = 0; // my own monotonic press counter
  let jukeAdopted: { seq: number; fromFp: string; off: number; at: number } | null = null;
  let jukeHeard = 0; // performance.now() of the last frame from the DJ we follow
  let jukeAudio: HTMLAudioElement | null = null;
  const JUKE_DJ_GONE_MS = 15000; // silence longer than three pings means the DJ walked away

  function loadJukeVol(): number {
    const v = Number(loadCallSetting("jukevol", "0.6"));
    return Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0.6;
  }
  function setJukeVol(v: number) {
    jukeVol = Math.min(1, Math.max(0, v));
    if (jukeAudio) jukeAudio.volume = jukeVol;
    try { localStorage.setItem("catcoms.call.jukevol", String(jukeVol)); } catch { /* ignore */ }
  }
  // The one deck element, made on first play and appended like the per-peer call audio.
  function jukeEl(): HTMLAudioElement {
    if (jukeAudio) return jukeAudio;
    const el = document.createElement("audio");
    el.id = "jukebox-audio";
    el.volume = jukeVol;
    el.muted = callDeafened;
    el.addEventListener("loadedmetadata", () => {
      jukeDur = Number.isFinite(el.duration) ? el.duration : 0;
      jukeSettle(); // the seek the src swap could not take yet
    });
    el.addEventListener("ended", () => jukeEnded());
    document.body.appendChild(el);
    jukeAudio = el;
    return el;
  }
  // I am the DJ while the transport we follow is my own press.
  function jukeIsDj(): boolean {
    return !!jukeAdopted && !!myFp && jukeAdopted.fromFp === myFp;
  }
  // Where the deck should be right now: the adopted offset plus locally measured elapsed time,
  // frozen while paused or stale. The progress UI reads this.
  function jukePos(): number {
    if (!jukeAdopted || !jukeNow) return 0;
    if (jukeNow.paused || jukeStale) return jukeAdopted.off;
    return jukeAdopted.off + (performance.now() - jukeAdopted.at) / 1000;
  }
  // Queue order (added_ms, id as the tiebreak so every machine agrees), minus the unfetchable.
  function jukePlayable(): JukeEntry[] {
    return [...jukeQueue]
      .sort((a, b) => a.added_ms - b.added_ms || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
      .filter((e) => !jukeFailed.has(e.cid));
  }
  async function refreshJukebox() {
    const server = callServer;
    const channel = callChannel;
    if (!inCall || server === null || !channel) {
      jukeQueue = [];
      return;
    }
    try {
      jukeQueue = await invoke<JukeEntry[]>("get_jukebox", { server, channel });
    } catch {
      jukeQueue = []; // no jukebox on this peer's build: an empty deck, not an error worth showing
    }
  }
  async function jukeAddTrack(cid: string, name: string) {
    const server = callServer;
    const channel = callChannel;
    if (server === null || !channel) return;
    try {
      await invoke<string>("jukebox_add", { server, channel, cid, name: name.slice(0, 200) });
      jukeFailed.delete(cid); // a re-add is also a retry
      await refreshJukebox();
    } catch (e) {
      error = String(e);
    }
  }
  async function jukeRemoveTrack(id: string) {
    const server = callServer;
    const channel = callChannel;
    if (server === null || !channel) return;
    try {
      await invoke("jukebox_remove", { server, channel, entry: id });
      await refreshJukebox();
    } catch (e) {
      error = String(e);
    }
  }
  // Claim the deck: my press outranks everything I have heard, and I apply it to myself on the same
  // path a receiver does, so the DJ is never a special case in the player.
  function jukeSend(entry: string, cid: string, name: string, off: number, paused: boolean) {
    if (!inCall || !callChannel) return;
    jukeSeq = Math.max(jukeSeq, jukeAdopted?.seq ?? 0) + 1;
    jukeAdopt(jukeSeq, myFp, entry, cid, name, off, paused);
    broadcast({ callId: callChannel, type: "juke", seq: jukeSeq, entry, cid, name, off, paused });
  }
  function jukeAdopt(seq: number, fromFp: string, entry: string, cid: string, name: string, off: number, paused: boolean) {
    const same = jukeNow?.entry === entry && jukeNow?.cid === cid;
    jukeAdopted = { seq, fromFp, off, at: performance.now() };
    jukeHeard = jukeAdopted.at;
    jukeStale = false;
    jukeNow = entry || cid ? { entry, cid, name, paused, dj: fromFp === myFp ? "" : fromFp } : null;
    if (!jukeNow) {
      jukeDur = 0;
      jukeStop(); // entry "" is the DJ saying the queue ran out
      return;
    }
    void jukeApply(same);
  }
  // Put the element where the adopted transport says it should be, fetching the blob first the one
  // time. Only the CURRENT transport may touch the element: the await below loses races with a
  // newer press, so the track is rechecked after it.
  async function jukeApply(sameTrack: boolean) {
    const now = jukeNow;
    if (!now || !now.cid) return;
    const cid = now.cid;
    const entry = now.entry;
    let url = jukeUrls[cid];
    if (!url) {
      if (jukeFetching === cid) return; // already in flight; the ping after it re-syncs
      const server = callServer;
      if (server === null) return;
      jukeFetching = cid;
      try {
        const mime = safeMime(files.find((f) => f.cid === cid)?.mime ?? "") || "audio/mpeg";
        url = await loadBlobUrl(cid, mime, server);
      } catch {
        jukeFailed.add(cid); // nobody is sharing it: the deck cannot sit here
        jukeFetching = "";
        if (jukeNow?.cid === cid && jukeIsDj()) jukeSkip();
        return;
      }
      jukeFetching = "";
      jukeUrls[cid] = url;
      jukeReady = { ...jukeReady, [cid]: true };
      if (jukeNow?.cid !== cid || jukeNow?.entry !== entry) return; // a newer press landed mid-fetch
      sameTrack = false; // seek to the target as recomputed NOW, not the one we started with
    }
    const el = jukeEl();
    if (!sameTrack || el.src !== url) {
      if (el.src !== url) {
        el.src = url;
        jukeDur = 0;
      }
      jukeSettle();
      return;
    }
    // Same track, so this is a ping or a play/pause: only a real drift is worth a jarring snap.
    const target = jukePos();
    if (el.readyState > 0 && Math.abs(el.currentTime - target) > 2) {
      try { el.currentTime = target; } catch { /* not seekable yet */ }
    }
    const live = jukeNow;
    if (!live || live.paused) el.pause();
    else void el.play().catch(() => { /* still loading, or the webview wants a gesture first */ });
  }
  // Seek + play state on an element that may have just been handed a new src (currentTime only
  // takes once there is metadata, hence the second run from the loadedmetadata listener).
  function jukeSettle() {
    const el = jukeAudio;
    if (!el || !jukeNow) return;
    const target = jukeDur > 0 ? Math.min(jukePos(), jukeDur) : jukePos();
    if (Math.abs(el.currentTime - target) > 0.25) {
      try { el.currentTime = target; } catch { /* not seekable yet */ }
    }
    if (jukeNow.paused) el.pause();
    else void el.play().catch(() => { /* still loading, or the webview wants a gesture first */ });
  }
  function jukeStop() {
    const el = jukeAudio;
    if (!el) return;
    el.pause();
    el.removeAttribute("src");
    el.load();
  }
  // Controls. Every one of them broadcasts and applies through jukeSend, so pressing anything here
  // is what makes me the DJ.
  function jukePlayEntry(id: string) {
    const e = jukeQueue.find((x) => x.id === id);
    if (!e) return;
    jukeFailed.delete(e.cid); // an explicit press is also a retry of a track that would not fetch
    jukeSend(e.id, e.cid, e.name, 0, false);
  }
  function jukeToggle() {
    if (!inCall) return;
    if (!jukeNow || !jukeNow.cid) {
      const first = jukePlayable()[0];
      if (first) jukePlayEntry(first.id);
      return;
    }
    // A press on a stale deck resumes it (and claims it) rather than pausing an already dead DJ.
    jukeSend(jukeNow.entry, jukeNow.cid, jukeNow.name, jukePos(), jukeStale ? false : !jukeNow.paused);
  }
  function jukeSkip() {
    const list = jukePlayable();
    const i = list.findIndex((e) => e.id === jukeNow?.entry);
    const next = list[i + 1]; // i is -1 with nothing playing, so this starts at the top
    if (next) jukeSend(next.id, next.cid, next.name, 0, false);
    else jukeSend("", "", "", 0, true); // queue exhausted: everyone stops
  }
  // Only the DJ advances. Everyone else's element just stops and waits for the broadcast, so the
  // room can never fan out into per-listener playlists.
  function jukeEnded() {
    if (!inCall || !jukeIsDj()) return;
    jukeSkip();
  }
  // A transport frame off the wire. Peer input, so it is validated hard, then adopted only if it
  // beats what we follow: a higher seq, or the same seq from a higher fingerprint so a simultaneous
  // press resolves the same way on every machine.
  function jukeRecv(fromFp: string, msg: Record<string, unknown>) {
    const seq = msg.seq;
    const entry = msg.entry;
    const cid = msg.cid;
    const name = msg.name;
    const off = msg.off;
    const paused = msg.paused;
    if (typeof seq !== "number" || !Number.isInteger(seq) || seq < 0) return;
    if (typeof entry !== "string" || entry.length > 200) return;
    if (typeof cid !== "string" || (cid !== "" && !/^[0-9a-f]{1,128}$/.test(cid))) return;
    if (typeof name !== "string") return;
    if (typeof off !== "number" || !Number.isFinite(off) || off < 0) return;
    if (typeof paused !== "boolean") return;
    const cur = jukeAdopted;
    const newer = !cur || seq > cur.seq || (seq === cur.seq && fromFp > cur.fromFp);
    // Not newer, but a ping from the DJ we already follow: it keeps the deck alive and re-syncs it.
    if (!newer && !(cur && seq === cur.seq && fromFp === cur.fromFp)) return;
    jukeAdopt(seq, fromFp, entry, cid, name.slice(0, 200), off, paused);
  }
  // Rides the 5s presence ping rather than owning a timer: as DJ I re-announce the transport (same
  // seq, fresh offset) so late joiners catch up and drift gets corrected; as a listener I use the
  // silence to notice a DJ who left.
  function jukeTick() {
    if (!inCall || !jukeAdopted || !jukeNow) return;
    if (jukeIsDj()) {
      broadcast({ callId: callChannel, type: "juke", seq: jukeAdopted.seq, entry: jukeNow.entry, cid: jukeNow.cid, name: jukeNow.name, off: jukePos(), paused: jukeNow.paused });
      return;
    }
    if (jukeNow.paused || jukeStale || performance.now() - jukeHeard <= JUKE_DJ_GONE_MS) return;
    jukeAdopted = { ...jukeAdopted, off: jukePos(), at: performance.now() }; // freeze where we got to
    jukeStale = true; // anyone's next press claims the deck
    jukeAudio?.pause();
  }
  // Leaving the room takes the deck with it, blobs included (each one is a whole decrypted track).
  function jukeReset() {
    jukeStop();
    jukeAudio?.remove();
    jukeAudio = null;
    for (const c of Object.keys(jukeUrls)) {
      try { URL.revokeObjectURL(jukeUrls[c]); } catch { /* a data: url has nothing to revoke */ }
      delete jukeUrls[c];
    }
    jukeFailed.clear();
    jukeReady = {};
    jukeQueue = [];
    jukeNow = null;
    jukeAdopted = null;
    jukeStale = false;
    jukeFetching = "";
    jukeDur = 0;
  }

  // --- Jukebox dock (rendering state only) ----------------------------------------------------
  // Nothing here touches the transport: it is the view over it. jukePos() is a plain function over
  // performance.now(), so no assignment ever tells Svelte the progress bar moved: jukePaint does,
  // twice a second, and only while a track is genuinely running.
  let jukePickerOpen = $state(false); // the "add from share" overlay
  let jukePaint = $state(0);
  $effect(() => {
    if (!inCall || !jukeNow || jukeNow.paused || jukeStale) return;
    const t = setInterval(() => (jukePaint = performance.now()), 500);
    return () => clearInterval(t);
  });
  // The queue as the DJ will actually play it, minus whatever is already on the deck.
  let jukeUpNext = $derived(jukePlayable().filter((e) => e.id !== jukeNow?.entry));
  let jukeAudioFiles = $derived(files.filter((f) => f.mime.startsWith("audio/")));
  // `files` is the ACTIVE server's share, while the room is on callServer: they are the same list
  // only while you are looking at the server you are called into. Every share-derived chip (gone,
  // expiring, the picker itself) is gated on this rather than lying about another server's share.
  let jukeShareInView = $derived(inCall && activeServerId !== null && activeServerId === callServer);
  function jukeClock(s: number): string {
    if (!Number.isFinite(s) || s < 0) return "0:00";
    return `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
  }
  // `_tick` is jukePaint. It is deliberately unused: reading it in the markup is what re-runs these.
  function jukeElapsed(_tick: number): string {
    return jukeClock(jukeDur > 0 ? Math.min(jukePos(), jukeDur) : jukePos());
  }
  function jukePct(_tick: number): number {
    if (!jukeNow || jukeDur <= 0) return 0;
    return Math.max(0, Math.min(100, (jukePos() / jukeDur) * 100));
  }
  // Whole days of circulation left on the listing behind a cid, or -1 when it is not close, not
  // recorded, pinned, or not this share's to answer for. `expires` is ms-epoch, the same unit the
  // Files surface hands to relDay.
  function jukeExpiryDays(cid: string): number {
    if (!jukeShareInView) return -1;
    const f = files.find((x) => x.cid === cid);
    if (!f || !f.expires_known || f.expires === null || isPinned(cid)) return -1;
    const days = Math.ceil((f.expires - nowTick) / 86_400_000);
    return days <= 7 ? Math.max(0, days) : -1;
  }
  // The share no longer carries this track: the deck can still name it, but nobody can serve it.
  function jukeGone(cid: string): boolean {
    return jukeShareInView && !files.some((f) => f.cid === cid);
  }
  // The mono tag on a picker row: the file's own extension, or the mime subtype when it has none.
  function jukeExt(f: UiFile): string {
    const dot = f.name.lastIndexOf(".");
    const ext = dot > 0 ? f.name.slice(dot + 1) : "";
    const tag = ext && ext.length <= 5 ? ext : f.mime.split("/")[1] ?? "";
    return (tag || "audio").slice(0, 5).toUpperCase();
  }

  // --- Video (camera / screen share) ----------------------------------------------------------
  // One video slot per person: the camera and a screen share swap through the same sender via
  // replaceTrack, so only the FIRST video ever renegotiates. Mesh reality check: every sender
  // uploads its video once per peer, so this is for small rooms; the SFU hookup is the scale path.
  let camStream: MediaStream | null = null; // whatever the slot currently captures
  let myVideo = $state<"" | "cam" | "screen">("");
  let localVideoStream = $state<MediaStream | null>(null); // the self-preview tile reads this
  let remoteStreams = $state<Record<string, MediaStream>>({}); // fp -> their video stream
  function dropRemoteVideo(fp: string, stream: MediaStream) {
    if (remoteStreams[fp] !== stream) return; // an ended track from a replaced, older stream
    const { [fp]: _s, ...rest } = remoteStreams;
    remoteStreams = rest;
  }
  async function startVideo(kind: "cam" | "screen") {
    let s: MediaStream;
    try {
      s = kind === "cam"
        ? await navigator.mediaDevices.getUserMedia({
            // Mesh-friendly by construction: each peer gets its own encode, so keep frames small.
            video: { width: { ideal: 640 }, height: { ideal: 360 }, frameRate: { ideal: 24 } },
          })
        : await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
    } catch {
      error = kind === "cam" ? "Couldn't access the camera (permission denied or no device)." : "Screen share was cancelled or unavailable.";
      return;
    }
    const track = s.getVideoTracks()[0];
    if (!track) return;
    const old = camStream;
    camStream = s;
    localVideoStream = s;
    myVideo = kind;
    track.onended = () => stopVideo(); // the browser's own "stop sharing" chrome ends the track
    for (const p of Object.values(callPeers)) {
      const vidSender = p.pc.getSenders().find((sn) => sn.track?.kind === "video");
      if (vidSender) void vidSender.replaceTrack(track); // same m-line: no renegotiation
      else {
        const sn = p.pc.addTrack(track, s); // first video: onnegotiationneeded takes it from here
        try {
          const prm = sn.getParameters();
          prm.encodings = [{ maxBitrate: kind === "cam" ? 500_000 : 1_200_000 }];
          void sn.setParameters(prm);
        } catch { /* pre-negotiation: the constraints above still cap it */ }
      }
    }
    if (old) for (const t of old.getTracks()) t.stop();
    pushInstState(); // vid state rides the same channel as the mute states
  }
  function stopVideo() {
    if (!camStream) return;
    for (const p of Object.values(callPeers)) {
      const sender = p.pc.getSenders().find((sn) => sn.track?.kind === "video");
      if (sender) { try { p.pc.removeTrack(sender); } catch { /* edge closing */ } }
    }
    for (const t of camStream.getTracks()) t.stop();
    camStream = null;
    localVideoStream = null;
    myVideo = "";
    pushInstState();
  }
  // Feeding a MediaStream to a <video> is the one thing markup cannot do: srcObject is a property,
  // never an attribute. Update swaps the stream in place (a replaceTrack keeps the same object,
  // so the guard makes that a no-op and never restarts playback).
  function srcObject(node: HTMLVideoElement, stream: MediaStream | null) {
    node.srcObject = stream;
    return {
      update(s: MediaStream | null) {
        if (node.srcObject !== s) node.srcObject = s;
      },
      destroy() {
        node.srcObject = null; // drop the reference so the stream can be collected
      },
    };
  }

  // --- Video focus view -------------------------------------------------------------------------
  // The 400px dock is the wrong shape for faces, so the first video takes the whole window. A
  // voice-only call NEVER does this: the dock exists precisely so voice can stay in the background.
  let focusOpen = $state(false);
  let focusDismissed = $state(false); // exiting focus must survive the auto-enter effect
  // Announced vs. arrived: the entry chip trusts their broadcast state, but auto-entering a
  // full-window overlay waits for a stream, so a stalled peer never blanks the screen.
  let videoAnnounced = $derived(myVideo !== "" || callParticipants.some((fp) => (peerMeta[fp]?.vid ?? 0) > 0));
  let videoLive = $derived(myVideo !== "" || callParticipants.some((fp) => (peerMeta[fp]?.vid ?? 0) > 0 && !!remoteStreams[fp]));
  $effect(() => {
    if (inCall && !focusDismissed && videoLive) focusOpen = true;
  });
  function openFocus() {
    focusOpen = true;
    focusDismissed = false;
  }
  function exitFocus() {
    focusOpen = false;
    focusDismissed = true; // otherwise the effect above re-opens it on the next frame
  }
  // Self first, then peers: one tile per person, and nothing else on the grid.
  let focusTiles = $derived([myFp, ...callParticipants]);
  let focusCols = $derived(focusTiles.length <= 1 ? 1 : focusTiles.length <= 4 ? 2 : 3);

  function createPeer(fp: string): CallPeer {
    const pc = new RTCPeerConnection({ iceServers: iceServers() });
    const peer: CallPeer = { fp, pc, dc: null, polite: myFp < fp, makingOffer: false, ignoreOffer: false };
    if (localStream) for (const t of localStream.getTracks()) pc.addTrack(t, localStream);
    if (camStream) for (const t of camStream.getTracks()) pc.addTrack(t, camStream); // joiner while my video is live
    // The instrument channel: negotiated (same id on both ends) and created BEFORE the offer, so
    // the SCTP section rides the first SDP exchange and nothing ever renegotiates for it. An old
    // build just never opens its end; notes then go nowhere, which degrades cleanly.
    try {
      peer.dc = pc.createDataChannel("inst", { negotiated: true, id: 7, ordered: true });
      peer.dc.onopen = () => pushInstState();
      peer.dc.onmessage = (e) => handleInstMsg(fp, e.data);
    } catch {
      /* data channels unavailable: voice still works */
    }
    // Perfect negotiation, offer side: fires for the initial tracks AND whenever video is added
    // later. The no-argument setLocalDescription picks offer/answer from the signaling state.
    pc.onnegotiationneeded = async () => {
      try {
        peer.makingOffer = true;
        await pc.setLocalDescription();
        void sendSignal(fp, { callId: callChannel, type: "offer", sdp: pc.localDescription });
      } catch {
        /* torn down mid-negotiation */
      } finally {
        peer.makingOffer = false;
      }
    };
    pc.onicecandidate = (e) => {
      if (e.candidate) void sendSignal(fp, { callId: callChannel, type: "ice", candidate: e.candidate.toJSON() });
    };
    pc.ontrack = (e) => {
      const stream = e.streams[0];
      if (!stream) return;
      if (e.track.kind === "audio") {
        attachRemote(fp, stream);
        return;
      }
      // Video: hand the stream to the tiles; clear it when the sender stops or removes the track.
      remoteStreams = { ...remoteStreams, [fp]: stream };
      e.track.onended = () => dropRemoteVideo(fp, stream);
      stream.onremovetrack = () => {
        if (!stream.getVideoTracks().length) dropRemoteVideo(fp, stream);
      };
    };
    pc.onconnectionstatechange = () => {
      callPeerStates = { ...callPeerStates, [fp]: pc.connectionState };
      if (pc.connectionState === "failed" || pc.connectionState === "closed") removePeer(fp);
    };
    callPeers[fp] = peer;
    callParticipants = Object.keys(callPeers);
    return peer;
  }
  function removePeer(fp: string) {
    const p = callPeers[fp];
    if (p) {
      try { p.pc.close(); } catch { /* already closed */ }
      delete callPeers[fp];
    }
    document.getElementById(`call-audio-${fp}`)?.remove();
    callParticipants = Object.keys(callPeers);
    const { [fp]: _drop, ...rest } = callPeerStates;
    callPeerStates = rest;
    // Silence and forget anything they were sounding; a dead edge must not drone on.
    stopAllFrom(fp);
    const { [fp]: _h, ...rh } = remoteHeld;
    remoteHeld = rh;
    const { [fp]: _m, ...pm } = peerMeta;
    peerMeta = pm;
    delete instBudget[fp];
    delete remoteWave[fp];
    dropAnalyser(fp); // a dead edge must not keep a name lit
    const { [fp]: _v, ...vm } = voiceMutedPeers;
    voiceMutedPeers = vm;
    const { [fp]: _vs, ...vs } = remoteStreams;
    remoteStreams = vs;
  }
  // A short status for the call bar: how many peers are connected, or "connecting" while ICE works.
  let callStatusText = $derived.by(() => {
    const n = callParticipants.length;
    if (n === 0) return "waiting for others…";
    const connected = callParticipants.filter((fp) => callPeerStates[fp] === "connected").length;
    if (connected === n) return `${n} connected`;
    const failed = callParticipants.some(
      (fp) => callPeerStates[fp] === "failed" || callPeerStates[fp] === "disconnected",
    );
    return failed ? `${connected}/${n} connected · check NAT/TURN` : `${connected}/${n} · connecting…`;
  });

  // --- Voice stage: speaking detection + mic meter -----------------------------------------------
  // One AudioContext, one analyser per source, one 120ms timer. Time-domain RMS is enough to light
  // a name and fill four bars, and it costs nothing next to the codecs already running. Analysers
  // are never wired to the destination: tapping a remote stream must not double it into the ears.
  let stageOpen = $state(false); // the expanded stage (vs. the collapsed call bar)
  let speaking = $state<Record<string, boolean>>({}); // "me" or a peer fp -> above the talk floor
  let micLevel = $state(0); // 0..1, drives the meter
  // How far each voice's ears are up: the same RMS the ring uses, quantised to four steps. It is
  // quantised for the same reason the mic meter is drawn as four bars rather than a continuous
  // fill: a value that changes every tick would re-render the stage eight times a second for a
  // difference no eye can see. Four steps is enough for ears to read as twitching.
  let earPerk = $state<Record<string, number>>({}); // "me" or a peer fp -> 0..3
  const perkOf = (rms: number): number =>
    rms <= SPEAK_FLOOR ? 0 : Math.min(3, 1 + Math.floor((rms / METER_FULL) * 2.4));
  let linksUp = $derived(callParticipants.filter((fp) => callPeerStates[fp] === "connected").length);
  type Meter = { src: MediaStreamAudioSourceNode; an: AnalyserNode; buf: ReturnType<typeof mkBuf> };
  const mkBuf = (n: number) => new Uint8Array(n);
  const meters: Record<string, Meter> = {};
  let meterCtx: AudioContext | null = null;
  let meterTimer: ReturnType<typeof setInterval> | undefined;
  const SPEAK_FLOOR = 0.02; // RMS below this is room tone, not a voice
  const METER_FULL = 0.25; // RMS that lights the last bar

  function addAnalyser(key: string, stream: MediaStream) {
    dropAnalyser(key);
    try {
      meterCtx ??= new AudioContext();
      if (meterCtx.state === "suspended") void meterCtx.resume();
      const src = meterCtx.createMediaStreamSource(stream);
      const an = meterCtx.createAnalyser();
      an.fftSize = 512;
      src.connect(an);
      meters[key] = { src, an, buf: mkBuf(an.fftSize) };
    } catch {
      /* no Web Audio here: the stage simply never lights up */
    }
  }
  function dropAnalyser(key: string) {
    const m = meters[key];
    if (!m) return;
    try { m.src.disconnect(); } catch { /* already torn down */ }
    delete meters[key];
    if (speaking[key]) {
      const { [key]: _s, ...rest } = speaking;
      speaking = rest;
    }
    if (earPerk[key] !== undefined) {
      const { [key]: _p, ...rest } = earPerk;
      earPerk = rest;
    }
  }
  function rmsOf(m: Meter): number {
    m.an.getByteTimeDomainData(m.buf);
    let sum = 0;
    for (let i = 0; i < m.buf.length; i++) {
      const v = (m.buf[i] - 128) / 128;
      sum += v * v;
    }
    return Math.sqrt(sum / m.buf.length);
  }
  function startMeters() {
    clearInterval(meterTimer);
    meterTimer = setInterval(() => {
      const next: Record<string, boolean> = {};
      const perk: Record<string, number> = {};
      let mine = 0;
      for (const [key, m] of Object.entries(meters)) {
        const rms = rmsOf(m);
        if (key === "me") mine = rms;
        next[key] = rms > SPEAK_FLOOR;
        perk[key] = perkOf(rms);
      }
      // A muted mic transmits nothing, so it must never read as speaking however loud the room is.
      next.me = !callMuted && (next.me ?? false);
      if (callMuted) perk.me = 0; // and ears that twitch to a muted mic would be a lie about it
      // Only publish on a real change: a fresh object every 120ms would re-render the stage for
      // nothing eight times a second.
      const keys = Object.keys(next);
      if (keys.length !== Object.keys(speaking).length || keys.some((k) => speaking[k] !== next[k])) {
        speaking = next;
      }
      const pkeys = Object.keys(perk);
      if (pkeys.length !== Object.keys(earPerk).length || pkeys.some((k) => earPerk[k] !== perk[k])) {
        earPerk = perk;
      }
      micLevel = callMuted ? 0 : Math.min(1, mine / METER_FULL);
    }, 120);
  }
  function stopMeters() {
    clearInterval(meterTimer);
    meterTimer = undefined;
    for (const key of Object.keys(meters)) dropAnalyser(key);
    if (meterCtx) {
      const c = meterCtx;
      meterCtx = null;
      try { void c.close(); } catch { /* already closed */ }
    }
    speaking = {};
    earPerk = {};
    micLevel = 0;
  }
  // The connection glyph: three states, mono, no prose. EST is a live edge, NEG is still
  // handshaking, LOST is a dead one (NAT gave up, or they walked away without a "bye").
  function linkState(state: string): "est" | "neg" | "lost" {
    if (state === "connected") return "est";
    return state === "failed" || state === "disconnected" || state === "closed" ? "lost" : "neg";
  }
  function toggleInstDrawer() {
    instOpen = !instOpen;
    if (instOpen) void initMidi(); // only ask for MIDI once a keyboard is actually on screen
  }

  function recordPresence(server: number, channel: string, fp: string) {
    const key = roomKey(server, channel);
    voiceRooms = { ...voiceRooms, [key]: { ...(voiceRooms[key] ?? {}), [fp]: Date.now() } };
  }
  function dropPresence(server: number, channel: string, fp: string) {
    const key = roomKey(server, channel);
    if (!voiceRooms[key]) return;
    const r = { ...voiceRooms[key] };
    delete r[fp];
    voiceRooms = { ...voiceRooms, [key]: r };
  }
  function channelNameFor(server: number, channel: string): string {
    return servers.find((s) => s.id === server)?.channels.find((c) => c.id === channel)?.name ?? "voice";
  }
  // Notify (chime + banner) when a room I'm NOT in just became active: gated by the server setting.
  function maybeNotifyRoom(server: number, channel: string, wasActive: boolean) {
    if (wasActive) return;
    const key = roomKey(server, channel);
    if (alertedRooms.has(key)) return;
    if (inCall && callChannel === channel && callServer === server) return;
    if (!loadAccept(server)) return;
    alertedRooms.add(key);
    voiceAlert = { server, channel, name: channelNameFor(server, channel) };
    playMention();
  }
  // Join (or switch to) a channel's voice room. The channel id IS the call id.
  async function joinVoice(channel: string, server: number, name: string) {
    if (inCall && callChannel === channel && callServer === server) return;
    if (inCall) leaveVoice();
    callServer = server;
    if (!(await ensureMic())) { callServer = null; return; }
    callChannel = channel;
    callChannelName = name;
    inCall = true;
    callMuted = false;
    focusOpen = false;
    focusDismissed = false; // a new call earns a fresh chance to take the window
    voiceAlert = null;
    if (localStream) addAnalyser("me", localStream);
    startMeters();
    navigator.mediaDevices?.addEventListener?.("devicechange", onDeviceChange);
    alertedRooms.delete(roomKey(server, channel));
    recordPresence(server, channel, myFp);
    void refreshJukebox(); // the room's queue, whatever the DJ is currently on
    broadcast({ callId: channel, type: "hello", mic: 0, inst: instRxMuted ? 1 : 0 }); // announce + trigger existing members to offer
    clearInterval(pingTimer);
    pingTimer = setInterval(() => {
      if (callChannel && callServer !== null) {
        broadcast({ callId: callChannel, type: "voice-ping", mic: callMuted ? 1 : 0, inst: instRxMuted ? 1 : 0 });
        recordPresence(callServer, callChannel, myFp); // keep my own presence fresh
        jukeTick(); // the DJ's re-announce (and the listener's DJ-left check) ride this tick
      }
    }, 5000);
  }
  function leaveVoice() {
    if (callChannel) broadcast({ callId: callChannel, type: "bye" });
    instReleaseAll(); // lift my own notes (and tell peers) before the edges go down
    if (camStream) {
      for (const t of camStream.getTracks()) t.stop();
      camStream = null;
    }
    localVideoStream = null;
    myVideo = "";
    remoteStreams = {};
    for (const fp of Object.keys(callPeers)) removePeer(fp);
    callHeld = [];
    remoteHeld = {};
    peerMeta = {};
    instOpen = false;
    stageOpen = false;
    focusOpen = false;
    focusDismissed = false;
    callDeafened = false;
    voiceMutedPeers = {};
    jukeReset();
    stopMeters();
    navigator.mediaDevices?.removeEventListener?.("devicechange", onDeviceChange);
    if (localStream) {
      for (const t of localStream.getTracks()) t.stop();
      localStream = null;
    }
    clearInterval(pingTimer);
    pingTimer = undefined;
    if (callServer !== null && callChannel) dropPresence(callServer, callChannel, myFp);
    inCall = false;
    callMuted = false;
    callChannel = "";
    callChannelName = "";
    callServer = null;
  }
  function toggleMute() {
    callMuted = !callMuted;
    if (localStream) for (const t of localStream.getAudioTracks()) t.enabled = !callMuted;
    pushInstState(); // peers show my mute state; tell them now rather than at the next ping
  }
  function joinActiveVoice() {
    if (activeServerId !== null && cur?.active) joinVoice(cur.active, activeServerId, activeName());
  }
  // A live room in some OTHER channel of the active server: surfaced as a header chip, because the
  // sidebar pill only helps while the channel list is scrolled into view. Recomputes as presence
  // pings and the stale-prune tick touch voiceRooms.
  let liveElsewhere = $derived.by(() => {
    if (activeServerId === null) return null;
    for (const c of cur?.channels ?? []) {
      if (c.id === cur?.active) continue; // the viewed channel already has the Join button
      if (inCall && callServer === activeServerId && callChannel === c.id) continue; // that's my call
      const n = roomMembers(activeServerId, c.id).length;
      if (n) return { id: c.id, name: c.name, n };
    }
    return null;
  });
  async function handleCallSignal(fromFp: string, payloadB64: string, server: number) {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(b64dec(payloadB64));
    } catch {
      return;
    }
    const cid = msg.callId as string | undefined;
    const type = msg.type as string | undefined;
    if (!cid || !type) return;
    // Presence: both "hello" (a newcomer) and "voice-ping" (heartbeat) mean someone's in a room.
    if (type === "hello" || type === "voice-ping") {
      const wasActive = roomMembers(server, cid).length > 0;
      recordPresence(server, cid, fromFp);
      maybeNotifyRoom(server, cid, wasActive);
      // Broadcast mute states ride the presence pings (the data channel also carries them, but
      // pings cover the window before it opens). Only my own room's states matter to the UI.
      if (inCall && cid === callChannel && typeof msg.mic === "number") {
        peerMeta = { ...peerMeta, [fromFp]: { mic: msg.mic === 1, inst: msg.inst === 1, vid: typeof msg.vid === "number" ? msg.vid : 0 } };
      }
      if (type === "voice-ping") return; // presence only
    }
    // Everything below is only for MY current room.
    if (!inCall || cid !== callChannel) return;
    if (type === "juke") {
      jukeRecv(fromFp, msg); // shared-listening transport: what is playing and where it is
      return;
    }
    if (type === "hello") {
      if (callPeers[fromFp]) return;
      playBlip(79); // audible arrival: there is no lobby, so the room itself says someone joined
      createPeer(fromFp); // its tracks + data channel raise onnegotiationneeded, which sends the offer
    } else if (type === "offer") {
      const peer = callPeers[fromFp] ?? createPeer(fromFp);
      const pc = peer.pc;
      // Perfect negotiation, answer side: on a collision the impolite end ignores the incoming
      // offer (its own is in flight and will win); the polite end lets setRemoteDescription
      // implicitly roll its own offer back.
      const collision = peer.makingOffer || pc.signalingState !== "stable";
      peer.ignoreOffer = !peer.polite && collision;
      if (peer.ignoreOffer) return;
      await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
      await pc.setLocalDescription(); // no-arg picks "answer" from the have-remote-offer state
      void sendSignal(fromFp, { callId: callChannel, type: "answer", sdp: pc.localDescription });
    } else if (type === "answer") {
      const pc = callPeers[fromFp]?.pc;
      // Guard against a stale answer landing after a rollback settled the state.
      if (pc && pc.signalingState === "have-local-offer") {
        await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
      }
    } else if (type === "ice") {
      const peer = callPeers[fromFp];
      if (peer && msg.candidate) {
        try {
          await peer.pc.addIceCandidate(new RTCIceCandidate(msg.candidate as RTCIceCandidateInit));
        } catch {
          if (!peer.ignoreOffer) { /* genuinely stale candidate: harmless */ }
        }
      }
    } else if (type === "bye") {
      removePeer(fromFp);
      dropPresence(server, cid, fromFp);
    }
  }

  // Per-channel composer drafts (in-memory): switching channels/servers preserves what you'd typed.
  let drafts = $state<Record<string, string>>({});
  function saveDraftFor(key: string | null) {
    if (!key) return;
    if (draft.trim()) drafts[key] = draft;
    else delete drafts[key];
  }
  function loadDraftFor(key: string | null) {
    draft = (key && drafts[key]) || "";
  }

  // Wrap the composer's selection (or insert at the caret) with markdown markers: the formatting
  // toolbar's bold/italic/etc. After it, the wrapped text stays selected so toggling reads naturally.
  function wrapSelection(before: string, after = before) {
    const ta = composerEl;
    const start = ta?.selectionStart ?? draft.length;
    const end = ta?.selectionEnd ?? start;
    const sel = draft.slice(start, end);
    draft = draft.slice(0, start) + before + sel + after + draft.slice(end);
    const a = start + before.length;
    queueMicrotask(() => {
      if (composerEl) {
        composerEl.focus();
        composerEl.selectionStart = a;
        composerEl.selectionEnd = a + sel.length;
      }
    });
  }

  async function send() {
    const text = draft.trim();
    if (!text || !cur || !cur.active || activeServerId === null) return;
    const reply_to = replyingTo;
    const key = chanKey();
    draft = "";
    replyingTo = "";
    mentionQuery = null;
    if (key) delete drafts[key];
    try {
      await invoke("send_message", { server: activeServerId, channel: cur.active, text, replyTo: reply_to });
    } catch (e) {
      error = String(e);
    }
  }

  // Inline edit of one of your own messages.
  let editingId = $state("");
  let editDraft = $state("");
  function startEdit(m: Msg) {
    editingId = m.id;
    editDraft = m.text;
  }
  function cancelEdit() {
    editingId = "";
    editDraft = "";
  }
  async function saveEdit(m: Msg) {
    const text = editDraft.trim();
    const ch = cur?.active;
    if (!text || activeServerId === null || !ch) {
      cancelEdit();
      return;
    }
    cancelEdit();
    if (text === m.text) return; // no change
    try {
      await invoke("edit_message", { server: activeServerId, channel: ch, msgId: m.id, text });
    } catch (e) {
      error = String(e);
    }
  }
  async function deleteMessage(m: Msg) {
    const ch = cur?.active;
    if (activeServerId === null || !ch) return;
    try {
      await invoke("delete_message", { server: activeServerId, channel: ch, msgId: m.id });
    } catch (e) {
      error = String(e);
    }
  }

  // Emoji reactions: a tiny quick-picker per message; toggling adds/removes your reaction.
  let reactionPickerFor = $state("");
  function toggleReactionPicker(m: Msg) {
    reactionPickerFor = reactionPickerFor === m.id ? "" : m.id;
  }
  async function toggleReaction(m: Msg, emoji: string) {
    const ch = cur?.active;
    reactionPickerFor = "";
    if (activeServerId === null || !ch || !m.id) return;
    try {
      await invoke("toggle_reaction", { server: activeServerId, channel: ch, msgId: m.id, emoji });
    } catch (e) {
      error = String(e);
    }
  }
  // A reaction emoji can be a unicode glyph or a custom `:name:` (a server emoji file). Returns the
  // custom code if it's the latter, so the chip + picker can render the emoji image.
  function customEmojiCode(emoji: string): string | null {
    const m = /^:([a-z0-9_+\-]{1,40}):$/i.exec(emoji);
    return m ? m[1].toLowerCase() : null;
  }

  // Pinned messages (owner/admin pin/unpin; a panel surfaces them).
  let showPinned = $state(false);
  let pinnedMsgs = $derived(messages.filter((m) => m.pinned));
  async function togglePin(m: Msg) {
    const ch = cur?.active;
    if (activeServerId === null || !ch || !m.id) return;
    try {
      await invoke("set_pin", { server: activeServerId, channel: ch, msgId: m.id, pinned: !m.pinned });
    } catch (e) {
      error = String(e);
    }
  }

  // The issue tracker feedback is filed against. The backend refuses to launch anything that
  // isn't under this, so the URL built here is the only one the app can ever open.
  const ISSUE_TRACKER = "https://github.com/Thalpy/Mewtual";

  function feedbackReport(): string {
    return [
      `**Type:** ${feedbackKind === "bug" ? "Bug report" : "Feature request"}`,
      `**App:** Mewtual desktop ${APP_VERSION}`,
      `**Environment:** ${navigator.userAgent}`,
      ``,
      feedbackText.trim(),
    ].join("\n");
  }

  // A title is optional in the form: fall back to the first line of the description so the
  // issue never lands on GitHub untitled.
  function feedbackSubject(): string {
    const typed = feedbackTitle.trim();
    const first = feedbackText.trim().split("\n")[0].trim();
    return (typed || first || (feedbackKind === "bug" ? "Bug report" : "Feature request")).slice(0, 120);
  }

  async function copyFeedback() {
    try {
      await navigator.clipboard.writeText(feedbackReport());
      feedbackCopied = true;
      setTimeout(() => (feedbackCopied = false), 2000);
    } catch (e) {
      error = String(e);
    }
  }

  // File the report on the tracker by opening GitHub's new-issue form, prefilled, in the
  // user's browser. The app holds no GitHub credentials and posts nothing itself: the user
  // reviews the filled-in form and presses Submit, which also keeps them as the issue author
  // so maintainers can reply to them.
  async function openFeedbackIssue() {
    const labels = feedbackKind === "bug" ? "bug" : "enhancement";
    const url = (body: string) =>
      `${ISSUE_TRACKER}/issues/new?labels=${labels}` +
      `&title=${encodeURIComponent(feedbackSubject())}` +
      `&body=${encodeURIComponent(body)}`;
    let body = feedbackReport();
    // GitHub serves a 414 rather than a form past roughly 8k of URL, and percent-encoding
    // inflates the body several times over. Trim the tail instead of handing over a dead
    // link, and put the full text on the clipboard so nothing typed is lost.
    const LIMIT = 6000;
    const NOTE = "\n\n_(Report truncated: the full text is on your clipboard.)_";
    if (url(body).length > LIMIT) {
      await copyFeedback();
      while (body.length > 200 && url(body + NOTE).length > LIMIT) body = body.slice(0, -200);
      body += NOTE;
    }
    try {
      await invoke("open_issue_url", { url: url(body) });
      feedbackOpened = true;
      setTimeout(() => (feedbackOpened = false), 4000);
    } catch (e) {
      error = String(e);
    }
  }

  // --- update check: quiet on launch, never nagging ---------------------------------------------
  // Releases are minisign-signed and the download + verification happen in Rust (see the updater
  // plugin), so the webview only learns "there is a newer version" and can ask for it to be
  // installed. An unsigned or tampered bundle is refused before anything is written to disk.
  let updateAvail = $state<{ version: string; notes: string } | null>(null);
  let updateBusy = $state(false);
  let updatePct = $state(0);
  let updateHandle: Update | null = null;
  // Remembers the one version the user actively refused, so "Skip" means skip rather than
  // "ask me again tomorrow". Anything newer than it is still offered.
  const UPDATE_SKIP_KEY = "mewtual.skipUpdate";

  async function checkForUpdate(manual = false) {
    try {
      const found = await check();
      if (!found) {
        if (manual) toast(`You are on the latest version (${APP_VERSION})`, "ok");
        return;
      }
      if (!manual && localStorage.getItem(UPDATE_SKIP_KEY) === found.version) return;
      updateHandle = found;
      updateAvail = { version: found.version, notes: (found.body ?? "").trim() };
    } catch (e) {
      // Silent unless they asked: being offline, or GitHub being unreachable, is not something
      // the user can act on, and an app that greets you with an error on every launch is worse
      // than one that quietly checks again next time.
      if (!manual) return;
      // A build from source has no endpoint configured, on purpose (see docs/RELEASING.md):
      // only official builds point at the official release feed. Say so rather than showing
      // a raw plugin error to someone who is running their own build.
      const noChannel = String(e).includes("endpoints");
      toast(
        noChannel
          ? "This build has no update channel: it was built from source, so update it the way you built it."
          : `Could not check for updates: ${e}`,
        noChannel ? "info" : "err",
        9000,
      );
    }
  }

  async function installUpdate() {
    if (!updateHandle || updateBusy) return;
    updateBusy = true;
    updatePct = 0;
    let total = 0;
    let got = 0;
    try {
      await updateHandle.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        else if (ev.event === "Progress") {
          got += ev.data.chunkLength;
          updatePct = total ? Math.min(100, Math.round((got / total) * 100)) : 0;
        }
      });
      await relaunch();
    } catch (e) {
      updateBusy = false;
      toast(`Update failed: ${e}`, "err", 9000);
    }
  }

  // "Later" hides it for this run; the next launch offers it again. "Skip" retires this version
  // for good: it stays reachable from Settings, so refusing an update is never a dead end.
  function dismissUpdate(forever: boolean) {
    if (forever && updateAvail) localStorage.setItem(UPDATE_SKIP_KEY, updateAvail.version);
    updateAvail = null;
  }

  async function copyInvite() {
    if (!cur) return;
    try {
      await navigator.clipboard.writeText(wrapInvite(cur.invite, cur.id));
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // Clipboard may be unavailable in the webview: the textarea allows manual copy.
    }
  }

  let mintingInvite = $state(false);
  // Mint a fresh single-use invite on demand (owner or admin: the backend gates on can_invite).
  // The new invite carries the live bootstrap address, so it works even after a restart changed
  // the listen port. An admin's invitee is owner-serialized (admitted when the owner is online).
  async function generateInvite() {
    if (activeServerId === null || !cur) return;
    mintingInvite = true;
    try {
      cur.invite = await invoke<string>("mint_invite_fresh", { server: activeServerId });
      copied = false;
    } catch (e) {
      error = String(e);
    } finally {
      mintingInvite = false;
    }
  }

  // A short two-note chime via the Web Audio API (no asset to bundle), gated by the
  // notification-sound preference. Played for messages you aren't actively looking at.
  let audioCtx: AudioContext | null = null;
  function playChime(freqs: number[]) {
    if (!soundOn) return;
    try {
      audioCtx = audioCtx ?? new AudioContext();
      const ctx = audioCtx;
      if (ctx.state === "suspended") void ctx.resume();
      const now = ctx.currentTime;
      freqs.forEach((freq, i) => {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        osc.type = "sine";
        osc.frequency.value = freq;
        const t = now + i * 0.09;
        gain.gain.setValueAtTime(0.0001, t);
        gain.gain.exponentialRampToValueAtTime(0.16, t + 0.01);
        gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.18);
        osc.connect(gain).connect(ctx.destination);
        osc.start(t);
        osc.stop(t + 0.2);
      });
    } catch {
      /* audio unavailable */
    }
  }
  // A regular new-message chime (two notes) vs a distinct, brighter rising triad for a message that
  // mentions you or replies to you.
  function playNotify() {
    playChime([880, 1318.5]);
  }
  function playMention() {
    playChime([987.8, 1318.5, 1760]);
  }

  // ---- location & history --------------------------------------------------
  // "Where you are" as one comparable value: the top-level area, and inside a group the surface
  // plus that surface's own selection. Back and forward restore exactly this and nothing more;
  // scroll offset, open overlays and drafts belong to the moment rather than to the place.
  type Loc = {
    area: "inbox" | "dms" | "group";
    server: number | null;
    view: Tab;
    channel: string; // chat: the active channel id
    page: string; // wiki: the open page
    folder: string; // files: the open folder path
  };
  const NAV_MAX = 100; // a trail this long is browsing, not backtracking

  const here = (): Loc => ({
    area: inboxView ? "inbox" : dmHome && activeServerId === null ? "dms" : "group",
    server: activeServerId,
    view,
    channel: cur?.active ?? "",
    page: activeWikiPage,
    folder,
  });
  // Same PLACE, not same object: only the fields the current surface actually shows count, so
  // drilling through folders while you are reading chat never counts as a move.
  const sameLoc = (a: Loc, b: Loc) =>
    a.area === b.area &&
    a.server === b.server &&
    (a.area !== "group" ||
      (a.view === b.view &&
        (a.view !== "chat" || a.channel === b.channel) &&
        (a.view !== "wiki" || a.page === b.page) &&
        (a.view !== "files" || a.folder === b.folder)));

  let navStack = $state<Loc[]>([]);
  let navAt = $state(-1);
  let navApplying = false; // a back/forward is being applied: what it changes is not a new place
  let navStepBase = -2; // >= -1 while a multi-hop jump collapses into one entry
  // A group you have since left cannot be returned to, so those entries are stepped over instead
  // of offered, and the buttons grey out when nothing reachable is left that way.
  const navAlive = (l: Loc) => l.area !== "group" || servers.some((s) => s.id === l.server);
  let canGoBack = $derived(navAt > 0 && navStack.slice(0, navAt).some(navAlive));
  let canGoFwd = $derived(navAt >= 0 && navStack.slice(navAt + 1).some(navAlive));

  function recordLoc(loc: Loc) {
    if (loc.area === "group" && loc.server === null) return; // nothing selected: not a place yet
    const top = navAt >= 0 ? navStack[navAt] : null;
    if (top && sameLoc(top, loc)) {
      navStack[navAt] = loc; // same place, fresher detail (the folder you left it in, say)
      return;
    }
    // Inside an open step the entry that step already pushed moves along with it, so a jump that
    // crosses a server AND a channel leaves one place to come back from instead of three.
    if (navStepBase >= -1 && navAt > navStepBase) {
      navStack[navAt] = loc;
      return;
    }
    const next = [...navStack.slice(0, navAt + 1), loc]; // a fresh move drops the forward trail
    navStack = next.length > NAV_MAX ? next.slice(next.length - NAV_MAX) : next;
    navAt = navStack.length - 1;
  }
  // The recorder watches the location itself rather than each entry point, so a route added later
  // (a wikilink, a search hit, a context menu) lands in the history without having to be told.
  // Recording is untracked: it writes the stack, and reading the stack here would loop.
  $effect(() => {
    const loc = here();
    if (locked || navApplying) return;
    untrack(() => recordLoc(loc));
  });

  // Jumps that cross several awaits announce themselves, so their hops collapse into the single
  // place the user actually asked for.
  function navStepStart() {
    navStepBase = navAt;
  }
  function navStepEnd() {
    tick().then(() => (navStepBase = -2));
  }

  // Put the app back at a recorded place. Order matters: switching group resets the surface, the
  // wiki page and the folder, so the group has to land first.
  async function applyLoc(loc: Loc) {
    navApplying = true;
    try {
      if (loc.area === "inbox") {
        openInbox();
        return;
      }
      if (loc.area === "dms") {
        enterDmHome();
        return;
      }
      inboxView = false;
      if (loc.server !== null && loc.server !== activeServerId) await switchServer(loc.server);
      if (loc.view === "chat") {
        if (loc.channel && cur?.active !== loc.channel) await switchTo(loc.channel);
        switchView("chat"); // switchTo moves the channel; the surface is this call's job
      } else if (loc.view === "wiki" && loc.page) {
        menu = null;
        view = "wiki";
        // Awaited, not left to switchView: openWikiPage reads the page list to decide read vs
        // edit mode, so a page that arrived second would reopen in the editor.
        await refreshWiki();
        await openWikiPage(loc.page, { noRedirect: true }); // you are going back TO the target
      } else {
        if (loc.view === "files") folder = loc.folder;
        switchView(loc.view);
      }
    } finally {
      await tick(); // let the effect see the applied state before recording resumes
      navApplying = false;
    }
  }

  // Moving the cursor is instant; landing there is not. A second press while the first is still
  // in flight just moves the cursor again, and the drain picks up wherever it ended: mashing the
  // thumb button walks the trail rather than being swallowed one press at a time.
  function navGo(i: number) {
    navAt = i;
    if (navApplying) return;
    void navDrain();
  }
  async function navDrain() {
    let target: Loc | null = navStack[navAt] ?? null;
    while (target) {
      await applyLoc(target);
      const landed: Loc | null = navStack[navAt] ?? null;
      target = landed && !sameLoc(landed, target) ? landed : null;
    }
  }
  function navBack() {
    for (let i = navAt - 1; i >= 0; i--) {
      if (!navAlive(navStack[i])) continue;
      navGo(i);
      return;
    }
  }
  function navForward() {
    for (let i = navAt + 1; i < navStack.length; i++) {
      if (!navAlive(navStack[i])) continue;
      navGo(i);
      return;
    }
  }

  // ---- title bar ambience ---------------------------------------------------
  // The strip is the only surface that is visible in every app state, the lock screen included,
  // so the rule for it is: colour and shape may persist, named content may not.
  const APP_VERSION = __APP_VERSION__;
  let windowFocused = $state(true);
  // The hairline carries the active server's published accent: an ambient "which world am I in"
  // that costs no space and, being a colour rather than a name, survives a screenshot.
  let tbEdge = $derived(locked || !followLiveryNow ? "" : livery.accent);
  // The ident line, in the status bar's register, so the app reads as one framed terminal window.
  let tbPreset = $derived(
    (followLiveryNow ? livery.preset : appearance.preset) || "nightshade",
  );

  // ---- news ticker ----------------------------------------------------------
  // The strip only moves when something actually happened, so the motion itself is the signal
  // rather than decoration. Items are built where the data lands rather than from the raw events,
  // because an event only says "the wiki changed" while a diff says WHICH page.
  type TickerKind = "status" | "wiki" | "event";
  type TickerItem = { id: string; kind: TickerKind; text: string; at: number; go: () => void };
  const TICKER_TTL = 5 * 60_000; // news for five minutes; after that it is just history
  const TICKER_MAX = 8;
  let tickerItems = $state<TickerItem[]>([]);
  function pushTicker(kind: TickerKind, id: string, text: string, go: () => void) {
    if (locked) return; // nothing that names app content may reach a locked screen
    if (!text.trim() || tickerItems.some((t) => t.id === id)) return;
    const at = Date.now();
    const kept = tickerItems.filter((t) => at - t.at < TICKER_TTL);
    tickerItems = [...kept, { id, kind, text, at, go }].slice(-TICKER_MAX);
  }
  function pruneTicker() {
    const at = Date.now();
    const kept = tickerItems.filter((t) => at - t.at < TICKER_TTL);
    if (kept.length !== tickerItems.length) tickerItems = kept;
  }
  // A ticker item is a place, so following one is a navigation: it goes through the same step
  // machinery as any other jump and Back returns you to where you were reading.
  async function goSurface(server: number, v: Tab) {
    navStepStart();
    try {
      if (server !== activeServerId) await switchServer(server);
      switchView(v);
    } finally {
      navStepEnd();
    }
  }
  async function goWikiPage(server: number, page: string) {
    navStepStart();
    try {
      if (server !== activeServerId) await switchServer(server);
      menu = null;
      view = "wiki";
      await refreshWiki();
      await openWikiPage(page, { noRedirect: true });
    } finally {
      navStepEnd();
    }
  }

  // ---- mascot ---------------------------------------------------------------
  // The app's mood in one glyph: asleep when nobody is looking or the vault is shut, ears up when
  // something wants you, busy while transfers run. Every input here is state the app already
  // tracks, so the cat can never disagree with the rest of the chrome.
  let catBlink = $state(false);
  let catArt = $derived(
    locked || !windowFocused
      ? catSleepArt
      : catBlink
        ? catBlinkArt
        : mentionChannels.size > 0
          ? catAlertArt
          : activeDownloads > 0
            ? catSyncArt
            : catIdleArt,
  );

  // Why the cat is in the pose it is in. The mascot is the one indicator with no words on it, so
  // this is the readout that makes it legible: it goes on the button's title and its aria-label.
  // Same rule as the rest of the strip: a locked screen may describe the app's state but must
  // never name its content, so the locked and unfocused cases say nothing about channels.
  let catWhy = $derived(
    locked
      ? "asleep: the vault is locked"
      : !windowFocused
        ? "asleep: this window isn't focused"
        : mentionChannels.size > 0
          ? `ears up: ${mentionChannels.size} channel${mentionChannels.size === 1 ? "" : "s"} mentioned you (click to go)`
          : activeDownloads > 0
            ? `busy: ${activeDownloads} transfer${activeDownloads === 1 ? "" : "s"} running`
            : "settled: nothing is waiting (click to pet)",
  );

  // A pose the cat holds for a moment after being touched, on top of whatever mood it is in.
  // Transient and purely local: nothing about it is sent, stored, or reflected anywhere.
  let catPose = $state<"" | "stretch" | "startle">("");
  let catPoseTimer: ReturnType<typeof setTimeout> | undefined;
  function holdPose(p: "stretch" | "startle", ms: number) {
    clearTimeout(catPoseTimer);
    catPose = p;
    catPoseTimer = setTimeout(() => (catPose = ""), ms);
  }

  // A purr: low sawtooth under a ~22 Hz tremolo, which is roughly where a real one sits. Built on
  // the same lazily-created context the chimes use, and gated by the same sound preference; a
  // toy that ignores "sounds off" is a bug, however small.
  function playPurr() {
    if (!soundOn) return;
    try {
      audioCtx = audioCtx ?? new AudioContext();
      const ctx = audioCtx;
      if (ctx.state === "suspended") void ctx.resume();
      const now = ctx.currentTime;
      const osc = ctx.createOscillator();
      osc.type = "sawtooth";
      osc.frequency.setValueAtTime(26, now);
      // The rumble sits under a low-pass so it reads as a purr and not as a buzz.
      const filt = ctx.createBiquadFilter();
      filt.type = "lowpass";
      filt.frequency.value = 220;
      // Tremolo: an LFO on the gain is what turns a drone into a purr.
      const lfo = ctx.createOscillator();
      lfo.type = "sine";
      lfo.frequency.value = 22;
      const lfoGain = ctx.createGain();
      lfoGain.gain.value = 0.05;
      const gain = ctx.createGain();
      gain.gain.setValueAtTime(0.0001, now);
      gain.gain.exponentialRampToValueAtTime(0.07, now + 0.12);
      gain.gain.setValueAtTime(0.07, now + 0.62);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.9);
      lfo.connect(lfoGain).connect(gain.gain);
      osc.connect(filt).connect(gain).connect(ctx.destination);
      osc.start(now);
      lfo.start(now);
      osc.stop(now + 0.95);
      lfo.stop(now + 0.95);
    } catch {
      /* no Web Audio here: the cat purrs silently */
    }
  }

  // Touching the cat. When something wants you the mascot is already saying so, so the click
  // follows it: the oldest unread mention (Sets keep insertion order, so the first entry is the
  // one that has been waiting longest). Otherwise there is nowhere to go and it is just a cat.
  function petCat() {
    if (!locked && mentionChannels.size > 0) {
      const target = [...mentionChannels][0];
      void goMention(target);
      return;
    }
    // Asleep and prodded: a startle, no purr. Awake and idle: a stretch and a rumble.
    if (locked || !windowFocused) holdPose("startle", 420);
    else {
      holdPose("stretch", 700);
      playPurr();
    }
  }
  // Following the cat is a navigation like any other, so it goes through the step machinery and
  // Back returns you to whatever you were reading.
  async function goMention(channel: string) {
    navStepStart();
    try {
      switchView("chat");
      await switchTo(channel);
    } finally {
      navStepEnd();
    }
  }

  // ---- ticker view state ----------------------------------------------------
  // An item crawls exactly once and is then consumed: the lane is a notification, not a loop.
  // When the queue empties the slot settles back to the ident line, so a bar that is moving
  // always means something arrived, and a bar that stops means you are caught up.
  let tbShown = $state<Set<string>>(new Set());
  // Newest six unshown: a burst drops its oldest rather than crawling for minutes.
  let tbQueue = $derived(tickerItems.filter((i) => !tbShown.has(i.id)).slice(-6));
  let tbHead = $derived(tbQueue[0] ?? null);
  // The same thresholds the voice stage's meter uses, so the two readings of one mic agree.
  let tbMicBars = $derived([0, 0.25, 0.5, 0.75].filter((t) => micLevel > t).length);
  const tbCrawlDur = (text: string) => Math.min(20, Math.max(6, 5.5 + text.length * 0.1));
  function tbAdvance(id: string) {
    const next = new Set(tbShown);
    next.add(id);
    // Ids that have aged out of the feed can never come back, so the set stays bounded.
    for (const k of next) if (!tickerItems.some((i) => i.id === k)) next.delete(k);
    tbShown = next;
  }

  // ---- window chrome -------------------------------------------------------
  // The OS title bar is off (decorations:false), so the strip at the top of <main> is ours: the
  // empty parts of it are a drag region and these three drive the window. The maximise glyph has
  // to follow the real window state, which also changes by snap, double-click and the OS.
  const appWindow = getCurrentWindow();
  let winMaximized = $state(false);
  const syncMaximized = () => void appWindow.isMaximized().then((m) => (winMaximized = m));

  onMount(() => {
    syncMaximized();
    // Look for a new release shortly after launch rather than during it: the first seconds
    // belong to unlocking and reconnecting, and nothing here is urgent.
    const updateTimer = setTimeout(() => void checkForUpdate(), 4000);
    // Which gate to draw: unlock, or first-run setup. An older backend without the command
    // has a vault by definition (it could only have been reached through the old gate), so a
    // failure falls back to "unlock" rather than offering to found a second identity.
    invoke<boolean>("vault_exists")
      .then((v) => (vaultExists = v))
      .catch(() => (vaultExists = true));
    const subs: Promise<UnlistenFn>[] = [
      appWindow.onResized(() => syncMaximized()),
      appWindow.onFocusChanged(({ payload }) => (windowFocused = payload)),
      listen<{ server: number; channel: string }>("channel-updated", (e) => {
        const { server, channel } = e.payload;
        // Any server's channel changed → the cross-server inbox may have a new entry (debounced).
        scheduleInboxReload();
        // A DM got a message → its activity stats changed; keep the friends sorting fresh.
        if (dmHome && servers.find((x) => x.id === server)?.isDm) refreshDmStats();
        // Jukebox edits ride the same event, and the room I'm listening in need not be the one I'm looking at.
        if (inCall && server === callServer && channel === callChannel) void refreshJukebox();
        if (server === activeServerId && channel === cur?.active) {
          refreshTopic(); // topic edits ride the same channel-updated event
          refresh().then(() => {
            // You're looking at this channel: only chime if the window isn't focused; use the
            // mention chime if the just-arrived (newest) message is aimed at you.
            if (document.hasFocus()) return;
            const last = messages[messages.length - 1];
            const forMe =
              last &&
              last.author !== myFp &&
              (mentionsMe(last.text) || (!!last.reply_to && msgById.get(last.reply_to)?.author === myFp));
            if (forMe) playMention();
            else playNotify();
          });
          return;
        }
        const s = servers.find((x) => x.id === server);
        if (s && s.channels.some((c) => c.id === channel)) {
          if (!s.unread.includes(channel)) s.unread.push(channel);
          if (server !== activeServerId) s.dot = true;
          if (server !== activeServerId) {
            playNotify(); // another server: no per-server identity here to detect a mention
          } else if (mentionChannels.has(channel)) {
            playMention(); // already a known mention channel: new activity is still aimed at me
          } else {
            // A non-active channel of the server I'm in: scan it for a message that @-mentions me or
            // replies to one of mine. A hit gets the distinct mention chime + a badge; else the
            // generic chime. (Already-badged channels are handled above without a re-scan.)
            invoke<Msg[]>("get_messages", { server, channel })
              .then((msgs) => {
                if (server !== activeServerId) return; // switched servers mid-fetch: drop it
                if (targetsMe(channel, msgs)) {
                  if (!mentionChannels.has(channel)) mentionChannels = new Set(mentionChannels).add(channel);
                  playMention();
                } else {
                  playNotify();
                }
              })
              .catch(() => playNotify());
          }
        }
      }),
      listen<{ server: number; count: number }>("members-changed", (e) => {
        if (e.payload.server === activeServerId) {
          refreshMembers();
          if (view === "files") refreshFiles(); // membership change ⇒ re-check fetch availability
        }
      }),
      listen<{ server: number }>("profiles-updated", (e) => {
        if (e.payload.server === activeServerId) refreshProfiles();
      }),
      listen<{ server: number }>("files-updated", (e) => {
        if (e.payload.server === activeServerId) refreshFiles();
      }),
      listen<{
        server: number;
        cid: string;
        done: number;
        total: number;
        provider: string | null;
      }>("download-progress", (e) => {
        const d = downloads[dlKey(e.payload.server, e.payload.cid)];
        if (!d) return; // only track explicitly-initiated downloads
        d.progress = e.payload.total > 0 ? e.payload.done / e.payload.total : 0;
        if (e.payload.done === 0) d.provider = undefined; // fresh transfer: drop any prior provider
        if (e.payload.provider) d.provider = e.payload.provider; // keep the latest live provider
        if (e.payload.done >= e.payload.total) d.status = "done";
        else if (d.status === "queued") d.status = "downloading";
      }),
      listen<{ server: number }>("status-updated", (e) => {
        if (e.payload.server === activeServerId) refreshStatuses();
      }),
      listen<{ server: number }>("wiki-updated", (e) => {
        if (e.payload.server === activeServerId && view === "wiki") refreshWiki();
      }),
      listen<{ server: number }>("roles-updated", (e) => {
        if (e.payload.server === activeServerId) refreshRoles();
      }),
      listen<{ server: number }>("livery-changed", (e) => {
        refreshServerIconFor(e.payload.server); // rail icon may have changed for any server
        if (e.payload.server === activeServerId) refreshLivery();
      }),
      listen<{ server: number; channel: string; states: DeliveryState[] }>("delivery-changed", (e) => {
        if (e.payload.server !== activeServerId || e.payload.channel !== cur?.active) return;
        for (const s of e.payload.states) delivery[s.id] = s;
      }),
      listen<{ server: number }>("badges-changed", (e) => {
        if (e.payload.server === activeServerId) refreshBadges();
      }),
      listen<{ server: number }>("events-changed", (e) => {
        if (e.payload.server === activeServerId) refreshEvents();
      }),
      listen<{ server: number }>("devices-changed", (e) => {
        if (e.payload.server === activeServerId) refreshDevices();
      }),
      listen<{ server: number }>("dm-requests-changed", (e) => {
        // A friend request may have arrived over ANY server (active or not): refresh that server's.
        refreshDmRequests(e.payload.server);
      }),
      listen<{ server: number; from_fp: string; payload: string }>("call-signal", (e) => {
        void handleCallSignal(e.payload.from_fp, e.payload.payload, e.payload.server);
      }),
      listen<{ server: number; online: string[] }>("connectivity-changed", (e) => {
        if (e.payload.server === activeServerId) {
          const next = new Set(e.payload.online);
          const t = Date.now();
          // Record the transitions we witness, so presence detail can show real durations.
          for (const fp of next)
            if (!onlineMembers.has(fp)) {
              onlineSince[fp] = t;
              delete lastSeen[fp];
            }
          for (const fp of onlineMembers)
            if (!next.has(fp)) {
              lastSeen[fp] = t;
              delete onlineSince[fp];
            }
          onlineMembers = next;
          refreshFiles(); // a peer came/went: re-evaluate the availability hint (has_peers)
        }
      }),
      listen<{ server: number; caution: boolean }>("eclipse-changed", (e) => {
        if (e.payload.server === activeServerId) eclipseCaution = e.payload.caution;
      }),
      listen<{ server: number }>("server-closed", (e) => {
        servers = servers.filter((s) => s.id !== e.payload.server);
        if (activeServerId === e.payload.server) {
          activeServerId = servers.length ? servers[0].id : null;
          if (activeServerId !== null) switchServer(activeServerId);
        }
      }),
    ];
    // Global keyboard shortcuts: Escape closes the top-most overlay/menu; Ctrl/Cmd+1–5 switch
    // tabs; Ctrl/Cmd+K opens the quick switcher.
    const onKey = (e: KeyboardEvent) => {
      // Melody unlock: the home row is a piano while the lock screen's melody tab is up.
      if (gateEntry && unlockMethod === "melody" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        const k = e.key.toLowerCase();
        const pc = KEY_TO_PC[k];
        if (pc !== undefined) {
          e.preventDefault();
          if (e.repeat) return; // auto-repeat is one long hold, not a stream of notes
          const note = noteAt(pc);
          keyNotes.set(k, note); // pin it: shifting register mid-hold must still release THIS note
          noteOn(note);
          return;
        }
        if (k === "z" || k === "x") {
          e.preventDefault();
          if (!e.repeat) setOctave(melodyOctave + (k === "x" ? 1 : -1));
          return;
        }
        if (k >= "1" && k <= "7") {
          e.preventDefault();
          if (!e.repeat) setOctave(Number(k));
          return;
        }
        if (e.key === "Backspace") {
          e.preventDefault();
          stopPlayback();
          melodySeq = melodySeq.slice(0, -1);
          return;
        }
        if (e.key === "Enter" && melodySeq.length && !heldNotes.length) {
          e.preventDefault();
          gateSubmit();
          return;
        }
      }
      // The same home row, routed into the call instead of the lock. `!locked` keeps the two
      // apart: while locked the branch above owns these keys and has already returned.
      if (!locked && inCall && instOpen && (stageOpen || focusOpen) && !e.ctrlKey && !e.metaKey && !e.altKey && !typingTarget(e.target)) {
        const k = e.key.toLowerCase();
        const pc = KEY_TO_PC[k];
        if (pc !== undefined) {
          e.preventDefault();
          if (e.repeat) return; // auto-repeat is one long hold, not a stream of notes
          const note = (instOctave + 1) * 12 + pc;
          instKeyNotes.set(k, note); // pinned: z/x mid-hold must still release THIS note
          instNoteOn(note);
          return;
        }
        if (k === "z" || k === "x") {
          e.preventDefault();
          if (!e.repeat) setInstOctave(instOctave + (k === "x" ? 1 : -1));
          return;
        }
      }
      // Alt+arrow walks the location history, as it does in a browser or a file manager.
      if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        e.preventDefault();
        if (e.key === "ArrowLeft") navBack();
        else navForward();
        return;
      }
      if (e.key === "Escape") {
        if (showQuickSwitch) closeQuickSwitch();
        else if (scanOpen) closeScan(null);
        else if (showLinkDevice) closeLinkDevice();
        else if (verifyFor) verifyFor = null;
        else if (menu) menu = null;
        else if (lightbox && fileInfo) closeFileInfo(); // Properties opened over the viewer
        else if (lightbox) closeLightbox();
        else if (reactionPickerFor) reactionPickerFor = "";
        else if (replyingTo) replyingTo = "";
        else if (showEmoji) showEmoji = false;
        else if (showInsert) closeInsert();
        else if (fileInfo) closeFileInfo();
        else if (showWikiHelp) showWikiHelp = false;
        else if (showFeedback) showFeedback = false;
        else if (showServerSettings) showServerSettings = false;
        else if (showSettings) showSettings = false;
        else if (showSearch) closeSearch();
        // A card over the dock: it goes before the surface it was opened from folds away.
        else if (jukePickerOpen) jukePickerOpen = false;
        // Last links: focus covers the window, so it yields the key before the dock does. Both are
        // furniture rather than modals, so they only fold once nothing else on screen wants it.
        else if (focusOpen) exitFocus();
        else if (stageOpen) stageOpen = false;
        // The space folds last: carrying and the tray release first, then the view itself.
        else if (spaceOpen && spaceCarried) spaceCarried = null;
        else if (spaceOpen && spaceTrayPinned) spaceTrayPinned = false;
        else if (spaceOpen) spaceOpen = false;
        return;
      }
      // Hold T while the space is up: the tray of unplaced servers slides out.
      if (spaceOpen && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === "t" && !typingTarget(e.target)) {
        e.preventDefault();
        if (!e.repeat) spaceTrayHeld = true;
        return;
      }
      // Ctrl/Cmd+Shift+F: search with the advanced filter panel already open.
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.shiftKey && e.key.toLowerCase() === "f") {
        if (activeServerId !== null) {
          e.preventDefault();
          view = "chat";
          openSearch(true);
        }
        return;
      }
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
        const tabs: Tab[] = ["chat", "files", "status", "wiki", "profile", "downloads", "events"];
        if (e.key >= "1" && e.key <= "7") {
          e.preventDefault();
          if (activeServerId !== null) switchView(tabs[Number(e.key) - 1]);
        } else if (e.key.toLowerCase() === "l") {
          // Lock: back to the passphrase gate, from anywhere, without touching the node.
          e.preventDefault();
          lockScreen();
        } else if (e.key.toLowerCase() === "k") {
          e.preventDefault();
          openQuickSwitch();
        } else if (e.key.toLowerCase() === "o") {
          // Orbit: the 360 server space, from anywhere inside the unlocked app.
          e.preventDefault();
          toggleSpace();
        } else if (e.key.toLowerCase() === "f") {
          // Search messages in the active conversation (no browser find in the webview).
          if (activeServerId !== null) {
            e.preventDefault();
            view = "chat";
            openSearch();
          }
        }
      }
    };
    // Melody keys are held instruments, not triggers: the lift is what commits the note value.
    const onKeyUp = (e: KeyboardEvent) => {
      const k = e.key.toLowerCase();
      if (k === "t") spaceTrayHeld = false; // the tray is held open, not toggled
      // Release by the pinned note, not by what sits at that key now: the register may have
      // moved (or the drawer closed) between the press and the lift.
      const inst = instKeyNotes.get(k);
      if (inst !== undefined) {
        instKeyNotes.delete(k);
        instNoteOff(inst);
      }
      const note = keyNotes.get(k);
      if (note === undefined) return;
      keyNotes.delete(k);
      noteOff(note);
    };
    // Losing focus mid-hold would otherwise strand a sounding note and an open chord group.
    const onBlur = () => {
      keyNotes.clear();
      releaseAll();
      stopPlayback();
      instKeyNotes.clear();
      instReleaseAll(); // a note stranded here keeps sounding in every other ear in the call
      spaceTrayHeld = false; // the keyup that would close it may land in another window
    };
    // The thumb buttons walk the same history as the title bar's arrows. They arrive as buttons
    // 3 and 4; preventDefault stops the webview from acting on them as well.
    const onMouseNav = (e: MouseEvent) => {
      if (e.button !== 3 && e.button !== 4) return;
      e.preventDefault();
      if (e.button === 3) navBack();
      else navForward();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    window.addEventListener("mousedown", onMouseNav);
    // Keep relative presence times current.
    const tick = setInterval(() => {
      nowTick = Date.now();
      pruneTicker(); // stale news stops being news
    }, 60_000);
    // One slow blink about every half minute, and only while somebody is actually looking.
    const blink = setInterval(() => {
      if (!windowFocused || locked) return;
      catBlink = true;
      setTimeout(() => (catBlink = false), 140);
    }, 30_000);
    // Prune stale voice-room presence so indicators clear when people leave/crash without a "bye",
    // and re-arm the room-active alert for a room that empties out.
    const callCleanup = setInterval(() => {
      const cut = Date.now() - VOICE_STALE_MS;
      let changed = false;
      const next: Record<string, Record<string, number>> = {};
      for (const [key, room] of Object.entries(voiceRooms)) {
        const fresh: Record<string, number> = {};
        for (const [fp, t] of Object.entries(room)) if (t > cut) fresh[fp] = t;
        if (Object.keys(fresh).length) next[key] = fresh;
        else alertedRooms.delete(key);
        if (Object.keys(fresh).length !== Object.keys(room).length) changed = true;
      }
      if (changed) voiceRooms = next;
    }, 4000);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("mousedown", onMouseNav);
      releaseAll();
      stopPlayback();
      clearInterval(tick);
      clearInterval(blink);
      clearInterval(callCleanup);
      clearTimeout(inboxTimer);
      clearTimeout(updateTimer);
      clearInterval(pingTimer);
      subs.forEach((p) => p.then((un) => un()));
    };
  });
</script>

{#snippet styledName(name: string, color: string, font: string, effect: string)}
  <span class="name {fontClass(font)} {fxClass(effect)}" style={colorStyle(color) + fxStyle(effect)}>{name}</span>
{/snippet}

{#snippet nameTag(fp: string)}
  {@const p = profiles[fp]}
  {@render styledName(nameOf(fp), p?.color ?? "", p?.font ?? "", p?.effect ?? "")}
{/snippet}

{#snippet avatarTag(fp: string)}
  {@const p = profiles[fp]}
  {#if p?.avatar}
    <img class="avatar" src={imgSrc(p.avatar)} alt="" />
  {:else}
    <span class="avatar fallback" style={p?.color ? `background:${p.color}` : ""}>
      {nameOf(fp).slice(0, 1).toUpperCase()}
    </span>
  {/if}
{/snippet}

<!-- The profile editor, rendered by BOTH the profile surface (Ctrl+5) and Settings → My
     Profile: one form, two doors, so the two can never drift apart. -->
{#snippet profileEditor()}
  <div class="profile-tab tab-pane">
    <div class="field">
      <span class="muted">Banner</span>
      {#if pBanner}
        <img class="banner-preview" src={imgSrc(pBanner)} alt="" />
      {/if}
      <div class="avatar-row">
        <label class="upload-btn">
          {pBanner ? "Replace banner" : "Upload banner"}
          <input type="file" accept="image/*" onchange={(e) => { const t = e.currentTarget; void loadBanner(t.files).then(() => (t.value = "")); }} />
        </label>
        {#if pBanner}
          <button type="button" class="ghost" onclick={() => (pBanner = "")}>Remove</button>
        {/if}
      </div>
      <span class="muted small">Tops your profile card. A small animated GIF or WebP stays animated.</span>
    </div>
    <label class="field">
      <span class="muted">Name</span>
      <input bind:value={pName} placeholder="display name" />
    </label>
    <div class="field">
      <span class="muted">Font</span>
      <div class="ns-tiles">
        {#each NAME_FONTS as f}
          <button
            type="button"
            class="ns-tile"
            class:active={pFont === f.id}
            title={f.label}
            aria-label={f.label}
            aria-pressed={pFont === f.id}
            onclick={() => (pFont = f.id)}
          ><span class="name {fontClass(f.id)}">Gg</span></button>
        {/each}
      </div>
    </div>
    <div class="field">
      <span class="muted">Effect</span>
      <div class="ns-tiles">
        {#each NAME_EFFECTS as fx}
          {@const dead = fxMotionOff && ANIM_FX.has(fx.id)}
          <button
            type="button"
            class="ns-tile"
            class:active={pEffect === fx.id}
            class:motion-dead={dead}
            title={dead ? `${fx.label}: this one animates, and motion is off (Appearance: Hover motion, or the system's reduced-motion)` : fx.label}
            aria-label={fx.label}
            aria-pressed={pEffect === fx.id}
            onclick={() => (pEffect = fx.id)}
          ><span class="name {fxClass(fx.id)}" style={colorStyle(pColor)}>{fx.label}</span></button>
        {/each}
        <button
          type="button"
          class="ns-tile"
          class:active={GRAD2_RE.test(pEffect)}
          title="Custom gradient: your stops, your angle"
          aria-label="Custom gradient"
          aria-pressed={GRAD2_RE.test(pEffect)}
          onclick={() => (pEffect = grad2Id())}
        ><span class="name fx-grad2" style={`color:${pGradStops[0]}` + fxStyle(grad2Id())}>Gradient</span></button>
      </div>
      {#if GRAD2_RE.test(pEffect)}
        <div class="grad-maker">
          {#each pGradStops as stop, si (si)}
            <span class="grad-stop">
              <input type="color" value={stop} aria-label={`Gradient stop ${si + 1}`} oninput={(e) => { pGradStops[si] = e.currentTarget.value; pEffect = grad2Id(); }} />
              {#if pGradStops.length > 2}
                <button type="button" class="grad-del" title="Remove this stop" aria-label={`Remove gradient stop ${si + 1}`} onclick={() => { pGradStops.splice(si, 1); pEffect = grad2Id(); }}>✕</button>
              {/if}
            </span>
          {/each}
          {#if pGradStops.length < GRAD_MAX_STOPS}
            <button type="button" class="ghost small" onclick={() => { pGradStops.push(pGradStops[pGradStops.length - 1]); pEffect = grad2Id(); }}>＋ stop</button>
          {/if}
          <input type="range" min="0" max="360" step="15" value={pGradDeg} aria-label="Gradient angle" oninput={(e) => { pGradDeg = +e.currentTarget.value; pEffect = grad2Id(); }} />
          <span class="muted small">{pGradDeg}°</span>
        </div>
        <div class="grad-maker">
          <span class="muted small">Scroll</span>
          <input type="range" min="0" max="10" step="1" value={pGradSpeed} aria-label="Gradient scroll speed" oninput={(e) => { pGradSpeed = +e.currentTarget.value; pEffect = grad2Id(); }} />
          <button type="button" class="ghost small" disabled={!pGradSpeed} onclick={() => { pGradRev = !pGradRev; pEffect = grad2Id(); }}>{pGradRev ? "◀ reverse" : "▶ forward"}</button>
          <span class="muted small">{pGradSpeed ? `speed ${pGradSpeed}` : "still"}</span>
        </div>
        <span class="muted small">Up to {GRAD_MAX_STOPS} stops. Scroll follows the gradient's angle{fxMotionOff ? " (motion is off, so it holds still for you)" : ""}. Builds that predate gradients show your flat colour instead.</span>
      {/if}
    </div>
    <div class="field">
      <span class="muted">Colour</span>
      <div class="ns-swatches">
        <input type="color" bind:value={pColor} aria-label="Custom name colour" />
        {#each NAME_COLORS as c}
          <button
            type="button"
            class="ns-swatch"
            class:active={pColor === c}
            title={c}
            aria-label={`Name colour ${c}`}
            aria-pressed={pColor === c}
            style={`background:${c}`}
            onclick={() => (pColor = c)}
          ></button>
        {/each}
      </div>
    </div>
    <label class="field">
      <span class="muted">About you</span>
      <textarea bind:value={pDescription} rows="3" maxlength="280" placeholder="A short bio shown on your profile card…"></textarea>
    </label>
    <div class="field">
      <span class="muted">Message bubble</span>
      <div class="bubble-presets">
        {#each BUBBLE_PRESETS as b}
          <button
            type="button"
            class="bubble-swatch"
            class:active={pBubble === b.value}
            title={b.label}
            style={b.value ? `background:${b.value}` : ""}
            onclick={() => (pBubble = b.value)}
          >{#if !b.value}Aa{/if}</button>
        {/each}
        <button
          type="button"
          class="bubble-swatch"
          class:active={pBubble === customBubble()}
          title="Custom gradient"
          style={`background:${customBubble()}`}
          onclick={() => (pBubble = customBubble())}
        ></button>
      </div>
      {#if pBubble === customBubble()}
        <div class="grad-maker">
          <input type="color" value={pBubA} aria-label="Bubble gradient start colour" oninput={(e) => { pBubA = e.currentTarget.value; pBubble = customBubble(); }} />
          <input type="color" value={pBubB} aria-label="Bubble gradient end colour" oninput={(e) => { pBubB = e.currentTarget.value; pBubble = customBubble(); }} />
        </div>
        <span class="muted small">Keep it dark enough to read white text on: a text shadow backs it up, but not by much.</span>
      {/if}
    </div>
    <div class="field">
      <span class="muted">Avatar</span>
      <div class="avatar-row">
        {#if pAvatar}
          <img class="avatar lg" src={imgSrc(pAvatar)} alt="" />
        {:else}
          <span class="avatar lg fallback" style={`background:${pColor}`}>
            {(pName || displayName).slice(0, 1).toUpperCase()}
          </span>
        {/if}
        <input type="file" accept="image/*" onchange={(e) => loadAvatar(e.currentTarget.files)} />
        {#if pAvatar}
          <button type="button" class="ghost" onclick={() => (pAvatar = "")}>Remove</button>
        {/if}
      </div>
      <span class="muted small">A GIF or WebP under 64KiB keeps its animation; anything else becomes a 128px square.</span>
    </div>
    <p class="preview">
      Preview: {@render styledName(pName || displayName, pColor, pFont, pEffect)}
    </p>
    <button onclick={saveProfile}>Save profile</button>
  </div>
{/snippet}

<!-- The settings live preview: the REAL message-log markup at miniature scale, fed by the
     profile DRAFT, so it can never drift from the log and every knob (density, text size,
     clock, flatten, bubble, name style) applies the moment you turn it. -->
{#snippet previewLog()}
  {@const pv = appearance.flat || !pBubble ? "" : `background:${pBubble}`}
  <ul class="messages stx-plog">
    <li class:has-bubble={!!pv} style={pv}>
      <span class="t">
        <span class="gutter-avatar">
          {#if pAvatar}
            <img class="avatar" src={imgSrc(pAvatar)} alt="" />
          {:else}
            <span class="avatar fallback" style={`background:${pColor}`}>{(pName || displayName).slice(0, 1).toUpperCase()}</span>
          {/if}
        </span>
      </span>
      <div class="m-body">
        <span class="author">
          <span class="author-link">{@render styledName(pName || displayName, pColor, pFont, pEffect)}</span>
          {#if myFp && badges[myFp]}
            {@const b = badges[myFp]}
            <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""}>{b.label}</span>
          {/if}
          <span class="time">{fmtTime(Date.now())}</span>
        </span>
        <span class="text">tea is ready when you are <span class="mention mention-me">@you</span></span>
      </div>
    </li>
    <li class="grouped" class:has-bubble={!!pv} style={pv}>
      <span class="t">{fmtTime(Date.now())}</span>
      <div class="m-body"><span class="text">bringing biscuits too</span></div>
    </li>
  </ul>
{/snippet}

<!-- One roster row in the member column (rendered under the online / offline group heads). -->
{#snippet memberRow(m: Member, online: boolean)}
  <li
    title={m.fingerprint}
    class:is-you={m.you}
    class="member-row"
    use:contextMenu={() => memberMenu(m)}
  >
    <span class="presence" class:online title={presenceText(m.fingerprint, m.you)}>●</span>
    <button type="button" class="member-link" onclick={() => showProfile(m.fingerprint)}>
      {@render avatarTag(m.fingerprint)}
      {@render nameTag(m.fingerprint)}
    </button>
    {#if !m.you && verifiedFps.has(m.fingerprint)}
      <span class="vf-check" title="You verified this member out of band">✓</span>
    {/if}
    {#if badges[m.fingerprint]}
      {@const b = badges[m.fingerprint]}
      <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""} title="Badge assigned by a server admin">{b.label}</span>
    {/if}
    {#if roles[m.fingerprint] && roles[m.fingerprint] !== "member"}
      <span class="role-badge {roles[m.fingerprint]}" title={roles[m.fingerprint]}>{roleAbbr(roles[m.fingerprint])}</span>
    {/if}
    {#if m.you}<span class="you-badge">you</span>{/if}
    {#if !m.you && !online && lastSeen[m.fingerprint]}
      <span class="last-seen" title={presenceText(m.fingerprint, false)}>{relTime(nowTick - lastSeen[m.fingerprint])}</span>
    {/if}
  </li>
{/snippet}

<!-- A member's linked devices, nested under their roster row (multi-device M4). -->
{#snippet companionRows(originFp: string)}
  {#each Object.entries(deviceMap).filter(([, d]) => d.origin === originFp) as [cfp, d] (cfp)}
    {@const conline = onlineMembers.has(cfp)}
    <li class="member-row companion" title={cfp}>
      <span class="presence" class:online={conline} title={conline ? "Device online" : "Device offline"}>●</span>
      <span class="dev-tag">· {d.name}</span>
    </li>
  {/each}
{/snippet}

<!-- Inline line-icons: stroke follows currentColor, so active/hover states tint them via the button's color. -->
{#snippet icoDm()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M3.5 5.5h11.5v8H8.5l-4 3.2v-3.2h-1z" />
    <path d="M18 9.5h2.5v7.7l-3.3-2.4H11" />
  </svg>
{/snippet}

{#snippet icoInbox()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M3.5 13.5 6 5.5h12l2.5 8v5H3.5z" />
    <path d="M3.5 13.5h5l1.8 2.7h3.4l1.8-2.7h5" />
  </svg>
{/snippet}

<!-- The brand cat, drawn down from the logo's own geometry (assets/cat/icon-cat.svg): same ear
     angle, same chubby head, same happy closed eyes. -->
{#snippet icoCat()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M5.6 9.1 6.1 4.2l4.2 2.9c.55-.1 1.1-.15 1.7-.15s1.15.05 1.7.15l4.2-2.9.5 4.9c.9 1.2 1.4 2.7 1.4 4.3 0 4.2-3.5 7.4-7.8 7.4s-7.8-3.2-7.8-7.4c0-1.6.5-3.1 1.4-4.3Z" />
    <path d="M7.1 11.8c.5 1.05 2.4 1.05 2.9 0M14 11.8c.5 1.05 2.4 1.05 2.9 0" />
    <path d="M11.2 13.7h1.6l-.8 1.1Z" fill="currentColor" stroke="none" />
    <path d="M10.7 15.9c.8 0 1.3-.4 1.3-1.1 0 .7.5 1.1 1.3 1.1" />
    <path d="M8.2 15.1 5.9 14.55M8.2 16 6 16.5M15.8 15.1 18.1 14.55M15.8 16 18 16.5" stroke-width="1.2" />
  </svg>
{/snippet}

{#snippet icoPlus()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M12 5.4v13.2M5.4 12h13.2" />
  </svg>
{/snippet}

{#snippet icoSearch()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="10.5" cy="10.5" r="6.2" />
    <path d="M15.1 15.1 20.4 20.4" />
  </svg>
{/snippet}

<!-- A member typeahead for the advanced-search panel: type to narrow, ↑/↓/Enter or click to pick,
     empty the box to drop the filter. `current` is the selected fingerprint, `set` writes it back. -->
{#snippet personPicker(p: Picker, current: string, set: (fp: string) => void, placeholder: string)}
  {@const opts = pickerOptions(p)}
  <span class="sa-picker">
    <input
      class="sa-person"
      class:on={!!current}
      {placeholder}
      value={p.q}
      autocomplete="off"
      spellcheck="false"
      oninput={(e) => onPickerInput(p, e.currentTarget.value, current, set)}
      onfocus={() => (p.open = true)}
      onblur={() => (p.open = false)}
      onkeydown={(e) => onPickerKey(e, p, set)}
    />
    {#if current}
      <button
        class="sa-person-clear"
        type="button"
        title="Clear"
        aria-label="Clear"
        onclick={() => { set(""); p.q = ""; refilter(); }}
      >✕</button>
    {/if}
    {#if p.open && opts.length}
      <ul class="sa-options" role="listbox">
        {#each opts as o, i (o.fp)}
          <li>
            <button
              type="button"
              class="sa-option"
              class:active={i === p.idx}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => choosePerson(p, o, set)}
            >{o.name}</button>
          </li>
        {/each}
      </ul>
    {/if}
  </span>
{/snippet}

<!-- One toggle in the advanced-search panel; `set` writes back into `filters`. -->
{#snippet fchip(label: string, on: boolean, set: (v: boolean) => void, hint: string)}
  <button
    type="button"
    class="fchip"
    class:on
    title={hint}
    aria-pressed={on}
    onclick={() => { set(!on); refilter(); }}
  >
    {label}
  </button>
{/snippet}

{#snippet icoWrench()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
  </svg>
{/snippet}

{#snippet icoPin()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M20 10c0 5.6-8 12-8 12s-8-6.4-8-12a8 8 0 0 1 16 0z" />
    <circle cx="12" cy="10" r="2.9" />
  </svg>
{/snippet}

{#snippet icoPhone()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M21.5 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 1.6 4.2a2 2 0 0 1 2-2.2h3a2 2 0 0 1 2 1.7c.13.96.36 1.9.7 2.8a2 2 0 0 1-.45 2.1L7.6 9.9a16 16 0 0 0 6 6l1.2-1.2a2 2 0 0 1 2.1-.45c.9.34 1.84.57 2.8.7a2 2 0 0 1 1.8 2.05z" />
  </svg>
{/snippet}

{#snippet icoHangup()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M21.5 16.9v3a2 2 0 0 1-2.2 2 19.8 19.8 0 0 1-8.6-3.1 19.5 19.5 0 0 1-6-6A19.8 19.8 0 0 1 1.6 4.2a2 2 0 0 1 2-2.2h3a2 2 0 0 1 2 1.7c.13.96.36 1.9.7 2.8a2 2 0 0 1-.45 2.1L7.6 9.9a16 16 0 0 0 6 6l1.2-1.2a2 2 0 0 1 2.1-.45c.9.34 1.84.57 2.8.7a2 2 0 0 1 1.8 2.05z" />
    <path d="M21 3 3.4 20.6" />
  </svg>
{/snippet}

{#snippet icoGear()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="5.2" />
    <circle cx="12" cy="12" r="2" />
    <path d="M12 6.8V4M12 17.2V20M6.8 12H4M17.2 12H20M8.32 8.32 6.34 6.34M15.68 15.68 17.66 17.66M15.68 8.32 17.66 6.34M8.32 15.68 6.34 17.66" />
  </svg>
{/snippet}

<!-- The mark that fronts the gate: logo, wordmark, and one line of context for the screen. -->
{#snippet brandMark(sub: string)}
  <div class="brand">
    <img class="brand-logo" src={logoUrl} alt="" draggable="false" />
    <span class="brand-name">Mewtual</span>
    {#if sub}<span class="brand-sub">{sub}</span>{/if}
  </div>
{/snippet}

{#snippet icoFeedback()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M4 4.5h16v12H9.6L5 20.2v-3.7H4z" />
    <path d="M12 7.8v4.1" />
    <circle cx="12" cy="14.2" r="0.85" fill="currentColor" stroke="none" />
  </svg>
{/snippet}

{#snippet icoOrbit()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="4.6" />
    <ellipse cx="12" cy="12" rx="9.4" ry="3.6" transform="rotate(-18 12 12)" />
    <circle cx="19.4" cy="8.2" r="1.1" fill="currentColor" stroke="none" />
  </svg>
{/snippet}

<!-- One 90-degree wall of a preset space backdrop. All fills ride the theme tokens, so
     presets and accent overrides recolor the room itself; nothing here is a literal hex. -->
{#snippet spaceWall(b: string, fy: number)}
  {#if b === "den"}
    {#if fy === 0}
      <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <rect x="30" y="26" width="40" height="40" rx="1.2" style="fill: color-mix(in oklab, var(--bg-0) 55%, black); stroke: var(--border); stroke-width: 0.7" />
        <path d="M50 26v40M30 46h40" style="stroke: var(--border); stroke-width: 0.7; fill: none" />
        <circle cx="58" cy="36" r="4.4" style="fill: var(--text-2); opacity: 0.85" />
        <circle cx="55.8" cy="34.5" r="0.9" style="fill: var(--muted); opacity: 0.6" />
        <circle cx="36" cy="32" r="0.5" style="fill: var(--text-2); opacity: 0.7" />
        <circle cx="42" cy="56" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <circle cx="65" cy="59" r="0.45" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="33" cy="61" r="0.4" style="fill: var(--text-2); opacity: 0.45" />
        <rect x="28.5" y="66" width="43" height="2.4" rx="0.5" style="fill: var(--bg-elev); stroke: var(--border); stroke-width: 0.4" />
      </svg>
      <div class="sp-cat">{@html catSleepArt}</div>
    {:else if fy === 90}
      <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <rect x="37" y="36" width="26" height="17" rx="1" style="fill: color-mix(in oklab, var(--bg-0) 55%, black); stroke: var(--border); stroke-width: 0.7" />
        <rect x="41" y="40" width="12" height="1.5" rx="0.6" style="fill: var(--accent); opacity: 0.55" />
        <rect x="41" y="43.6" width="17" height="1.5" rx="0.6" style="fill: var(--accent); opacity: 0.3" />
        <rect x="41" y="47.2" width="8" height="1.5" rx="0.6" style="fill: var(--accent); opacity: 0.4" />
        <rect x="48" y="53" width="4" height="3.2" style="fill: color-mix(in oklab, var(--bg-elev) 70%, var(--bg-0))" />
        <rect x="30" y="56.2" width="40" height="2.6" rx="0.7" style="fill: var(--bg-elev); stroke: var(--border); stroke-width: 0.4" />
        <rect x="32.5" y="58.8" width="1.8" height="13" style="fill: color-mix(in oklab, var(--bg-elev) 70%, var(--bg-0))" />
        <rect x="65.7" y="58.8" width="1.8" height="13" style="fill: color-mix(in oklab, var(--bg-elev) 70%, var(--bg-0))" />
        <path d="M64 55.4q0.8-2.6 3-1.8q0.5 1.3-0.9 1.8z" style="fill: var(--bg-elev); stroke: var(--border); stroke-width: 0.3" />
      </svg>
    {:else if fy === 180}
      <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <rect x="32" y="28" width="36" height="46" rx="1" style="fill: color-mix(in oklab, var(--panel) 65%, var(--bg-0)); stroke: var(--border); stroke-width: 0.7" />
        <path d="M32 44h36M32 60h36" style="stroke: var(--border); stroke-width: 0.7; fill: none" />
        <rect x="35" y="35.5" width="2.6" height="8.5" style="fill: color-mix(in oklab, var(--accent) 40%, var(--bg-elev))" />
        <rect x="38.5" y="33.8" width="2.2" height="10.2" style="fill: color-mix(in oklab, var(--muted) 35%, var(--bg-elev))" />
        <rect x="41.6" y="36.4" width="2.6" height="7.6" style="fill: color-mix(in oklab, var(--accent) 20%, var(--bg-elev))" />
        <rect x="45.2" y="34.6" width="2" height="9.4" style="fill: color-mix(in oklab, var(--text-2) 25%, var(--bg-elev))" />
        <rect x="53" y="37.6" width="8.4" height="6.4" rx="0.8" style="fill: var(--bg-elev); stroke: var(--border); stroke-width: 0.4" />
        <path d="M37 56.6q-1.4-3.4 2-3.4h4.4q3.4 0 2 3.4q-1 2-2.5 0.5h-3.4q-1.5 1.5-2.5-0.5z" style="fill: var(--bg-elev); stroke: var(--border); stroke-width: 0.3" />
        <rect x="52" y="63" width="7.5" height="9" style="fill: color-mix(in oklab, var(--muted) 25%, var(--bg-elev)); opacity: 0.85" />
        <rect x="38" y="66" width="10" height="6" style="fill: color-mix(in oklab, var(--accent) 25%, var(--bg-elev)); opacity: 0.85" />
      </svg>
    {:else}
      <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <rect x="42" y="30" width="16" height="42" style="fill: var(--panel); stroke: var(--border); stroke-width: 0.7" />
        <circle cx="44.5" cy="52" r="0.9" style="fill: var(--faint)" />
        <path d="M64 65l5 0l-0.9 7l-3.2 0z" style="fill: color-mix(in oklab, var(--bg-elev) 70%, var(--bg-0)); stroke: var(--border); stroke-width: 0.3" />
        <path d="M66.5 65q-3.4-5.4-6.4-5.9q4-1.5 6.4 2.4q1-5.9 4.4-6.9q-1.5 4.4-3 7.4q3.4-2.4 5.4-1q-3 2.5-5.9 4z" style="fill: color-mix(in oklab, var(--ok) 22%, var(--bg-elev))" />
      </svg>
    {/if}
  {:else if b === "ridge"}
    <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      {#if fy === 0}
        <circle cx="66" cy="30" r="5" style="fill: var(--text-2); opacity: 0.8" />
        <circle cx="30" cy="24" r="0.5" style="fill: var(--text-2); opacity: 0.7" />
        <circle cx="48" cy="34" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <path d="M0 66L18 48L34 62L52 42L72 60L88 52L100 60L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 75%, var(--bg-0))" />
        <path d="M0 74L24 62L50 72L78 64L100 72L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 35%, var(--bg-0))" />
      {:else if fy === 90}
        <circle cx="38" cy="26" r="0.5" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="70" cy="34" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <path d="M0 62L22 44L46 60L68 38L88 58L100 50L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 75%, var(--bg-0))" />
        <path d="M0 76L30 64L64 74L100 66L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 35%, var(--bg-0))" />
      {:else if fy === 180}
        <circle cx="26" cy="30" r="0.5" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="58" cy="22" r="0.4" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="84" cy="36" r="0.4" style="fill: var(--text-2); opacity: 0.4" />
        <path d="M0 58L26 50L44 58L70 46L100 62L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 75%, var(--bg-0))" />
        <path d="M0 78Q50 70 100 78L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 35%, var(--bg-0))" />
        <path d="M20 82Q50 78 80 82" style="stroke: var(--text-2); stroke-width: 0.4; opacity: 0.25; fill: none" />
      {:else}
        <circle cx="44" cy="28" r="0.5" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="76" cy="24" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <path d="M0 54L20 46L42 64L66 44L84 60L100 56L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 75%, var(--bg-0))" />
        <path d="M0 72L36 62L70 76L100 68L100 100L0 100Z" style="fill: color-mix(in oklab, var(--panel) 35%, var(--bg-0))" />
      {/if}
    </svg>
  {:else}
    <svg class="sp-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      {#if fy === 0}
        <circle cx="64" cy="36" r="4.2" style="fill: var(--bg-elev); stroke: color-mix(in oklab, var(--accent) 45%, var(--bg-elev)); stroke-width: 0.4" />
        <ellipse cx="64" cy="36" rx="7.4" ry="2" transform="rotate(-18 64 36)" style="stroke: var(--accent); stroke-width: 0.4; opacity: 0.55; fill: none" />
        <circle cx="30" cy="28" r="0.5" style="fill: var(--text-2); opacity: 0.75" />
        <circle cx="44" cy="50" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <circle cx="76" cy="58" r="0.45" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="52" cy="24" r="0.35" style="fill: var(--text-2); opacity: 0.45" />
      {:else if fy === 90}
        <circle cx="34" cy="34" r="0.5" style="fill: var(--text-2); opacity: 0.7" />
        <circle cx="58" cy="26" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <circle cx="70" cy="48" r="0.5" style="fill: var(--text-2); opacity: 0.65" />
        <circle cx="46" cy="60" r="0.35" style="fill: var(--text-2); opacity: 0.4" />
        <circle cx="26" cy="52" r="0.4" style="fill: var(--text-2); opacity: 0.55" />
      {:else if fy === 180}
        <circle cx="40" cy="30" r="0.5" style="fill: var(--text-2); opacity: 0.7" />
        <circle cx="64" cy="40" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <circle cx="30" cy="56" r="0.45" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="74" cy="60" r="0.35" style="fill: var(--text-2); opacity: 0.45" />
        <path d="M20 20l2.6 0M21.3 18.7l0 2.6" style="stroke: var(--text-2); stroke-width: 0.3; opacity: 0.5" />
      {:else}
        <circle cx="50" cy="28" r="0.5" style="fill: var(--text-2); opacity: 0.7" />
        <circle cx="28" cy="42" r="0.4" style="fill: var(--text-2); opacity: 0.5" />
        <circle cx="66" cy="52" r="0.45" style="fill: var(--text-2); opacity: 0.6" />
        <circle cx="80" cy="30" r="0.35" style="fill: var(--text-2); opacity: 0.45" />
      {/if}
    </svg>
  {/if}
{/snippet}

{#snippet icoLock()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="4.6" y="10.4" width="14.8" height="10" rx="2.2" />
    <path d="M8.2 10.4V7.6a3.8 3.8 0 0 1 7.6 0v2.8" />
    <path d="M12 14v2.8" />
  </svg>
{/snippet}

{#snippet icoSpeaker()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M4 9.4h3.4L12 5.4v13.2L7.4 14.6H4z" />
    <path d="M15.7 9.2a4 4 0 0 1 0 5.6" />
  </svg>
{/snippet}

{#snippet icoMic()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="9.2" y="2.6" width="5.6" height="10.4" rx="2.8" />
    <path d="M5.8 11.2a6.2 6.2 0 0 0 12.4 0" />
    <path d="M12 17.4V20.6M9 20.6h6" />
  </svg>
{/snippet}

{#snippet icoMicOff()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="9.2" y="2.6" width="5.6" height="10.4" rx="2.8" />
    <path d="M5.8 11.2a6.2 6.2 0 0 0 12.4 0" />
    <path d="M12 17.4V20.6M9 20.6h6" />
    <path d="M3.4 3.4 20.6 20.6" />
  </svg>
{/snippet}

{#snippet icoNote()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M9.4 17.6V4.9l9.4-1.9v12.7" />
    <circle cx="6.8" cy="17.6" r="2.6" />
    <circle cx="16.2" cy="15.7" r="2.6" />
  </svg>
{/snippet}

<!-- Transport glyphs for the jukebox deck. Solid fills: these are presses, not states. -->
{#snippet icoPlay()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M7.8 4.6 19 12 7.8 19.4z" fill="currentColor" stroke="none" />
  </svg>
{/snippet}

{#snippet icoPause()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M8.8 5v14M15.2 5v14" />
  </svg>
{/snippet}

{#snippet icoSkip()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M5.6 5.2 15 12 5.6 18.8z" fill="currentColor" stroke="none" />
    <path d="M18 5.2v13.6" />
  </svg>
{/snippet}

{#snippet icoCam()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="2.6" y="6" width="12.6" height="12" rx="2.4" />
    <path d="M15.2 11.2 21.4 7.6v8.8l-6.2-3.6z" />
  </svg>
{/snippet}

{#snippet icoScreen()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="2.6" y="4.2" width="18.8" height="12.6" rx="2.2" />
    <path d="M12 16.8v3.6M8.4 20.4h7.2" />
  </svg>
{/snippet}

<!-- Corners pulling inward: the universal "give the window back" glyph. -->
{#snippet icoFocusOut()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M9.6 3.4v6.2H3.4M14.4 3.4v6.2h6.2M9.6 20.6v-6.2H3.4M14.4 20.6v-6.2h6.2" />
  </svg>
{/snippet}

{#snippet icoChevUp()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M6 15.2 12 9.2l6 6" />
  </svg>
{/snippet}

{#snippet icoChevDown()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M6 9.2 12 15.2l6-6" />
  </svg>
{/snippet}

<!--
  Ears on a face. The same silhouette the window wears, perked by how loud that voice is right
  now: the speaking ring already says WHETHER someone is talking, so these say HOW MUCH, off the
  identical RMS reading. Both come from `earPerk`, so the ring and the ears can never disagree.
  Decoration with a job: in a busy room the twitch finds the live speaker faster than a border.
-->
{#snippet catEars(key: string)}
  <span class="cat-ears" data-perk={earPerk[key] ?? 0} aria-hidden="true">{@html earsSvg}</span>
{/snippet}

<!-- Live mic level: four bars lit from micLevel. Muted reads as empty, never as "quiet". -->
{#snippet micMeter()}
  <span class="stage-meter" title={callMuted ? "Your mic is muted" : "Your mic level"} aria-hidden="true">
    {#each [0, 1, 2, 3] as i}
      <i class="stage-bar" class:lit={!callMuted && micLevel > i * 0.25}></i>
    {/each}
  </span>
{/snippet}

<!--
  The instrument drawer, shared by both call surfaces: the stage docks it under the self block,
  the focus view docks it under the control bar. One copy so the two can never drift, and so a
  note held while switching surfaces is still the same held note.
-->
{#snippet instDrawer()}
  <div class="inst-drawer">
    <div class="inst-head">
      <span class="inst-head-ico">{@render icoNote()}</span>
      <span class="stage-label">INSTRUMENTS</span>
      <span class="stage-spacer"></span>
      {#if midiName}<span class="inst-midi">MIDI · {midiName}</span>{/if}
    </div>

    <div class="inst-ctl">
      {#each INST_TILES as t (t.wave)}
        <button
          class="ghost inst-wave"
          class:on={myTimbre === t.wave}
          aria-pressed={myTimbre === t.wave}
          title={`Send your notes as a ${t.wave} wave`}
          onclick={() => setTimbre(t.wave)}
        >
          <svg class="inst-wv" viewBox="0 0 26 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d={t.d} />
          </svg>
          <span class="inst-wave-lbl">{t.label}</span>
        </button>
      {/each}
      <span class="stage-spacer"></span>
      <button class="ghost small inst-oct-btn" title="Register down (z)" aria-label="Register down" onclick={() => setInstOctave(instOctave - 1)}>−</button>
      <span class="inst-oct">C{instOctave}–C{instOctave + 2}</span>
      <button class="ghost small inst-oct-btn" title="Register up (x)" aria-label="Register up" onclick={() => setInstOctave(instOctave + 1)}>＋</button>
    </div>

    <!-- 25 keys from the register base. Mine wins the tint over a peer's: I have to be
         able to see what I am playing even while someone else holds the same note. -->
    <div class="inst-board">
      <div class="piano inst-piano">
        {#each instKeys as note (note)}
          {@const pc = note % 12}
          {@const rfp = callHeld.includes(note) ? "" : instHolder.get(note) ?? ""}
          <button
            type="button"
            class="piano-key"
            class:sharp={PC_SHARP[pc]}
            class:held={callHeld.includes(note)}
            class:rheld={!!rfp}
            style={rfp ? `background:${instColor(rfp)};border-color:${instColor(rfp)};color:#131218` : ""}
            title={rfp ? `${noteName(note)} · ${nameOf(rfp)}` : noteName(note)}
            onpointerdown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); instNoteOn(note); }}
            onpointerup={() => instNoteOff(note)}
            onpointercancel={() => instNoteOff(note)}
          >{pc === 0 ? noteName(note) : NOTE_NAMES[pc]}</button>
        {/each}
      </div>
      {#if instEdges.below.length}
        <div class="inst-edge low">
          {#each instEdges.below.slice(0, 3) as ed (ed.note)}
            <span class="inst-pill" style={`background:${instColor(ed.fp)};color:#131218`} title={`${nameOf(ed.fp)} is playing ${noteName(ed.note)}, below this register`}>
              <svg class="inst-tri" viewBox="0 0 8 8" aria-hidden="true"><path d="M6.4 0.8 0.9 4l5.5 3.2z" fill="currentColor" /></svg>
              {noteName(ed.note)}
            </span>
          {/each}
          {#if instEdges.below.length > 3}<span class="inst-pill more">+{instEdges.below.length - 3}</span>{/if}
        </div>
      {/if}
      {#if instEdges.above.length}
        <div class="inst-edge high">
          {#each instEdges.above.slice(0, 3) as ed (ed.note)}
            <span class="inst-pill" style={`background:${instColor(ed.fp)};color:#131218`} title={`${nameOf(ed.fp)} is playing ${noteName(ed.note)}, above this register`}>
              {noteName(ed.note)}
              <svg class="inst-tri" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.6 0.8 7.1 4l-5.5 3.2z" fill="currentColor" /></svg>
            </span>
          {/each}
          {#if instEdges.above.length > 3}<span class="inst-pill more">+{instEdges.above.length - 3}</span>{/if}
        </div>
      {/if}
    </div>

    <!-- Now playing: the audible truth, spelled out. Mine first, then everyone else's. -->
    <div class="inst-now">
      {#if instNowMine.length}
        <span class="inst-who">
          <span class="inst-sw" style="background:var(--accent)"></span>
          <span class="inst-nm">you</span>
          <span class="inst-sep">·</span>
          <span class="inst-notes">{instNowMine.map(noteName).join(" ")}</span>
          {#if instNowMine.length > 1}
            <span class="inst-sep">·</span>
            <span class="inst-chord">{chordName(instNowMine)}</span>
          {/if}
        </span>
      {/if}
      {#each instNowPeers as p (p.fp)}
        <span class="inst-who">
          <span class="inst-sw" style={`background:${instColor(p.fp)}`}></span>
          <span class="inst-nm">{nameOf(p.fp)}</span>
          <span class="inst-sep">·</span>
          <span class="inst-notes">{p.notes.map(noteName).join(" ")}</span>
        </span>
      {/each}
    </div>

    <div class="inst-hint">a w s e d f t g y h u j play · z/x shift · click keys or midi</div>
  </div>
{/snippet}

<!--
  The jukebox dock, shared by both call surfaces the same way instDrawer is: the stage docks it
  under the self block, the focus view docks it above the instruments. One copy, because the deck
  is one deck: the room hears a single track and the two surfaces must never spell it differently.
  Everything here reads the transport and presses it; none of it owns any of it.
-->
{#snippet jukeDock()}
  <div class="juke-dock">
    <div class="juke-head">
      <span class="juke-head-ico">{@render icoNote()}</span>
      <span class="stage-label">JUKEBOX</span>
      <!-- One chip, in the order that matters: a pull in flight beats a dead DJ beats "we agree". -->
      {#if jukeFetching}
        <span class="juke-chip info" title="Pulling the track off the share">FETCHING</span>
      {:else if jukeStale}
        <span class="juke-chip warn" title="The DJ went quiet: the deck is frozen until someone presses">DECK STALE</span>
      {:else if jukeNow}
        <span class="juke-chip ok" title="You are where the DJ says the room is">SYNCED</span>
      {/if}
      <span class="stage-spacer"></span>
      {#if jukeNow}
        <span class="stage-label juke-dj" title="Whoever pressed last owns the deck">dj {jukeIsDj() ? "you" : nameOf(jukeNow.dj)}</span>
      {/if}
      <span class="juke-vol-ico">{@render icoSpeaker()}</span>
      <input
        class="stage-vol juke-vol"
        type="range"
        min="0"
        max="1"
        step="0.05"
        value={jukeVol}
        aria-label="Jukebox volume for you"
        title="Jukebox volume (yours only)"
        oninput={(e) => setJukeVol(Number(e.currentTarget.value))}
      />
    </div>

    {#if jukeNow}
      {@const cur = jukeQueue.find((e) => e.id === jukeNow?.entry)}
      <div class="juke-now">
        <div class="juke-now-top">
          <span class="juke-now-nm" title={jukeNow.name}>{jukeNow.name}</span>
          <span class="juke-time">{jukeElapsed(jukePaint)} / {jukeDur > 0 ? jukeClock(jukeDur) : "?:??"}</span>
        </div>
        <div class="juke-bar"><i class="juke-bar-fill" style={`width:${jukePct(jukePaint)}%`}></i></div>
        <div class="juke-transport">
          <!-- No previous: the transport has no rewind, and a fake one would desync the room. -->
          <button
            class="juke-play"
            title={jukeNow.paused || jukeStale ? "Play for the room" : "Pause the room"}
            aria-label={jukeNow.paused || jukeStale ? "Play" : "Pause"}
            onclick={jukeToggle}
          >{#if jukeNow.paused || jukeStale}{@render icoPlay()}{:else}{@render icoPause()}{/if}</button>
          <button class="ghost juke-tbtn" title="Skip: takes the deck and moves everyone on" aria-label="Skip" onclick={jukeSkip}>{@render icoSkip()}</button>
          <span class="stage-spacer"></span>
          {#if cur}
            <span class="stage-label juke-by" style={`color:${instColor(cur.author)}`} title={`Queued by ${nameOf(cur.author)}`}>added by {nameOf(cur.author)}</span>
          {/if}
        </div>
      </div>
    {:else}
      <div class="juke-idle">
        <span class="stage-label">deck idle</span>
        <span class="juke-idle-hint">queue something and press play</span>
      </div>
    {/if}

    <div class="juke-rule">
      <span class="stage-label">UP NEXT</span>
      <span class="juke-rule-line"></span>
      <button
        class="ghost juke-add"
        disabled={!jukeShareInView}
        title={jukeShareInView ? "Queue a track from this server's share" : "Open the server this room is on to add from its share"}
        onclick={() => (jukePickerOpen = true)}
      >＋ from files</button>
    </div>

    {#if jukeUpNext.length === 0}
      <p class="juke-empty">nothing queued</p>
    {:else}
      <ul class="juke-queue">
        {#each jukeUpNext as e (e.id)}
          {@const gone = jukeGone(e.cid)}
          {@const days = jukeExpiryDays(e.cid)}
          <li class="juke-row" class:gone>
            <span class="juke-dot" style={`background:${instColor(e.author)}`} title={`Queued by ${nameOf(e.author)}`}></span>
            <button class="juke-nm" title={gone ? `${e.name} is no longer in the share` : `Play ${e.name} for the room`} onclick={() => jukePlayEntry(e.id)}>{e.name}</button>
            {#if jukeFetching === e.cid}
              <span class="juke-chip info">FETCHING</span>
            {/if}
            {#if gone}
              <span class="juke-chip gone" title="Nobody is sharing this any more">GONE</span>
            {:else if days >= 0}
              <span class="juke-chip warn" title="This listing drops out of circulation soon">EXPIRES {days}D</span>
            {/if}
            <button class="ghost juke-x" title="Take it off the queue" aria-label={`Remove ${e.name} from the queue`} onclick={() => jukeRemoveTrack(e.id)}>✕</button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/snippet}

<!--
  The sidebar's contextual block: what the sidebar shows depends on the surface selected in the
  content column's surface strip. Shared by the server and DM sidebars: `dm` suppresses the blocks
  that only make sense on a server (channels, the status feed's blurb).
-->
<!-- The "+" insert panel: link/embed this server's own content. One snippet, two homes : the
  chat composer (above it) and the wiki editor's toolbar (below it); insertTarget routes the
  insertion to the right caret. -->
{#snippet insertPanel()}
  <div class="insert-picker">
    <div class="ip-tabs" role="tablist">
      <button type="button" role="tab" aria-selected={insertTab === "files"} class:active={insertTab === "files"} onclick={() => (insertTab = "files")}>Files</button>
      {#if !cur?.isDm}
        <button type="button" role="tab" aria-selected={insertTab === "status"} class:active={insertTab === "status"} onclick={() => (insertTab = "status")}>Status</button>
        <button type="button" role="tab" aria-selected={insertTab === "wiki"} class:active={insertTab === "wiki"} onclick={() => (insertTab = "wiki")}>Wiki</button>
        <button type="button" role="tab" aria-selected={insertTab === "events"} class:active={insertTab === "events"} onclick={() => (insertTab = "events")}>Events</button>
      {/if}
      <span class="ip-count">{insertCount}</span>
      <button type="button" class="ghost small ip-close" title="Close (Esc)" onclick={closeInsert}>✕</button>
    </div>
    <input
      bind:this={insertInput}
      class="ip-search"
      bind:value={insertQuery}
      placeholder={insertTab === "files" ? "Find a file…" : insertTab === "status" ? "Find one of your posts…" : insertTab === "events" ? "Find an event…" : "Find a wiki page…"}
      onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); closeInsert(); (insertTarget === "wiki" ? wikiTextarea : composerEl)?.focus(); } }}
    />
    <div class="ip-list">
      {#if insertTab === "files"}
        {#each insertFiles as f}
          {@const media = !!safeMime(f.mime)}
          <div class="ip-row">
            <button
              type="button"
              class="ip-item"
              title={media ? "Embed this file inline" : "Insert a link to this file"}
              onclick={() => insertFileRef(f, media)}
            >
              <span class="ip-ico">{fileIcon(f.mime)}</span>
              <span class="ip-name">{f.name}</span>
              <span class="ip-meta">{f.path ? f.path + " · " : ""}{fmtSize(f.size)}</span>
            </button>
            {#if media}
              <button type="button" class="ip-alt" title="Insert a link instead of an inline embed" onclick={() => insertFileRef(f, false)}>link</button>
            {/if}
            <span class="ip-mode">{media ? "embed" : "link"}</span>
          </div>
        {:else}
          <p class="ip-empty muted">{insertLoading ? "Loading…" : insertQuery.trim() ? "No files match that." : "No files shared on this server yet."}</p>
        {/each}
      {:else if insertTab === "status"}
        {#each insertStatuses as s}
          <div class="ip-row">
            <button type="button" class="ip-item" title="Insert a link to this post" onclick={() => insertStatusRef(s)}>
              <span class="ip-ico">◈</span>
              <span class="ip-name">{msgSnippet(s.text, 70) || "(empty post)"}</span>
              <span class="ip-meta">{fmtTime(s.ts)}</span>
            </button>
            <span class="ip-mode">link</span>
          </div>
        {:else}
          <p class="ip-empty muted">{insertLoading ? "Loading…" : insertQuery.trim() ? "None of your posts match that." : "You haven't posted a status on this server yet."}</p>
        {/each}
      {:else if insertTab === "events"}
        {#each insertEvents as ev (ev.id)}
          <div class="ip-row">
            <button type="button" class="ip-item" title="Insert a link to this event" onclick={() => insertEventRef(ev)}>
              <span class="ip-ico">⧗</span>
              <span class="ip-name">{ev.title}</span>
              <span class="ip-meta">{fmtEventWhen(ev)}</span>
            </button>
            <span class="ip-mode">link</span>
          </div>
        {:else}
          <p class="ip-empty muted">{insertLoading ? "Loading…" : insertQuery.trim() ? "No events match that." : "No events on this server yet: add one on the ⧗ Events surface."}</p>
        {/each}
      {:else}
        {#each insertWikiPages as p}
          <div class="ip-row">
            <button type="button" class="ip-item" title="Insert a link to this page" onclick={() => insertWikiRef(p)}>
              <span class="ip-ico">📖</span>
              <span class="ip-name">{p}</span>
            </button>
            <span class="ip-mode">link</span>
          </div>
        {:else}
          <p class="ip-empty muted">{insertLoading ? "Loading…" : insertQuery.trim() ? "No pages match that." : "This server's wiki is empty."}</p>
        {/each}
      {/if}
    </div>
  </div>
{/snippet}

{#snippet contextNav(dm: boolean)}
  {#if view === "wiki"}
    {#if canModerate}
      <!-- Admin-only: the edit-review queue opener and the review-window setting. Members
           never see this section; their pending edits surface as a banner on the page. -->
      <h3 class="ctx-h"><span>Review</span></h3>
      <button
        class="wiki-review-open"
        class:pending={wikiPending.length > 0}
        class:active={wikiReviewOpen}
        title="Member edits waiting for approval; each auto-accepts at its deadline if nobody reviews it"
        onclick={() => (wikiReviewOpen = !wikiReviewOpen)}
      >⧗ Pending changes{#if wikiPending.length}<span class="wiki-review-count">{wikiPending.length}</span>{/if}</button>
      <label class="wiki-review-days">
        <span>review window</span>
        <select
          value={wikiReviewDays}
          title="How long a member's edit waits for review before it auto-accepts; off publishes edits immediately"
          onchange={(e) => setWikiReviewWindow(Number(e.currentTarget.value))}
        >
          {#each Array.from({ length: 31 }, (_, d) => d) as d}
            <option value={d}>{d === 0 ? "off" : `${d} day${d === 1 ? "" : "s"}`}</option>
          {/each}
        </select>
      </label>
    {/if}
    <h3 class="ctx-h">
      <span>Pages</span>
      <button class="wiki-help-btn" title="Formatting help" onclick={() => (showWikiHelp = true)}>?</button>
    </h3>
    {#if wikiPages.length > 6}
      <input class="list-search" bind:value={wikiFilter} placeholder="Search pages…" />
    {/if}
    {#if wikiFilter.trim()}
      <!-- Searching: a flat match list beats a tree you'd have to unfold. -->
      <ul class="channel-list wiki-pages">
        {#each filteredWikiPages as p}
          <li>
            <button
              class:active={p === activeWikiPage}
              onclick={() => openWikiPage(p)}
              use:contextMenu={() => wikiPageMenu(p)}
            >{p}{#if wikiMeta[p] === "wiki"}<span class="pg-fmt" title="Wikitext page">wt</span>{/if}</button>
          </li>
        {:else}
          <li class="muted small">No matching pages.</li>
        {/each}
      </ul>
    {:else}
      <!-- The page tree: names with `/` nest under collapsible folders ("Guides/Setup"). -->
      <ul class="channel-list wiki-pages wiki-tree">
        {#each wikiTreeRows as row (row.node.path)}
          <li class="wiki-tree-row" style:--tree-depth={row.depth}>
            {#if row.node.children.length > 0}
              <button
                class="wiki-tree-toggle"
                title={wikiCollapsed.has(row.node.path) ? "Expand" : "Collapse"}
                aria-expanded={!wikiCollapsed.has(row.node.path)}
                onclick={() => toggleWikiFolder(row.node.path)}
              >{wikiCollapsed.has(row.node.path) ? "▸" : "▾"}</button>
            {:else}
              <span class="wiki-tree-toggle leaf"></span>
            {/if}
            {#if row.node.page !== null}
              {@const p = row.node.page}
              <button
                class="wiki-tree-page"
                class:active={p === activeWikiPage}
                onclick={() => openWikiPage(p)}
                use:contextMenu={() => wikiPageMenu(p)}
              >{row.node.label}{#if wikiMeta[p] === "wiki"}<span class="pg-fmt" title="Wikitext page">wt</span>{/if}</button>
            {:else}
              <button
                class="wiki-tree-page wiki-tree-folder"
                title="A folder of pages: no page named exactly this"
                onclick={() => toggleWikiFolder(row.node.path)}
              >{row.node.label}</button>
            {/if}
          </li>
        {:else}
          <li class="muted small">No pages yet.</li>
        {/each}
      </ul>
    {/if}
    <form class="new-page" onsubmit={(e) => { e.preventDefault(); createWikiPage(); }}>
      <input bind:value={newWikiPage} placeholder="+ new page (use / to nest)" />
    </form>
  {:else if view === "files"}
    <h3><span>Folders</span></h3>
    <ul class="channel-list folder-nav">
      <li>
        <button class:active={folder === ""} onclick={() => (folder = "")}>⌂ Home</button>
      </li>
      {#each folderView.subs as sub}
        <li><button onclick={() => enterFolder(sub)}>▸ {sub}</button></li>
      {/each}
    </ul>
    <label class="upload-btn">
      {uploading ? "Uploading…" : "＋ Share a file here"}
      <input type="file" disabled={uploading} onchange={(e) => { uploadFile(e.currentTarget.files); e.currentTarget.value = ''; }} />
    </label>
    <form class="new-folder" onsubmit={(e) => { e.preventDefault(); const n = newFolder.trim(); if (n) { enterFolder(n); newFolder = ''; } }}>
      <input bind:value={newFolder} placeholder="＋ new folder…" />
    </form>
  {:else if view === "downloads"}
    <h3><span>Transfers</span></h3>
    <button class="ghost small ctx-action" onclick={clearFinishedDownloads}>Clear finished</button>
  {:else if view === "events"}
    <h3><span>Upcoming</span></h3>
    {#each upcomingEvents.slice(0, 5) as e (e.id)}
      <div class="ev-side">
        <span class="ev-side-when">{fmtEventWhen(e)}</span>
        <span class="ev-side-title">{e.title}</span>
      </div>
    {:else}
      <p class="muted small">Nothing scheduled: add an event on the right.</p>
    {/each}
  {:else if view === "status" && !dm}
    <h3><span>Status</span></h3>
    <p class="muted small">A slow feed for this server: one post at a time, no replies.</p>
  {:else if !dm}
    <h3><span>Channels</span> <span class="key">[ctrl+k]</span></h3>
    <ul class="channel-list">
      {#each cur?.channels ?? [] as c}
        <li class="channel-row">
          <button
            class="channel-name"
            class:active={c.id === cur?.active && view === "chat"}
            class:unread={cur?.unread.includes(c.id)}
            onclick={() => { switchTo(c.id); view = "chat"; }}
          >
            <span class="chan-hash">#</span>{c.name}
            {#if mentionChannels.has(c.id)}<span class="mention-badge" title="You were mentioned">@</span>{/if}
            {#if cur?.unread.includes(c.id)}<span class="dot">●</span>{/if}
          </button>
          {#if activeServerId !== null}
            {@const sv = activeServerId}
            {@const vn = roomMembers(sv, c.id).length}
            {#if vn}
              <button
                class="voice-pill btn-ico"
                class:in={inCall && callChannel === c.id}
                title={inCall && callChannel === c.id ? "You're in this voice room" : `Join voice (${vn} in)`}
                onclick={() => joinVoice(c.id, sv, c.name)}
              >{@render icoSpeaker()} {vn}</button>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    <form class="join-channel" onsubmit={(e) => { e.preventDefault(); addChannel(); }}>
      <input bind:value={newChannel} placeholder="+ join or create #channel" />
    </form>
  {/if}
{/snippet}

<!-- Your identity on this server, pinned to the sidebar's bottom edge → opens the profile editor. -->
{#snippet youPanel()}
  {#if myFp}
    <button type="button" class="you-panel" title="Edit your profile & name style on this server" onclick={() => switchView("profile")}>
      {@render avatarTag(myFp)}
      <span class="who">
        <span class="nm">{@render nameTag(myFp)}</span>
        <span class="st">online · e2e</span>
      </span>
      <span class="you-edit">☰</span>
    </button>
  {/if}
{/snippet}

<div
  class="titlebar"
  class:blurred={!windowFocused}
  class:maximized={winMaximized}
  class:caution={eclipseCaution && !locked}
  style={tbEdge ? `--tb-edge:${tbEdge}` : ""}
  data-tauri-drag-region
>
  <span class="tb-ears" aria-hidden="true">{@html earsSvg}</span>
  <span class="tb-brand" data-tauri-drag-region>Mewtual</span>
  <button
    type="button"
    class="tb-cat"
    class:wants={mentionChannels.size > 0 && !locked}
    data-pose={catPose}
    title={catWhy}
    aria-label={`Mascot: ${catWhy}`}
    onclick={petCat}
  >{@html catArt}</button>
  {#if !locked}
    <div class="tb-nav">
      <button
        type="button"
        class="tb-btn nav"
        disabled={!canGoBack}
        aria-label="Back"
        title="Back (Alt+Left, or the back button on your mouse)"
        onclick={navBack}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M6.5 1.5L3 5l3.5 3.5" /></svg>
      </button>
      <button
        type="button"
        class="tb-btn nav"
        disabled={!canGoFwd}
        aria-label="Forward"
        title="Forward (Alt+Right, or the forward button on your mouse)"
        onclick={navForward}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M3.5 1.5L7 5l-3.5 3.5" /></svg>
      </button>
    </div>
  {/if}
  <!-- One thing at a time, by priority: a shut vault shows nothing here at all, since this strip
       is in every screenshot of the lock screen. -->
  <div class="tb-slot" data-tauri-drag-region>
    {#if locked}
      <!-- Deliberately empty: the wordmark is the whole bar while the vault is shut. -->
    {:else if inCall}
      <span class="tb-call" data-tauri-drag-region title="Your mic, live in this room">
        <span class="tb-mic" aria-hidden="true">
          {#each [1, 2, 3, 4] as n}
            <i class="tb-mic-bar" class:on={tbMicBars >= n} style="--n: {n}"></i>
          {/each}
        </span>
        <span class="tb-call-name" data-tauri-drag-region>{callChannelName}</span>
      </span>
    {:else if tbHead}
      {@const head = tbHead}
      <span class="tb-lane" data-tauri-drag-region>
        {#key head.id}
          <button
            type="button"
            class="tb-tick tb-k-{head.kind}"
            style="--crawl: {tbCrawlDur(head.text)}s"
            title={head.text}
            onclick={head.go}
            onanimationend={() => tbAdvance(head.id)}
          >
            <span class="tb-tick-glyph" aria-hidden="true"></span>
            <span class="tb-tick-text">{head.text}</span>
          </button>
        {/key}
      </span>
    {:else}
      <span class="tb-ident" data-tauri-drag-region>mewtual@{tbPreset} · {APP_VERSION}</span>
    {/if}
  </div>
  <span class="tb-drag" data-tauri-drag-region></span>
  <div class="tb-controls">
    <button type="button" class="tb-btn" aria-label="Minimise" title="Minimise" onclick={() => appWindow.minimize()}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 5.5h9" /></svg>
    </button>
    <button
      type="button"
      class="tb-btn"
      aria-label={winMaximized ? "Restore" : "Maximise"}
      title={winMaximized ? "Restore" : "Maximise"}
      onclick={() => appWindow.toggleMaximize()}
    >
      {#if winMaximized}
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 3.5h6v6h-6z" /><path d="M3 3V0.5h6.5V7H7" /></svg>
      {:else}
        <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 0.5h9v9h-9z" /></svg>
      {/if}
    </button>
    <button type="button" class="tb-btn close" aria-label="Close" title="Close" onclick={() => appWindow.close()}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9" /></svg>
    </button>
  </div>
</div>

<main>
  {#if eclipseCaution && activeServerId !== null && !locked}
    <div class="eclipse-banner" role="status">
      ⚠ You may be isolated from this server: few members are reachable. Verify a member out of band.
    </div>
  {/if}
  {#if locked}
    <div class="start gate" class:setup={inSetup}>
      {#if vaultExists === null}
        <!-- One frame of nothing beats guessing: showing "unlock" to a new user, or "set up" to
             someone who already has a vault, is worse than a moment of quiet. -->
        {@render brandMark("")}
        <p class="muted small">Checking this device…</p>
      {:else if inSetup && setupStep === "welcome"}
        {@render brandMark("first run")}
        <p class="muted">
          Mewtual is peer to peer: there is no account and no company server holding your
          messages. Your identity lives in a vault on this machine, sealed with a secret only
          you know.
        </p>
        <div class="setup-choices">
          <button type="button" class="setup-card" onclick={() => { setupPath = "new"; setupStep = "secret"; }}>
            <span class="sc-ico">{@render icoLock()}</span>
            <span class="sc-title">Set up this device</span>
            <span class="sc-body">
              Start fresh here. You'll choose how to unlock the vault, then found a server or
              join one with an invite.
            </span>
          </button>
          <button type="button" class="setup-card" onclick={() => { setupPath = "sync"; setupStep = "secret"; }}>
            <span class="sc-ico">{@render icoDm()}</span>
            <span class="sc-title">I already use Mewtual</span>
            <span class="sc-body">
              Link this device to one you already have. It still needs its own vault secret
              first; then your other device shows a code and approves this one in person.
            </span>
          </button>
        </div>
        <p class="muted small">
          Nothing is written to disk until you finish. There is no password reset and no
          recovery email: the vault <em>is</em> the identity, so back it up once it exists.
        </p>
      {:else if inSetup && setupStep === "look"}
        {@render brandMark("step 3 of 3 · make it yours")}
        <p class="muted">
          Pick a look. Colours keep their jobs in every preset: green is presence, gold is
          mentions, red is danger. All of this is in Settings later, and servers can publish
          their own livery you can follow or ignore.
        </p>
        <div class="preset-row">
          {#each PRESETS as p (p.id)}
            <button
              type="button"
              class="preset-btn"
              class:active={appearance.preset === p.id}
              onclick={() => (appearance = { ...appearance, preset: p.id, accent: "" })}
            >
              <span class="preset-sw" style={`background:${p.sw}`}></span>{p.name}
            </button>
          {/each}
        </div>
        <div class="field">
          <span class="muted small">Accent: keep the preset's mood, swap the highlight colour</span>
          <div class="accent-row">
            {#each ACCENT_CHOICES as a (a)}
              <button
                type="button"
                class="accent-sw"
                class:active={appearance.accent === a}
                style={`background:${a}`}
                aria-label={`Accent colour ${a}`}
                title={a}
                onclick={() => (appearance = { ...appearance, accent: appearance.accent === a ? "" : a })}
              ></button>
            {/each}
            <input
              type="color"
              class="accent-custom"
              title="Custom accent colour"
              aria-label="Custom accent colour"
              value={appearance.accent || "#977df2"}
              oninput={(e) => (appearance = { ...appearance, accent: e.currentTarget.value })}
            />
          </div>
        </div>
        <label class="toggle">
          <input
            type="checkbox"
            checked={appearance.density === "compact"}
            onchange={() => (appearance = { ...appearance, density: appearance.density === "compact" ? "" : "compact" })}
          />
          <span>Compact density: tighter rows, more on screen</span>
        </label>
        <label class="toggle">
          <input
            type="checkbox"
            checked={appearance.chrome !== "clean"}
            onchange={() => (appearance = { ...appearance, chrome: appearance.chrome === "clean" ? "terminal" : "clean" })}
          />
          <span>Terminal chrome: scanlines &amp; glow on the frame</span>
        </label>
        <label class="toggle">
          <input
            type="checkbox"
            checked={appearance.motion !== "off"}
            onchange={() => (appearance = { ...appearance, motion: appearance.motion === "off" ? "" : "off" })}
          />
          <span>Hover motion: icons lift and turn under the pointer</span>
        </label>
        {#if error}<p class="error">{error}</p>{/if}
        <div class="setup-actions">
          <button class="ghost" onclick={setupRestart} disabled={unlocking}>← change my secret</button>
          <button onclick={setupFinish} disabled={unlocking || !setupFirst}>
            {unlocking ? "Creating your vault…" : "Create my vault"}
          </button>
        </div>
      {:else}
      {#if inSetup}
        {@render brandMark(setupStep === "confirm" ? "step 2 of 3 · confirm it" : "step 1 of 3 · choose your secret")}
        {#if setupStep === "confirm"}
          <p class="muted">
            Now do it again. Nothing has been written yet, and this is the only check you get:
            once the vault exists, a secret you can't reproduce is an identity you've lost.
          </p>
          {#if setupMismatch}
            <p class="error">That didn't match the first one. Cleared: try again, or go back and pick a different secret.</p>
          {/if}
        {:else}
          <p class="muted">
            Three ways in: a passphrase, a sigil you draw, or a tune you play. All three seal
            the same vault, so pick the one you'll actually reproduce months from now. A
            passphrase is the strongest.
          </p>
        {/if}
      {:else}
        {@render brandMark("")}
        <p class="muted">
          Unlock your servers: with a passphrase, a sigil, or a tune. All three seal the
          same vault; pick the one you'll actually remember.
        </p>
      {/if}
      <!-- Locked while confirming: the two performances are compared as encoded strings, so
           switching method between them could only ever mismatch. -->
      {@const tabsLocked = inSetup && setupStep === "confirm"}
      <div class="ul-tabs" role="tablist">
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "pass"} aria-selected={unlockMethod === "pass"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "pass"; }}>
          Passphrase <span class="ul-rec">recommended</span>
        </button>
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "sigil"} aria-selected={unlockMethod === "sigil"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "sigil"; }}>Sigil</button>
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "melody"} aria-selected={unlockMethod === "melody"} onclick={() => { unlockMethod = "melody"; initMidi(); }}>Melody</button>
      </div>
      {#if unlockMethod === "pass"}
        <label class="field">
          <span class="muted">Passphrase</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="password"
            bind:value={passphrase}
            onkeydown={(e) => e.key === "Enter" && passphrase && gateSubmit()}
            placeholder="passphrase"
            autofocus
          />
        </label>
      {:else if unlockMethod === "sigil"}
        <p class="muted small">
          Inscribe your sigil: drag node to node over the circle (order and direction count;
          lift and press again for a second stroke), tap a node to cycle its mark, pick one or
          more focus emoji, and whisper a magic word. Keyboard: Tab to a node, Enter adds it to
          the stroke, <span class="fp">c</span> cycles its mark. Path, emoji and word together
          are the secret (marks are optional), so a short sigil of 6–10 hops is plenty.
        </p>
        <div class="sigil-wrap" class:complete={sigilComplete}>
          <svg
            bind:this={sigilSvgEl}
            class="sigil-svg"
            viewBox={`0 0 ${SIGIL_VIEW} ${SIGIL_VIEW}`}
            role="application"
            aria-label="Sigil circle: 19 nodes. Tab between nodes; Enter adds the focused node to the current stroke; C cycles its colour mark."
            onpointerdown={sigilPointerDown}
            onpointermove={sigilPointerMove}
            onpointerup={sigilPointerUp}
            onpointercancel={sigilPointerUp}
          >
            <defs>
              <marker id="sigil-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path class="sigil-arrow" d="M 0 0 L 8 4 L 0 8 z" />
              </marker>
              <path id="sigil-ringpath" d={ringPathD(R_TEXT)} />
            </defs>
            <!-- Decorative ring guides run through the node rings; nodes draw over them. -->
            <circle class="sigil-ring" cx={SIGIL_C} cy={SIGIL_C} r={R_INNER} />
            <circle class="sigil-ring" cx={SIGIL_C} cy={SIGIL_C} r={R_OUTER} />
            <!-- Ring inscription. An EMPTY word shows nothing; from the FIRST character the
                 ring is a CONSTANT-count rune band derived from (session seed, word),
                 stretched around the full circumference; the SAME count for 1 character or
                 30. Do NOT "fix" this into a length-proportional ring: per-character runes
                 leak the word's length and a repeated sequence leaks it via its period. The
                 empty/non-empty step leaks exactly one bit, which the disabled Unlock button
                 already gives away (an empty word can't form a valid secret). It reshuffles
                 as you type; the keyed tspans remount only where a rune actually changed,
                 which is the "being inscribed" flicker; but recovers to nothing without the
                 session seed (reveal toggle aside). -->
            {#if sigilWordLen}
              <text class="sigil-ring-text">
                {#if sigilShowWord}
                  <textPath href="#sigil-ringpath" startOffset="1%">{normalizeWord(sigilWord)}</textPath>
                {:else}
                  <textPath href="#sigil-ringpath" textLength={Math.round(2 * Math.PI * R_TEXT) - 8} lengthAdjust="spacing">
                    {#each ringGlyphs(sigilWord, sigilSeed) as g, i (`${i}:${g}`)}<tspan class="sigil-glyph">{g}</tspan>{/each}
                  </textPath>
                {/if}
              </text>
            {/if}
            <!-- The chosen emoji repeat as the points around the circle, alternating through
                 the set (display order is the pick order; only the encoder sorts). -->
            {#if sigilEmojis.length}
              {#each ringPoints(12, R_EMOJI, 15) as p, i (i)}
                <text class="sigil-ring-emoji" x={p.x} y={p.y}>{sigilEmojis[i % sigilEmojis.length]}</text>
              {/each}
            {/if}
            {#each sigilStrokes as st, i (i)}
              <polyline
                class="sigil-path"
                points={st.map((n) => `${LATTICE[n].x},${LATTICE[n].y}`).join(" ")}
                marker-end="url(#sigil-arrow)"
              />
            {/each}
            {#if sigilDrawing.length > 1}
              <polyline class="sigil-path live" points={sigilDrawing.map((n) => `${LATTICE[n].x},${LATTICE[n].y}`).join(" ")} />
            {/if}
            {#each LATTICE as n, i (i)}
              <circle
                class="sigil-node"
                class:lit={sigilDrawing.includes(i) || sigilStrokes.some((s) => s.includes(i))}
                cx={n.x}
                cy={n.y}
                r={NODE_R}
                role="button"
                tabindex="0"
                aria-label={`${nodeLabel(i)}, mark: ${COLOR_NAMES[sigilColors[i]]}. Enter adds to stroke, C cycles the mark.`}
                onkeydown={(e) => sigilNodeKey(e, i)}
              />
              <!-- Colour marks: every variant is a different SHAPE as well as hue (dot / ring /
                   diamond), so the 4-way signal survives colour-blindness and any palette. -->
              {#if sigilColors[i] === 1}
                <circle class="sigil-mark m1" cx={n.x} cy={n.y} r="3.4" />
              {:else if sigilColors[i] === 2}
                <circle class="sigil-mark m2" cx={n.x} cy={n.y} r="4.6" />
              {:else if sigilColors[i] === 3}
                <rect class="sigil-mark m3" x={n.x - 3.4} y={n.y - 3.4} width="6.8" height="6.8" transform={`rotate(45 ${n.x} ${n.y})`} />
              {/if}
            {/each}
          </svg>
          <canvas bind:this={sigilFx} class="sigil-fx" width="576" height="576" aria-hidden="true"></canvas>
          {#if sigilSummon}<div class="sigil-summon" aria-hidden="true">🐈‍⬛</div>{/if}
        </div>
        <div class="ul-seq">
          {#if sigilDrawing.length}
            <span class="muted small mono">stroke: {sigilDrawing.join("-")}</span>
            <button type="button" class="ghost small" title="Finish the current stroke (a drag finishes when you lift)" onclick={sigilCommitStroke} disabled={sigilDrawing.length < 2}>end stroke</button>
            <button type="button" class="ghost small" onclick={() => (sigilDrawing = [])}>drop</button>
          {:else if sigilStrokes.length || sigilMarked}
            <span class="muted small mono">
              {sigilStrokes.length ? `${encodeSigilPath(sigilStrokes)} · ${segmentCount(sigilStrokes)} hop${segmentCount(sigilStrokes) === 1 ? "" : "s"}` : "no path yet"}{sigilMarked ? ` · ${sigilMarked} marked` : ""}
            </span>
            {#if sigilStrokes.length}
              <button type="button" class="ghost small" title="Remove the last stroke" onclick={sigilUndo}>⌫</button>
            {/if}
            <button type="button" class="ghost small" title="Clear the path and all marks" onclick={() => { sigilStrokes = []; sigilColors = Array(19).fill(0); }}>Clear</button>
          {:else}
            <span class="muted small">No sigil yet: drag from one node to another (tap to mark).</span>
          {/if}
        </div>
        <details class="sigil-emoji-pick">
          <summary>{sigilEmojis.length ? `focus emoji: ${sigilEmojis.join(" ")}` : "choose focus emoji"} <span class="muted">({sigilEmojis.length}/{MAX_SIGIL_EMOJI}; click again to remove)</span></summary>
          <div class="sigil-emoji-grid">
            {#each EMOJI_SETS as set (set.label)}
              {#each set.list as em (em)}
                <button
                  type="button"
                  class="sigil-emoji"
                  class:sel={sigilEmojis.includes(em)}
                  title={set.label}
                  disabled={!sigilEmojis.includes(em) && sigilEmojis.length >= MAX_SIGIL_EMOJI}
                  onclick={() => toggleSigilEmoji(em)}
                >{em}</button>
              {/each}
            {/each}
          </div>
        </details>
        <label class="field">
          <span class="muted">Magic word</span>
          <input
            type="password"
            bind:value={sigilWord}
            onkeydown={(e) => e.key === "Enter" && sigilSecret && gateSubmit()}
            placeholder="magic word"
            autocomplete="off"
          />
        </label>
        <label class="sigil-show muted small">
          <input type="checkbox" bind:checked={sigilShowWord} />
          inscribe my actual word in the ring (visible to anyone watching)
        </label>
        {#if sigilComplete}
          <div class="ul-meter {bitsTier(sigilBits)}">≈ {sigilBits} bits{sigilBits < 28 ? ": add hops or a longer word" : sigilBits < 44 ? ": okay; a little more is stronger" : ": strong"}</div>
        {:else}
          <span class="muted small mono">sigil {sigilStrokes.length ? "✓" : "·"} · emoji {sigilEmojis.length ? "✓" : "·"} · word {sigilWordLen ? "✓" : "·"}</span>
        {/if}
      {:else}
        <p class="muted small">
          Play your unlock tune: octaves count (C6 is not C4), notes played together are one
          chord, and how long you hold sets the note value. On-screen keys, the
          <span class="fp">a w s e d f t g y h u j</span> row (<span class="fp">z</span>/<span class="fp">x</span> shift register,
          <span class="fp">1</span>–<span class="fp">7</span> jump to it), and a MIDI keyboard all feed the same staff.
          Avoid famous tunes; they're guessable.
        </p>
        <div class="piano-head">
          <button type="button" class="ghost small" title="Register down (z)" onclick={() => setOctave(melodyOctave - 1)}>−</button>
          <span class="piano-oct">C{melodyOctave}–C{melodyOctave + 2}</span>
          <button type="button" class="ghost small" title="Register up (x)" onclick={() => setOctave(melodyOctave + 1)}>＋</button>
          <button
            type="button"
            class="ghost small rhythm-toggle"
            class:on={melodyRhythm}
            title="Count how long each note is held as part of the secret. Off = pitches only."
            onclick={toggleRhythm}
          >{melodyRhythm ? "♩ rhythm on" : "rhythm off"}</button>
        </div>
        <div class="piano">
          {#each Array.from({ length: 25 }, (_, i) => (melodyOctave + 1) * 12 + i) as note (note)}
            {@const pc = note % 12}
            <button
              type="button"
              class="piano-key"
              class:sharp={PC_SHARP[pc]}
              class:held={heldNotes.includes(note)}
              title={noteName(note)}
              onpointerdown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); noteOn(note); }}
              onpointerup={() => noteOff(note)}
              onpointercancel={() => noteOff(note)}
            >{pc === 0 ? noteName(note) : NOTE_NAMES[pc]}</button>
          {/each}
        </div>
        <div class="sheet-wrap" class:empty={!melodySeq.length} bind:clientWidth={sheetW}>
          <svg
            class="sheet"
            width={sheet.w}
            height={sheet.h}
            viewBox={`0 ${sheet.minY} ${sheet.w} ${sheet.h}`}
            role="img"
            aria-label={melodySeq.length ? `Score: ${sheetText}` : "Empty score"}
          >
            <!-- Grand staff: treble above, bass below, joined at both ends. -->
            {#each [...TREBLE_LINES, ...BASS_LINES] as s (s)}
              <line class="sheet-line" x1="6" y1={yOf(s)} x2={sheet.w - 14} y2={yOf(s)} />
            {/each}
            <line class="sheet-line bar" x1="6.5" y1={STAFF_TOP} x2="6.5" y2={STAFF_BOT} />
            <line class="sheet-line bar" x1={sheet.w - 14.5} y1={STAFF_TOP} x2={sheet.w - 14.5} y2={STAFF_BOT} />
            <text class="sheet-clef" x="14" y="58">𝄞</text>
            <text class="sheet-clef small" x="14" y="80">𝄢</text>
            {#each sheet.events as ev, i (i)}
              <g class="sheet-ev" class:sounding={i === playIdx}>
                {#each ev.ledgers as l, j (j)}
                  <line class="sheet-line" x1={l.x - 9} y1={l.y} x2={l.x + 9} y2={l.y} />
                {/each}
                {#if ev.stem}
                  <line class="sheet-stem" x1={ev.stem.x} y1={ev.stem.y1} x2={ev.stem.x} y2={ev.stem.y2} />
                  {#if ev.flag}
                    <!-- Eighth-note flag, curling back from the free end of the stem. -->
                    <path
                      class="sheet-flag"
                      d={`M ${ev.stem.x} ${ev.stem.y2} q 9 5 8 13 q 3 -9 -8 -18 z`}
                      transform={ev.stem.y2 < ev.stem.y1 ? "" : `rotate(180 ${ev.stem.x} ${ev.stem.y2})`}
                    />
                  {/if}
                {/if}
                {#each ev.heads as h, j (j)}
                  {#if h.sharp}<text class="sheet-acc" x={h.x - 9} y={h.y + 3.4}>♯</text>{/if}
                  <ellipse class="sheet-head" class:hollow={!ev.filled} cx={h.x} cy={h.y} rx={HEAD_RX} ry={HEAD_RY} transform={`rotate(-18 ${h.x} ${h.y})`} />
                {/each}
                {#if ev.label}<text class="sheet-chord" x={ev.x} y={sheet.labelY}>{ev.label}</text>{/if}
              </g>
            {/each}
          </svg>
        </div>
        <div class="ul-seq">
          {#if heldNotes.length || chordBuf.length}
            <span class="sheet-live mono">
              ▮ {chordBuf.map((n) => noteName(n)).join(" ")}
              {#if chordBuf.length > 1}<em>{chordName([...chordBuf].sort((a, b) => a - b))}</em>{/if}
              {#if melodyRhythm}· {DUR_NAMES[durClass(holdMs)]}{/if}
            </span>
          {:else if melodySeq.length}
            <button
              type="button"
              class="ghost small"
              class:playing
              title={playing ? "Stop" : "Play the sequence back"}
              onclick={playMelody}
            >{playing ? "■ stop" : "▶ play"}</button>
            <span class="muted small mono">{melodySeq.length} event{melodySeq.length === 1 ? "" : "s"}</span>
            <button type="button" class="ghost small" title="Remove the last note (Backspace)" onclick={() => { stopPlayback(); melodySeq = melodySeq.slice(0, -1); }}>⌫</button>
            <button type="button" class="ghost small" onclick={() => { stopPlayback(); melodySeq = []; }}>Clear</button>
          {:else}
            <span class="muted small">No notes yet: hold a key to write one.</span>
          {/if}
        </div>
        {#if melodySeq.length}
          <div class="ul-meter {bitsTier(melodyBits)}">≈ {melodyBits} bits{melodyBits < 28 ? ": too short, keep playing" : melodyBits < 44 ? ": okay; longer is stronger" : ": strong"}</div>
        {/if}
        {#if melodyRhythm}
          <p class="muted small">
            Hold length becomes the note: under {DUR_MAX_MS[0]}ms an eighth, under {DUR_MAX_MS[1]}ms a
            quarter, under {DUR_MAX_MS[2]}ms a half, longer a whole. Rhythm is part of the secret:
            turn it off above if you'd rather the tune be pitches only.
          </p>
        {/if}
        {#if midiName}<div class="ul-midi">⌁ MIDI: {midiName}</div>{/if}
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
      {#if inSetup}
        <div class="setup-actions">
          <button class="ghost" onclick={() => (setupStep === "confirm" ? setupRestart() : (setupStep = "welcome"))}>← back</button>
          <button onclick={gateSubmit} disabled={!unlockSecret()}>
            {setupStep === "confirm" ? "Confirm" : "Continue"}
          </button>
        </div>
      {:else}
        <button onclick={() => unlock()} disabled={unlocking || !unlockSecret()}>
          {unlocking ? "Unlocking…" : "Unlock"}
        </button>
      {/if}
      {/if}
    </div>
  {:else if servers.length === 0 || showAdd}
    <div class="start">
      {@render brandMark(servers.length ? "" : "your vault is ready")}
      {#if showAdd && servers.length}
        <button class="ghost" onclick={() => (showAdd = false)}>← back</button>
      {/if}
      {#if syncIntent}
        <p class="muted">
          Vault created. Now hand the pairing code below to the device you already use: it
          shows the same code, you compare them, and it approves this one.
        </p>
      {/if}
      <label class="field">
        <span class="muted">Display name</span>
        <input bind:value={displayName} placeholder="display name" />
      </label>
      <details>
        <summary>Network (optional)</summary>
        <label class="field">
          <span class="muted">
            Reachable address so others can join over a network: your LAN IP (e.g.
            192.168.1.5), or a public IP / host:port if port-forwarded. Leave blank for
            same-machine only.
          </span>
          <input bind:value={advertise} placeholder="LAN/public IP (optional)" />
        </label>
        <label class="field">
          <span class="muted">
            Relay address (optional): paste a relay node's multiaddr to be reachable over
            the internet with no port-forward (zero-config NAT traversal).
          </span>
          <input bind:value={relay} placeholder="/ip4/…/tcp/…/p2p/… (optional)" />
        </label>
        <label class="field">
          <span class="muted">
            Rendezvous address (optional): paste a rendezvous node's multiaddr to register there,
            so people can join with <em>just the invite</em> (no address needed). Saved as your
            default.
          </span>
          <input bind:value={rendezvous} placeholder="/ip4/…/tcp/…/p2p/… (optional)" />
        </label>
      </details>
      <button onclick={found} disabled={busy}>
        {busy ? "Working…" : "Found a server"}
      </button>
      <hr />
      <p class="muted">…or join an existing server with an invite:</p>
      <textarea class="invite-code" bind:value={joinInvite} rows="3" placeholder="paste invite here"></textarea>
      <div class="pc-actions">
        <button onclick={join} disabled={busy || !joinInvite.trim()}>Join</button>
        <button class="ghost" disabled={scanOpen} onclick={() => scanQr((t) => { if (t) joinInvite = t; })}>⛶ Scan invite QR</button>
      </div>
      <details open={syncIntent}>
        <summary>Link this device to another device you own</summary>
        <p class="muted small">
          Your other device stays the master: it will show a code and ask permission before
          this device gets anything. Server admission for linked devices arrives with the
          next protocol slice: the grant is stored until then.
        </p>
        {#if !pairBlob}
          <button class="ghost" onclick={pairBegin}>Generate pairing code</button>
        {:else}
          {#if pairDeviceId}
            <p class="muted small">This device's code: <span class="fp">{pairDeviceId.slice(0, 8)}</span>: the master shows the same one.</p>
          {/if}
          <textarea class="invite-code" rows="3" readonly value={pairBlob}></textarea>
          <canvas class="qr-canvas" use:qr={pairBlob}></canvas>
          <div class="pc-actions">
            <button class="ghost small" onclick={() => copyText(pairBlob)}>Copy pairing code</button>
            <button class="ghost small" disabled={soundBusy !== ""} onclick={() => sendBySound(pairBlob)}>{soundBusy === "send" ? "Playing…" : "🔊 Send by sound"}</button>
          </div>
          <label class="field">
            <span class="muted small">Then paste the grant from the master device:</span>
            <textarea class="invite-code" rows="3" bind:value={pairBundle} placeholder="paste grant bundle…"></textarea>
          </label>
          <button class="ghost small" disabled={scanOpen} onclick={() => scanQr((t) => { if (t) pairBundle = t; })}>⛶ Scan grant QR</button>
          <input type="password" bind:value={pairPass} placeholder="transport passphrase (set on the master)" />
          <button class="ghost small" disabled={!pairBundle.trim() || !pairPass} onclick={pairOpen}>Open grant</button>
          {#if pairSummary}
            <p class="muted small">{pairSummary}</p>
            <button disabled={pairJoining} onclick={pairJoinAll}>
              {pairJoining ? "Joining…" : "Join granted servers now"}
            </button>
            {#if pairJoinResults.length}
              <ul class="pair-results">
                {#each pairJoinResults as r (r.name)}
                  <li class:ok={r.ok} class:err={!r.ok}>
                    {r.ok ? "✓" : "✕"} {r.name}{r.error ? `: ${r.error}` : ""}
                  </li>
                {/each}
              </ul>
              {#if pairJoinResults.some((r) => !r.ok)}
                <p class="muted small">
                  A failed server keeps its grant: the usual cause is its owner being offline
                  (your admission is queued and completes when they return). Retry any time.
                </p>
              {/if}
            {/if}
          {/if}
        {/if}
      </details>
      {#if error}<p class="muted" style="color:#ff6b6b">{error}</p>{/if}
    </div>
  {:else}
    <div class="app">
      <!-- The rail is three bands: DMs/inbox pinned at the top, the server strip scrolling in
           the middle (however many servers you join), and feedback/settings pinned at the
           bottom. Only the middle band scrolls, so the fixed destinations never slide away. -->
      <nav class="rail">
        <div class="rail-fixed">
          <button
            class="server-icon dm-circle"
            class:active={dmHome}
            title="Direct messages & friends"
            onclick={enterDmHome}
          >
            {@render icoDm()}
            {#if dmRequests.length}
              <span class="rail-badge">{dmRequests.length}</span>
            {:else if dmList.some((d) => d.unread.length || d.dot)}
              <span class="rail-dot">●</span>
            {/if}
          </button>
          <button
            class="server-icon inbox-circle"
            class:active={inboxView}
            title="Inbox: mentions & replies"
            onclick={openInbox}
          >
            {@render icoInbox()}
            {#if inboxUnseenCount}
              <span class="rail-badge">{inboxUnseenCount}</span>
            {/if}
          </button>
          <div class="rail-sep"></div>
        </div>
        <div class="rail-scroll">
          {#each railServers as s}
            <button
              class="server-icon"
              class:active={s.id === activeServerId && !dmHome && !inboxView}
              title={s.name}
              onclick={() => switchServer(s.id)}
              use:contextMenu={() => serverMenu(s)}
            >
              {#if serverIcons[s.id] && appearance.icons !== "flat"}
                <img class="rail-img" src={imgSrc(serverIcons[s.id])} alt="" />
              {:else}
                {monogram(s.name)}
              {/if}
              {#if s.unread.length}
                <span class="rail-badge">{s.unread.length}</span>
              {:else if s.dot}
                <span class="rail-dot">●</span>
              {/if}
            </button>
          {/each}
          <button class="server-icon add" title="Add a server" onclick={() => (showAdd = true)}>+</button>
        </div>
        <div class="rail-fixed rail-foot">
          <div class="rail-sep"></div>
          <button class="server-icon orbit-btn" class:active={spaceOpen} title="Server space (Ctrl+O)" aria-label="Open the 360 server space" onclick={toggleSpace}>{@render icoOrbit()}</button>
          <button class="server-icon feedback-btn" title="Send feedback (bug / feature request)" aria-label="Send feedback" onclick={() => (showFeedback = true)}>{@render icoFeedback()}</button>
          <button class="server-icon gear" title="Settings" aria-label="Settings" onclick={() => openSettings()}>{@render icoGear()}</button>
        </div>
      </nav>

      {#if inboxView}
        <section class="inbox-screen">
          <div class="inbox-head">
            <h2>Inbox</h2>
            <div class="inbox-mode">
              <button class:active={inboxMode === "mentions"} onclick={() => (inboxMode = "mentions")}>Mentions</button>
              <button class:active={inboxMode === "news"} onclick={() => { inboxMode = "news"; loadNews(); }}>News</button>
            </div>
            <span class="muted small">
              {inboxMode === "mentions" ? "Mentions & replies, across every server & DM" : "Status posts & upcoming events, across your servers"}
            </span>
            <button class="ghost small inbox-refresh" onclick={() => (inboxMode === "mentions" ? loadInbox() : loadNews())} disabled={inboxMode === "mentions" ? inboxLoading : newsLoading}>↻ Refresh</button>
          </div>
          {#if inboxMode === "news"}
            {#if newsLoading && !newsItems.length}
              <p class="muted inbox-empty">Loading…</p>
            {:else}
              {#if newsUpcoming.length}
                <h3 class="ev-h"><span>Upcoming events</span></h3>
                <ul class="inbox-list">
                  {#each newsUpcoming as n (n.server + ":" + n.kind + ":" + n.ts + n.text)}
                    <li class="inbox-item">
                      <button class="inbox-jump" onclick={() => jumpToNews(n)}>
                        <div class="inbox-meta">
                          <span class="inbox-tag event-tag">⧗ event</span>
                          <span class="inbox-where">{n.serverName}</span>
                          <span class="inbox-time" title={new Date(n.ts).toLocaleString()}>{dayLabel(n.ts)}</span>
                        </div>
                        <div class="inbox-body"><span class="inbox-text">{n.text}</span></div>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
              <h3 class="ev-h"><span>Recent status</span></h3>
              {#if !newsFeed.length}
                <p class="muted inbox-empty">No status posts yet: servers' Status surfaces feed this.</p>
              {:else}
                <ul class="inbox-list">
                  {#each newsFeed as n (n.server + ":" + n.ts + ":" + n.author)}
                    <li class="inbox-item">
                      <button class="inbox-jump" onclick={() => jumpToNews(n)}>
                        <div class="inbox-meta">
                          <span class="inbox-tag reply-tag">◇ status</span>
                          <span class="inbox-where">{n.serverName}</span>
                          <span class="inbox-time" title={new Date(n.ts).toLocaleString()}>{fmtTime(n.ts)}</span>
                        </div>
                        <div class="inbox-body"><span class="inbox-text">{@html renderMessage(n.text, "")}</span></div>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {/if}
          {:else if inboxLoading && !inboxItems.length}
            <p class="muted inbox-empty">Loading…</p>
          {:else if !inboxItems.length}
            <p class="muted inbox-empty">
              Nothing yet. When someone <strong>@-mentions</strong> you or <strong>replies</strong> to one of your
              messages, it shows up here: with who said it, where, and a jump straight to it.
            </p>
          {:else}
            <ul class="inbox-list">
              {#each inboxItems as it (it.server + ":" + it.channel + ":" + it.message_id)}
                <li class="inbox-item" class:unseen={inboxUnseen(it)}>
                  <button class="inbox-jump" onclick={() => jumpToInbox(it)}>
                    <div class="inbox-meta">
                      <span class="inbox-kind">
                        {#if it.mention}<span class="inbox-tag mention-tag">@ mention</span>{/if}
                        {#if it.reply}<span class="inbox-tag reply-tag">↰ reply</span>{/if}
                      </span>
                      <span class="inbox-where">{it.is_dm ? "Direct message" : it.server_name} · #{inboxChannelName(it)}</span>
                      <span class="inbox-time" title={new Date(it.ts).toLocaleString()}>{fmtTime(it.ts)}</span>
                    </div>
                    <div class="inbox-body">
                      <strong class="inbox-author">{it.author_name || it.author.slice(0, 8)}</strong>
                      <span class="inbox-text">{@html renderMessage(it.text, "")}</span>
                    </div>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        </section>
      {:else}
      <aside class="sidebar">
        {#if dmHome}
          <div class="server-head">
            <strong class="server-title">Direct messages</strong>
          </div>
          <div class="dm-actions">
            <button class="ghost small" onclick={openNewDm}>＋ New DM</button>
            <button class="ghost small" onclick={openAddFriend}>Add friend</button>
          </div>
          {#if notice}<p class="dm-notice muted small">{notice}</p>{/if}
          {#if dmRequests.length}
            <div class="dm-requests">
              <h3>Friend requests</h3>
              {#each dmRequests as req (req.server + ":" + req.from_fp)}
                <div class="dm-req">
                  <span class="dm-req-name">{req.from_name} wants to DM you</span>
                  <div class="dm-req-actions">
                    <button class="ghost small" disabled={busy} onclick={() => acceptDmRequest(req)}>Accept</button>
                    <button class="ghost small" onclick={() => declineDmRequest(req)}>Decline</button>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
          {#if showNewDm}
            <form class="dm-form" onsubmit={(e) => { e.preventDefault(); newDm(); }}>
              <input bind:value={dmName} placeholder="Friend's name…" />
              <button disabled={busy || !dmName.trim()}>Create &amp; get code</button>
              <span class="muted small">Creates a private 1:1 and gives you a friend code to share.</span>
            </form>
          {/if}
          {#if showAddFriend}
            <form class="dm-form" onsubmit={(e) => { e.preventDefault(); addFriend(); }}>
              <input bind:value={dmName} placeholder="Friend's name…" />
              <textarea class="invite-code" bind:value={dmInvite} rows="2" placeholder="Paste their friend code…"></textarea>
              <button disabled={busy || !dmName.trim() || !dmInvite.trim()}>Connect</button>
            </form>
          {/if}
          {#if dmList.length > 1}
            <label class="dm-sort">
              <span class="muted small">Sort</span>
              <select bind:value={dmSort}>
                <option value="recent">Recent</option>
                <option value="activity">Most active</option>
                <option value="reconnect">Reconnect</option>
                <option value="alpha">A–Z</option>
              </select>
            </label>
          {/if}
          <ul class="dm-list">
            {#each sortedDmList as d (d.id)}
              {@const st = dmStats[d.id]}
              <li>
                <button class:active={d.id === activeServerId} onclick={() => switchServer(d.id)}>
                  <span class="dm-ava">{d.name.slice(0, 1).toUpperCase()}</span>
                  <span class="dm-label">{d.name}</span>
                  {#if d.unread.length}
                    <span class="dot">●</span>
                  {:else if st?.last_ts}
                    <span class="dm-hint muted" title="Last message">{relTime(nowTick - st.last_ts)}</span>
                  {/if}
                </button>
              </li>
            {:else}
              <li class="muted">No DMs yet: start one or accept a friend code.</li>
            {/each}
          </ul>
          {#if cur?.isDm && cur.invite}
            <div class="dm-code">
              <span class="muted small">Friend code for {cur.name}: share it to connect:</span>
              <textarea class="invite-code" readonly rows="2" value={cur.invite}></textarea>
              <button class="ghost small" onclick={copyInvite}>Copy code</button>
            </div>
          {/if}
          {#if cur?.isDm}
            {@render contextNav(true)}
            {@render youPanel()}
          {/if}
        {:else}
        <div class="server-head">
          <strong class="server-title" title={cur?.name}>{cur?.name ?? ""}</strong>
          <button class="ghost icon-btn" title="Server settings" onclick={() => openServerSettings()}>{@render icoWrench()}</button>
        </div>
        {@render contextNav(false)}

        {#if canInvite || cur?.invite}
          <button class="ghost invite-quick" onclick={() => openServerSettings(null, "invites")}>＋ Invite someone</button>
        {/if}
        {@render youPanel()}
        {/if}
      </aside>

      <section class="channel">
        {#if dmHome && !cur}
          <div class="dm-placeholder">
            <h2>Direct messages</h2>
            <p class="muted">Pick a conversation on the left, or start a <strong>New DM</strong> / <strong>Add a friend</strong> with their code. A DM is a private, end-to-end-encrypted 1:1: your identity here stays unlinkable to your servers.</p>
          </div>
        {:else}
        {#if cur}
          <!-- Surface strip: the content column's own nav. The sidebar is contextual to whatever
               is selected here (channels for chat, pages for the wiki, folders for files, …). -->
          <nav class="surface-bar" aria-label="Surfaces">
            <button type="button" class:active={view === "chat"} onclick={() => switchView("chat")}>
              <span class="sb-ico">#</span>chat
              {#if view !== "chat"}
                {#if mentionChannels.size > 0}
                  <span class="sb-mark at" title="You were mentioned">@</span>
                {:else if cur.unread.length}
                  <span class="sb-mark" title="Unread messages">●</span>
                {/if}
              {/if}
            </button>
            <button type="button" class:active={view === "files"} onclick={() => switchView("files")}>
              <span class="sb-ico">▤</span>files
              {#if files.length}<span class="tab-count">{files.length}</span>{/if}
            </button>
            <button type="button" class:active={view === "status"} onclick={() => switchView("status")}>
              <span class="sb-ico">◇</span>status
            </button>
            <button type="button" class:active={view === "wiki"} onclick={() => switchView("wiki")}>
              <span class="sb-ico">✎</span>wiki
            </button>
            <button type="button" class:active={view === "events"} onclick={() => switchView("events")}>
              <span class="sb-ico">⧗</span>events
              {#if upcomingEvents.length}<span class="tab-count">{upcomingEvents.length}</span>{/if}
            </button>
            <button type="button" class:active={view === "downloads"} onclick={() => switchView("downloads")}>
              <span class="sb-ico">↓</span>transfers
              {#if activeDownloads}<span class="tab-count">{activeDownloads}</span>{/if}
            </button>
          </nav>
        {/if}
        {#if view === "chat"}
          <!-- Header: identity on the left, the channel's description filling the middle, every
               action on the right. The member count lives in the members column, not here. -->
          <h2 class="chan-head">
            <span class="chan-title"><span class="ch-hash">#</span>{activeName()}</span>
            {#if editingTopic}
              <!-- svelte-ignore a11y_autofocus -->
              <input
                class="topic-edit"
                bind:value={topicDraft}
                placeholder="Set a channel topic… (Enter to save, Esc to cancel)"
                maxlength="256"
                autofocus
                onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); saveTopic(); } else if (e.key === "Escape") { e.preventDefault(); editingTopic = false; } }}
                onblur={() => (editingTopic = false)}
              />
            {:else if channelTopic}
              <button type="button" class="chan-topic" title={`${channelTopic}\n\nClick to edit (any member can)`} onclick={() => { topicDraft = channelTopic; editingTopic = true; }}>{channelTopic}</button>
            {:else}
              <button type="button" class="chan-topic empty" title="Set a channel topic (any member can)" onclick={() => { topicDraft = ""; editingTopic = true; }}>+ topic</button>
            {/if}
            <span class="head-actions">
              {#if firstUnreadIdx >= 0}
                <button class="ghost small jump-unread" title="Jump to where you left off" onclick={() => scrollToMatch(firstUnreadIdx)}>↑ {unreadCount} new</button>
              {/if}
              <span class="chip ok" title="Messages in this group are end-to-end encrypted (MLS)">MLS · E2E</span>
              <button class="ghost icon-btn search-toggle" title="Search messages (Ctrl+F · Ctrl+Shift+F for filters)" aria-label="Search messages" onclick={() => openSearch()}>{@render icoSearch()}</button>
              {#if pinnedMsgs.length}
                <button class="ghost small pinned-toggle btn-ico" class:active={showPinned} title="Pinned messages" onclick={() => (showPinned = !showPinned)}>{@render icoPin()} {pinnedMsgs.length}</button>
              {/if}
              {#if liveElsewhere}
                {@const live = liveElsewhere}
                <button
                  class="ghost small call-start btn-ico"
                  title={`Voice live in #${live.name}: click to join`}
                  onclick={() => { if (activeServerId !== null) joinVoice(live.id, activeServerId, live.name); }}
                >{@render icoSpeaker()} live #{live.name} · {live.n}</button>
              {/if}
              {#if !(inCall && callChannel === cur?.active)}
                {@const n = roomMembers(activeServerId ?? -1, cur?.active ?? "").length}
                <button class="ghost small call-start btn-ico" title="Join this channel's voice room (E2E)" onclick={joinActiveVoice}>{@render icoPhone()} {n ? `Join voice (${n})` : "Voice"}</button>
              {/if}
            </span>
          </h2>
          {#if showPinned && pinnedMsgs.length}
            <div class="pinned-panel">
              <div class="pinned-head"><strong class="btn-ico">{@render icoPin()} Pinned</strong><button class="ghost small" onclick={() => (showPinned = false)}>✕</button></div>
              <ul class="pinned-list">
                {#each pinnedMsgs as p (p.id)}
                  <li>
                    <button class="pinned-item" onclick={() => { showPinned = false; jumpToMessageId(p.id); }}>
                      <strong>{nameOf(p.author)}</strong>
                      <span class="pinned-text">{msgSnippet(p.text, 80)}</span>
                    </button>
                    {#if canModerate && !cur?.isDm}
                      <button class="ghost small pinned-unpin" title="Unpin" onclick={() => togglePin(p)}>✕</button>
                    {/if}
                  </li>
                {/each}
              </ul>
            </div>
          {/if}
          {#if showSearch}
            <div class="msg-search">
              <input
                bind:this={searchInput}
                value={searchQuery}
                placeholder={filterCount ? "Filtering: add text to narrow further…" : "Search this channel…"}
                oninput={(e) => onSearchInput(e.currentTarget.value)}
                onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); stepMatch(e.shiftKey ? -1 : 1); } else if (e.key === "Escape") { e.preventDefault(); closeSearch(); } }}
              />
              <span class="muted small">
                {searchQuery.trim() || filterCount ? (searchMatches.length ? `${searchPosClamped + 1} / ${searchMatches.length}` : "no matches") : ""}
              </span>
              <button class="ghost small" title="Previous (Shift+Enter)" disabled={!searchMatches.length} onclick={() => stepMatch(-1)}>↑</button>
              <button class="ghost small" title="Next (Enter)" disabled={!searchMatches.length} onclick={() => stepMatch(1)}>↓</button>
              <button
                class="ghost small search-filters-toggle"
                class:active={showSearchAdv}
                title="Advanced filters (Ctrl+Shift+F)"
                aria-expanded={showSearchAdv}
                onclick={() => (showSearchAdv = !showSearchAdv)}
              >
                Filters{filterCount ? ` (${filterCount})` : ""}
              </button>
              <button class="ghost small" title="Close (Esc)" onclick={closeSearch}>✕</button>
            </div>
          {/if}
          {#if showSearch && showSearchAdv}
            <div class="search-adv">
              <div class="sa-row">
                <label class="sa-field">
                  <span class="muted small">In</span>
                  <select bind:value={filters.channel} onchange={() => { loadScope(); refilter(); }}>
                    <option value="">This channel</option>
                    <option value="*">All channels ({searchChannels.length})</option>
                    {#each searchChannels as c (c.id)}
                      <option value={c.id}>#{c.name}</option>
                    {/each}
                  </select>
                </label>
                <div class="sa-field">
                  <span class="muted small">From</span>
                  {@render personPicker(fromPick, filters.from, (fp) => (filters.from = fp), "Anyone")}
                </div>
                <div class="sa-field">
                  <span class="muted small">Mentions</span>
                  {@render personPicker(mentionPick, filters.mentions, (fp) => (filters.mentions = fp), "Anyone")}
                </div>
                <label class="sa-field">
                  <span class="muted small">Sort</span>
                  <select bind:value={filters.sort} onchange={refilter}>
                    <option value="oldest">Oldest first</option>
                    <option value="newest">Newest first</option>
                    <option value="author">Name (A–Z)</option>
                    <option value="reactions">Most reactions</option>
                    <option value="replies">Most replies</option>
                  </select>
                </label>
                {#if scopeLoading}<span class="muted small">loading channels…</span>{/if}
              </div>
              <div class="sa-row">
                <span class="muted small sa-label">When</span>
                <label class="sa-field">
                  <span class="muted small">After</span>
                  <input type="date" bind:value={filters.after} onchange={refilter} />
                </label>
                <label class="sa-field">
                  <span class="muted small">Before</span>
                  <input type="date" bind:value={filters.before} onchange={refilter} />
                </label>
                <button class="ghost small" title="Today only" onclick={() => quickRange(0)}>Today</button>
                <button class="ghost small" title="The last 7 days" onclick={() => quickRange(7)}>7d</button>
                <button class="ghost small" title="The last 30 days" onclick={() => quickRange(30)}>30d</button>
                <button
                  class="ghost small"
                  title="Clear the date range"
                  disabled={!filters.after && !filters.before}
                  onclick={() => { filters.after = ""; filters.before = ""; refilter(); }}
                >Any time</button>
              </div>
              <div class="sa-row">
                <span class="muted small sa-label">Has</span>
                {@render fchip("Image", filters.hasImage, (v) => (filters.hasImage = v), "Messages embedding an image")}
                {@render fchip("Video", filters.hasVideo, (v) => (filters.hasVideo = v), "Messages embedding a video")}
                {@render fchip("Audio", filters.hasAudio, (v) => (filters.hasAudio = v), "Messages embedding an audio clip")}
                {@render fchip("File", filters.hasFile, (v) => (filters.hasFile = v), "Messages with a non-media attachment")}
                {@render fchip("Link", filters.hasLink, (v) => (filters.hasLink = v), "Messages containing an http(s) link")}
              </div>
              <div class="sa-row">
                <span class="muted small sa-label">Is</span>
                {@render fchip("Reply", filters.isReply, (v) => (filters.isReply = v), "Messages that reply to another")}
                {@render fchip("Has replies", filters.hasReplies, (v) => (filters.hasReplies = v), "Messages someone replied to")}
                {@render fchip("Pinned", filters.isPinned, (v) => (filters.isPinned = v), "Pinned messages")}
                {@render fchip("Edited", filters.isEdited, (v) => (filters.isEdited = v), "Messages edited after sending")}
                {@render fchip("Mentions me", filters.mentionsMe, (v) => (filters.mentionsMe = v), "Messages that @-mention you")}
                {@render fchip("From me", filters.fromMe, (v) => (filters.fromMe = v), "Your own messages")}
              </div>
              <div class="sa-row">
                <span class="muted small sa-label">Reactions</span>
                {@render fchip("Any", filters.reacted, (v) => (filters.reacted = v), "Messages with at least one reaction")}
                {@render fchip("Mine", filters.reactedByMe, (v) => (filters.reactedByMe = v), "Messages you reacted to")}
                <select class="sa-emoji" bind:value={filters.emoji} onchange={refilter} title="A specific reaction">
                  <option value="">Any emoji</option>
                  {#each searchEmoji as e (e)}
                    <option value={e}>{e}</option>
                  {/each}
                </select>
              </div>
              <div class="sa-row">
                <span class="muted small sa-label">Match</span>
                {@render fchip("Aa", filters.caseSensitive, (v) => (filters.caseSensitive = v), "Case-sensitive text match")}
                {@render fchip("Whole word", filters.wholeWord, (v) => (filters.wholeWord = v), "Match the query as a whole word, not as part of one")}
                <button class="ghost small sa-clear" disabled={!filterCount} onclick={() => { clearFilters(); refilter(); }}>Clear filters</button>
              </div>
              {#if searchMatches.length}
                <ul class="search-results">
                  {#each searchMatches.slice(0, SEARCH_RESULT_CAP) as h, ri (h.ch + ":" + h.idx)}
                    <li>
                      <button class="search-result" class:current={h === searchCur} onclick={() => goToHit(h, ri)}>
                        {#if filters.channel}<span class="sr-ch muted small">#{channelName(h.ch)}</span>{/if}
                        <span class="sr-name">{nameOf(h.m.author)}</span>
                        <span class="sr-ts muted small">{new Date(h.m.ts).toLocaleString()}</span>
                        {#if reactionCount(h.m)}<span class="sr-rx muted small">♥ {reactionCount(h.m)}</span>{/if}
                        {#if h.m.id && corpusReplies.get(h.m.id)}<span class="sr-rx muted small">💬 {corpusReplies.get(h.m.id)}</span>{/if}
                        <span class="sr-text">{msgSnippet(h.m.text, 90)}</span>
                      </button>
                    </li>
                  {/each}
                </ul>
                {#if searchMatches.length > SEARCH_RESULT_CAP}
                  <p class="muted small sa-more">Showing the first {SEARCH_RESULT_CAP} of {searchMatches.length}: narrow the filters to see the rest.</p>
                {/if}
              {:else if searchQuery.trim() || filterCount}
                <p class="muted small sa-more">
                  {scopeLoading ? "Loading the other channels…" : "Nothing in scope matches."}
                </p>
              {/if}
            </div>
          {/if}
          <ul class="messages" bind:this={messagesEl} use:richClicks>
            {#each messages as m, mi}
              {@const newDay = mi === 0 || !sameDay(messages[mi - 1].ts, m.ts)}
              {#if newDay}
                <li class="day-divider" aria-hidden="true"><span>{dayLabel(m.ts)}</span></li>
              {/if}
              {#if mi === firstUnreadIdx}
                <li class="unread-divider" aria-hidden="true"><span>new · {unreadCount} unread</span></li>
              {/if}
              {@const grouped =
                mi > 0 &&
                mi !== firstUnreadIdx &&
                !newDay &&
                !m.reply_to &&
                messages[mi - 1].author === m.author &&
                m.ts - messages[mi - 1].ts < 300000}
              {@const bubble = appearance.flat ? "" : bubbleStyle(m.author)}
              {@const tick = deliveryTick(m)}
              {@const ident = identityOf(m.author)}
              <li
                data-mi={mi}
                class:own={m.author === myFp}
                class:grouped
                class:unread={isUnread(m)}
                class:pings-me={m.author !== myFp && mentionsMe(m.text)}
                class:has-bubble={!!bubble}
                class:search-match={showSearch && searchMatchSet.has(mi)}
                class:search-current={showSearch && searchCur?.ch === cur?.active && searchCur?.idx === mi}
                class:flash={!!m.id && m.id === flashId}
                style={bubble}
                use:contextMenu={() => messageMenu(m)}
              >
                {#if grouped}
                  <span class="t" title={new Date(m.ts).toLocaleString()}>
                    {#if tick}<span class="dtick {tick.cls}" title={tick.tip}>{tick.g}</span>{/if}{fmtTime(m.ts)}
                  </span>
                {:else}
                  <!-- Header rows: the avatar owns the gutter (dead space anyway) and the time
                       moves inline after the name, so the picture runs bigger for free. -->
                  <span class="t">
                    <button class="gutter-avatar" type="button" title="View profile" onclick={() => showProfile(ident.fp)}>
                      {@render avatarTag(ident.fp)}
                    </button>
                  </span>
                {/if}
                <div class="m-body">
                {#if m.reply_to}
                  {@const parent = msgById.get(m.reply_to)}
                  <button
                    class="reply-quote"
                    type="button"
                    title="Jump to the replied message"
                    onclick={() => jumpToMessageId(m.reply_to)}
                  >
                    <span class="reply-arrow">↰</span>
                    {#if parent}
                      {@render nameTag(parent.author)}<span class="muted"> {msgSnippet(parent.text, 60)}</span>
                    {:else}
                      <span class="muted">original message</span>
                    {/if}
                  </button>
                {/if}
                {#if !grouped}
                  <span class="author">
                    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                    <span class="author-link" role="button" tabindex="0" onclick={() => showProfile(ident.fp)}>
                      {@render nameTag(ident.fp)}
                    </span>
                    {#if ident.tag}<span class="dev-tag" title="Sent from this member's linked device">· {ident.tag}</span>{/if}
                    {#if m.author !== myFp && verifiedFps.has(m.author)}
                      <span class="vf-check" title="You verified this member out of band">✓</span>
                    {/if}
                    {#if badges[m.author]}
                      {@const b = badges[m.author]}
                      <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""} title="Badge assigned by a server admin">{b.label}</span>
                    {/if}
                    <span class="time" title={new Date(m.ts).toLocaleString()}>
                      {#if tick}<span class="dtick {tick.cls}" title={tick.tip}>{tick.g}</span>{/if}{fmtTime(m.ts)}
                    </span>
                    {#if m.pinned}<span class="pin-mark" title="Pinned message">{@render icoPin()}</span>{/if}
                  </span>
                {:else if m.pinned}
                  <span class="author"><span class="pin-mark" title="Pinned message">{@render icoPin()}</span></span>
                {/if}
                {#if m.id && editingId !== m.id}
                  <div class="msg-actions">
                    <button class="msg-action" type="button" title="Add reaction" aria-label="Add reaction" onclick={() => toggleReactionPicker(m)}>{@render icoCat()}</button>
                    <button class="msg-action" type="button" title="Reply" aria-label="Reply" onclick={() => startReply(m)}>↰</button>
                    {#if m.author === myFp}
                      <button class="msg-action" type="button" title="Edit" aria-label="Edit" onclick={() => startEdit(m)}>✎</button>
                    {/if}
                    <button class="msg-action" type="button" title="More actions" aria-label="More actions" onclick={(e) => openMenu(e, messageMenu(m))}>⋯</button>
                  </div>
                {/if}
                {#if m.id && editingId === m.id}
                  <div class="msg-edit">
                    <textarea
                      bind:value={editDraft}
                      rows="2"
                      onkeydown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); saveEdit(m); } else if (e.key === "Escape") { e.preventDefault(); cancelEdit(); } }}
                    ></textarea>
                    <div class="msg-edit-actions">
                      <button class="ghost small" onclick={() => saveEdit(m)}>Save</button>
                      <button class="ghost small" onclick={cancelEdit}>Cancel</button>
                      <span class="muted small">Enter to save · Esc to cancel</span>
                    </div>
                  </div>
                {:else}
                  <span class="text">{@html renderMessage(m.text, myMentionName)}{#if m.edited}<span class="edited-tag muted" title={"edited " + new Date(m.edited).toLocaleString()}> (edited)</span>{/if}</span>
                {/if}
                {#if m.id && replyCounts.get(m.id)}
                  {@const n = replyCounts.get(m.id)}
                  <button class="reply-count" type="button" title="Jump to the first reply" onclick={() => jumpToFirstReply(m.id)}>
                    💬 {n} {n === 1 ? "reply" : "replies"}
                  </button>
                {/if}
                {#if m.reactions.length || (m.id && reactionPickerFor === m.id)}
                  <div class="reactions">
                    {#each m.reactions as r (r.emoji)}
                      {@const rcode = customEmojiCode(r.emoji)}
                      <button
                        class="reaction"
                        class:mine={r.by.includes(myFp)}
                        title={r.by.map(nameOf).join(", ")}
                        aria-pressed={r.by.includes(myFp)}
                        aria-label={`${r.emoji}, ${r.by.length}, ${r.by.includes(myFp) ? "remove your reaction" : "react"}`}
                        onclick={() => toggleReaction(m, r.emoji)}
                      >
                        {#if rcode && emojiUrls[rcode]}
                          <img class="r-emoji-img" src={emojiUrls[rcode]} alt={r.emoji} />
                        {:else}
                          <span class="r-emoji">{r.emoji}</span>
                        {/if}
                        {r.by.length}
                      </button>
                    {/each}
                    {#if m.id}
                      <button class="reaction add-reaction" title="Add reaction" aria-label="Add reaction" onclick={() => toggleReactionPicker(m)}>＋</button>
                    {/if}
                    {#if m.id && reactionPickerFor === m.id}
                      <div class="reaction-picker" role="menu">
                        {#each QUICK_EMOJI as e}
                          <button class="qe" type="button" aria-label={`React with ${e}`} onclick={() => toggleReaction(m, e)}>{e}</button>
                        {/each}
                        {#each Object.keys(emojiMap) as code}
                          <button class="qe" type="button" aria-label={`React with :${code}:`} onclick={() => toggleReaction(m, `:${code}:`)}>
                            {#if emojiUrls[code]}<img src={emojiUrls[code]} alt={code} />{:else}<span class="muted small">:{code}:</span>{/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}
                {#if m.author === myFp}
                  {@const rc = deliveryReceipt(m, mi)}
                  {#if rc}
                    <span class="m-receipt {rc.cls}" title={rc.tip}><span class="mr-g">{rc.g}</span> {rc.label}</span>
                  {/if}
                {/if}
                </div>
              </li>
            {:else}
              <li class="muted">No messages yet: say hello.</li>
            {/each}
          </ul>
          <div class="composer-wrap">
            {#if mentionQuery !== null && mentionCandidates.length}
              <div class="mention-popup">
                {#each mentionCandidates as c, i}
                  <button
                    type="button"
                    class="mention-option"
                    class:active={i === mentionIdx}
                    onmousedown={(e) => { e.preventDefault(); pickMention(c); }}
                  >
                    {@render avatarTag(c.fp)}{@render nameTag(c.fp)}
                  </button>
                {/each}
              </div>
            {/if}
            {#if replyingTo}
              <div class="reply-banner">
                <span class="reply-arrow">↰</span>
                {#if replyTarget}
                  <span>Replying to {@render nameTag(replyTarget.author)}<span class="muted"> {msgSnippet(replyTarget.text, 60)}</span></span>
                {:else}
                  <span class="muted">Replying to a message</span>
                {/if}
                <button class="ghost small reply-cancel" type="button" title="Cancel reply (Esc)" onclick={cancelReply}>✕</button>
              </div>
            {/if}
            {#if showInsert && insertTarget === "chat"}
              {@render insertPanel()}
            {/if}
            {#if showEmoji}
              <div class="emoji-picker">
                {#if Object.keys(emojiMap).length}
                  <h4 class="emoji-set-label">server emoji</h4>
                  <div class="emoji-grid">
                    {#each Object.keys(emojiMap) as code}
                      <button class="emoji-pick" type="button" title={":" + code + ":"} onclick={() => insertEmoji(code)}>
                        {#if emojiUrls[code]}<img src={emojiUrls[code]} alt={code} />{:else}<span class="muted">:{code}:</span>{/if}
                      </button>
                    {/each}
                  </div>
                {/if}
                {#each EMOJI_SETS as set (set.label)}
                  <h4 class="emoji-set-label">{set.label}</h4>
                  <div class="emoji-grid">
                    {#each set.list as e (e)}
                      <button class="emoji-pick" type="button" onclick={() => insertUnicodeEmoji(e)}>{e}</button>
                    {/each}
                  </div>
                {/each}
              </div>
            {/if}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <form
              class="composer"
              class:drag-over={dragOver}
              ondragover={(e) => { e.preventDefault(); dragOver = true; }}
              ondragleave={() => (dragOver = false)}
              ondrop={(e) => onComposerDrop("chat", e)}
              onsubmit={(e) => { e.preventDefault(); send(); }}
            >
              <button
                type="button"
                class="attach ip-toggle"
                class:on={showInsert && insertTarget === "chat"}
                title="Link or embed a file, one of your status posts, or a wiki page"
                aria-label="Insert a link or embed"
                aria-expanded={showInsert && insertTarget === "chat"}
                onclick={() => toggleInsert("chat")}
              >{@render icoPlus()}</button>
              <label class="attach" title="Attach image / video / audio">
                📎
                <input
                  type="file"
                  accept="image/*,video/*,audio/*"
                  multiple
                  disabled={uploading}
                  onchange={(e) => { embedFiles("chat", e.currentTarget.files); e.currentTarget.value = ''; }}
                />
              </label>
              <textarea
                bind:this={composerEl}
                bind:value={draft}
                rows="1"
                class="composer-input"
                placeholder={uploading ? "Uploading…" : dragOver ? "Drop to embed…" : "Message #" + activeName()}
                oninput={onComposerInput}
                onkeydown={onComposerKeydown}
                onblur={() => queueMicrotask(() => (mentionQuery = null))}
              ></textarea>
              <span class="c-hint">enter to send · shift+enter newline</span>
              <button type="button" class="attach" title="Emoji" onclick={() => (showEmoji = !showEmoji)}>{@render icoCat()}</button>
              <button type="submit" disabled={uploading}>Send</button>
            </form>
          </div>
        {:else if view === "files"}
          <div class="files-head">
            <h2>Files <span class="muted">· {files.length}</span></h2>
            <nav class="breadcrumb">
              <button class="crumb" onclick={() => (folder = "")}>🏠</button>
              {#each breadcrumbs as seg, i}
                <span class="crumb-sep">/</span>
                <button class="crumb" onclick={() => gotoCrumb(i)}>{seg}</button>
              {/each}
            </nav>
          </div>
          <ul class="file-list tab-pane">
            {#each folderView.subs as sub}
              <li>
                <button class="folder-name" onclick={() => enterFolder(sub)}>📁 {sub}</button>
              </li>
            {/each}
            {#each folderView.here as f}
              {@const av = availOf(f)}
              <li use:contextMenu={() => fileMenu(f)}>
                <span class="file-avail {av.cls}" title={av.label}>{av.icon}</span>
                <button class="file-name" title="View file details" onclick={() => openFileInfo(f)}>
                  {fileIcon(f.mime)} {f.name}
                </button>
                {#if isPinned(f.cid)}
                  <span class="file-pin" title="Embedded in a wiki page: never drops out of circulation">📌</span>
                {/if}
                <span class="muted file-size">{fmtSize(f.size)} · {nameOf(f.author)} · <span class="avail-text {av.cls}">{av.label}</span></span>
              </li>
            {/each}
            {#if folderView.subs.length === 0 && folderView.here.length === 0}
              <li class="muted">This folder is empty.</li>
            {/if}
          </ul>
        {:else if view === "status"}
          <h2>Status</h2>
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <form
            class="composer"
            class:drag-over={dragOver}
            ondragover={(e) => { e.preventDefault(); dragOver = true; }}
            ondragleave={() => (dragOver = false)}
            ondrop={(e) => onComposerDrop("status", e)}
            onsubmit={(e) => { e.preventDefault(); postStatus(); }}
          >
            <label class="attach" title="Attach image / video / audio">
              📎
              <input
                type="file"
                accept="image/*,video/*,audio/*"
                multiple
                disabled={uploading}
                onchange={(e) => { embedFiles("status", e.currentTarget.files); e.currentTarget.value = ''; }}
              />
            </label>
            <input bind:value={statusDraft} placeholder={uploading ? "Uploading…" : dragOver ? "Drop to embed…" : "Post a status…"} />
            <button type="submit" disabled={uploading}>Post</button>
          </form>
          <ul class="status-list tab-pane" bind:this={statusEl} use:richClicks>
            {#each statuses as s}
              <li data-sid={s.id} class:flash={!!s.id && s.id === flashStatusId}>
                <span class="status-head">
                  {@render avatarTag(s.author)}
                  {@render nameTag(s.author)}
                  <span class="time">{fmtTime(s.ts)}</span>
                </span>
                <span class="status-text">{@html renderMessage(s.text, myMentionName)}</span>
              </li>
            {:else}
              <li class="muted">No status posts yet.</li>
            {/each}
          </ul>
        {:else if view === "wiki"}
          <div class="wiki">
            {#if wikiReviewOpen && canModerate}
              <!-- The admin review surface replaces the article area: every pending member
                   edit on this server, oldest first, each diffed against the live page. -->
              <div class="wiki-review">
                <div class="wiki-review-head">
                  <h2 class="wiki-head">
                    <span class="wiki-head-name">Pending changes</span>
                    {#if wikiPending.length}<span class="wiki-review-count">{wikiPending.length}</span>{/if}
                  </h2>
                  <span class="wiki-tb-spacer"></span>
                  <button class="ghost small" title="Back to the article view" onclick={() => (wikiReviewOpen = false)}>close</button>
                </div>
                <p class="muted small wiki-review-blurb">
                  A proposal publishes when you approve it, or auto-accepts at its deadline.
                  Declining keeps the page as it is and records the proposal in its history.
                </p>
                {#if wikiPending.length === 0}
                  <div class="wiki-review-empty muted">Nothing is awaiting review.</div>
                {:else}
                  <ul class="wiki-review-list">
                    {#each wikiPending as p (p.id)}
                      <!-- A page absent from wikiMap means the proposal creates it: diff base "". -->
                      {@const lines = diffLines(wikiMap[p.page] ?? "", p.body)}
                      {@const stats = diffStats(lines)}
                      <li class="wiki-review-item">
                        <div class="wiki-review-item-head">
                          <button class="wikilink wiki-review-page" title="Open the live page" onclick={() => openWikiPage(p.page)}>{p.page}</button>
                          {#if !(p.page in wikiMap)}<span class="wiki-review-new">new page</span>{/if}
                          <span class="wiki-diff-stats"><span class="add">+{stats.added}</span> <span class="del">-{stats.removed}</span></span>
                        </div>
                        <div class="wiki-review-item-meta">
                          {@render avatarTag(p.author)}
                          {@render nameTag(p.author)}
                          <span class="muted small">submitted {fmtTime(p.ts)}</span>
                          <span class="wiki-review-deadline">auto-accepts {fmtTime(p.expires_ts)}</span>
                        </div>
                        <pre class="wiki-diff wiki-review-diff">{#each lines as l}<span class="dl {l.kind}">{l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "} {l.text}
</span>{/each}</pre>
                        <div class="wiki-review-actions">
                          <button class="wiki-review-approve" title="Publish this edit now" onclick={() => approveWikiEdit(p)}>Approve</button>
                          <button class="wiki-review-decline" title="Turn this edit down; the page stays as it is" onclick={() => declineWikiEdit(p)}>Decline</button>
                        </div>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {:else if activeWikiPage}
              <div class="wiki-editor">
                <div class="wiki-editor-head">
                  <h2 class="wiki-head">
                    {#if wikiRenaming}
                      <!-- svelte-ignore a11y_autofocus -->
                      <input
                        class="topic-edit wiki-rename"
                        bind:value={wikiRenameTo}
                        placeholder="New page name… (Enter to rename, Esc to cancel)"
                        maxlength="120"
                        autofocus
                        onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); commitWikiRename(); } else if (e.key === "Escape") { e.preventDefault(); wikiRenaming = false; } }}
                        onblur={() => (wikiRenaming = false)}
                      />
                    {:else}
                      <span class="wiki-head-name">{activeWikiPage}</span>
                      <span class="chip wiki-fmt" title="This page's formatting language: set it per page with the md / wiki switch in Edit mode">{wikiFormat === "wiki" ? "wikitext" : "markdown"}</span>
                      {#if wikiDirty}<span class="muted small">· unsaved</span>{/if}
                    {/if}
                  </h2>
                  <div class="wiki-head-tools">
                    {#if !wikiRenaming && wikiPages.includes(activeWikiPage)}
                      <button class="ghost small" class:active={showWikiHistory} title="Every revision of this page: who changed what, when; restore any of them" onclick={() => (showWikiHistory ? (showWikiHistory = false) : openWikiHistory())}>history</button>
                      {#if wikiReviewDays === 0 || canModerate}
                        <button class="ghost small" title="Rename this page (links to the old name go red; they aren't rewritten)" onclick={startWikiRename}>rename</button>
                        {#if wikiDeleteArmed}
                          <button class="ghost small wiki-del armed" title="This deletes the page for every member" onclick={() => deleteWikiPage(activeWikiPage)}>confirm delete</button>
                        {:else}
                          <button class="ghost small wiki-del" title="Delete this page (for every member)" onclick={armWikiDelete}>delete</button>
                        {/if}
                      {/if}
                    {/if}
                    <div class="wiki-mode">
                      <button class:active={!wikiEdit && !showWikiHistory} onclick={() => { wikiEdit = false; showWikiHistory = false; }}>Read</button>
                      <button class:active={wikiEdit} onclick={() => { wikiEdit = true; showWikiHistory = false; }}>Edit</button>
                    </div>
                  </div>
                </div>
                {#if wikiEdit}
                  <div class="wiki-toolbar">
                    <div class="wiki-fmt-toggle" role="group" aria-label="Page format">
                      <button class:active={wikiFormat === "md"} title="Markdown: **bold**, # Heading. The format is a page property, shared with everyone" onclick={() => setWikiPageFormat("md")}>md</button>
                      <button class:active={wikiFormat === "wiki"} title="Wikitext: '''bold''', == Heading ==. The format is a page property, shared with everyone" onclick={() => setWikiPageFormat("wiki")}>wiki</button>
                    </div>
                    <span class="wiki-tb-sep"></span>
                    <button class="wiki-tb" title={`Bold (${wikiFormat === "wiki" ? "'''text'''" : "**text**"}) · Ctrl+B`} onclick={() => wikiWrap(wikiFormat === "wiki" ? "'''" : "**")}><b>B</b></button>
                    <button class="wiki-tb" title={`Italic (${wikiFormat === "wiki" ? "''text''" : "*text*"}) · Ctrl+I`} onclick={() => wikiWrap(wikiFormat === "wiki" ? "''" : "*")}><i>I</i></button>
                    <button class="wiki-tb" title={`Section heading (${wikiFormat === "wiki" ? "== Heading ==" : "## Heading"})`} onclick={() => wikiHeading(2)}>H2</button>
                    <button class="wiki-tb" title={`Subsection (${wikiFormat === "wiki" ? "=== Heading ===" : "### Heading"})`} onclick={() => wikiHeading(3)}>H3</button>
                    <button class="wiki-tb" title="Link to a page ([[Page]], add |label for display text)" onclick={() => wikiWrap("[[", "]]", "Page Name")}>[[&nbsp;]]</button>
                    <button class="wiki-tb" title="Bulleted list" onclick={() => wikiList(false)}>•≡</button>
                    <button class="wiki-tb" title="Numbered list" onclick={() => wikiList(true)}>1≡</button>
                    <button class="wiki-tb" title="Insert a table" onclick={insertWikiTable}>⊞</button>
                    <button class="wiki-tb" title="Insert an infobox: the summary card that floats at the top right of the page (one per page)" onclick={insertWikiInfobox}>▤</button>
                    <span class="wiki-tb-sep"></span>
                    <div class="wiki-ip-anchor">
                      <button
                        class="wiki-tb"
                        class:active={showInsert && insertTarget === "wiki"}
                        title="Link or embed a shared file, a status post, another page, or an event"
                        aria-expanded={showInsert && insertTarget === "wiki"}
                        onclick={() => toggleInsert("wiki")}
                      >+ insert</button>
                      {#if showInsert && insertTarget === "wiki"}
                        {@render insertPanel()}
                      {/if}
                    </div>
                    <label class="wiki-tb wiki-tb-attach" title="Upload an image / video / audio and embed it at the caret">
                      📎 attach
                      <input type="file" accept="image/*,video/*,audio/*" multiple disabled={uploading}
                        onchange={(e) => { wikiEmbed(e.currentTarget.files); e.currentTarget.value = ''; }} />
                    </label>
                    <span class="wiki-tb-spacer"></span>
                    <button class="wiki-tb wide" class:active={wikiPreview} title="Live preview beside the editor" onclick={() => (wikiPreview = !wikiPreview)}>preview</button>
                  </div>
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="wiki-edit"
                    class:split={wikiPreview}
                    class:drag-over={dragOver}
                    ondragover={(e) => { e.preventDefault(); dragOver = true; }}
                    ondragleave={() => (dragOver = false)}
                    ondrop={onWikiDrop}
                  >
                    <textarea bind:this={wikiTextarea} bind:value={wikiBody} oninput={() => (wikiDirty = true)} onkeydown={onWikiEditKey} rows="18"
                      placeholder={wikiFormat === "wiki"
                        ? "Wikitext. == Heading ==, '''bold''', ''italic'', [[Page]] or [[Page|label]] links, * bullet / # numbered lists; drop or attach a file to embed it."
                        : "Markdown. # Heading, **bold**, *italic*, [[Page]] or [[Page|label]] links, - lists; drop or attach a file to embed it."}></textarea>
                    {#if wikiPreview}
                      <div class="wiki-render preview" bind:this={wikiPreviewEl} use:richClicks>{@html renderWiki(wikiBody, wikiFormat)}</div>
                    {/if}
                  </div>
                  <div class="wiki-edit-actions">
                    <button onclick={saveWikiPage} disabled={!wikiDirty}>Save page</button>
                    <span class="muted small">Ctrl+S saves · concurrent edits merge · drop a file anywhere in the editor to embed it</span>
                  </div>
                {:else if showWikiHistory}
                  <!-- The history browser: revisions on the left (newest first), the selected
                       revision's changes against its predecessor on the right. -->
                  <div class="wiki-history">
                    <ul class="wiki-hist-list">
                      {#each [...wikiHistory].reverse() as r (r.id)}
                        <li>
                          <button class="wiki-hist-row" class:active={r.id === wikiHistorySel} class:rejected={r.kind === "reject"} onclick={() => (wikiHistorySel = r.id)}>
                            <span class="wiki-hist-when">{fmtTime(r.ts)}</span>
                            <span class="wiki-hist-who">{@render nameTag(r.author)}</span>
                            <span class="wiki-hist-kind {r.kind}">{wikiRevLabel(r.kind)}</span>
                            {#if r.kind === "rename" && r.note}<span class="muted small">from "{r.note}"</span>{/if}
                            {#if (r.kind === "approve" || r.kind === "reject") && r.actor}<span class="muted small">by {@render nameTag(r.actor)}</span>{/if}
                          </button>
                        </li>
                      {:else}
                        <li class="muted small">No recorded revisions yet: history starts with the next edit.</li>
                      {/each}
                    </ul>
                    {#if wikiSelRev}
                      <div class="wiki-hist-detail">
                        <div class="wiki-hist-detail-head">
                          {#if wikiSelDiff.length}
                            {@const stats = diffStats(wikiSelDiff)}
                            <span class="wiki-diff-stats"><span class="add">+{stats.added}</span> <span class="del">-{stats.removed}</span></span>
                          {/if}
                          <span class="muted small">{wikiSelPrev ? "changes vs the previous revision" : "the first recorded revision"}</span>
                          <span class="wiki-tb-spacer"></span>
                          {#if wikiSelRev.kind !== "reject" && wikiSelRev.body !== wikiBody}
                            <button class="ghost small" title="Make this revision's text the current page body (recorded as a new revision; nothing is erased)" onclick={() => restoreWikiRev(wikiHistorySel)}>restore this version</button>
                          {/if}
                        </div>
                        <pre class="wiki-diff">{#each wikiSelDiff as l}<span class="dl {l.kind}">{l.kind === "add" ? "+" : l.kind === "del" ? "-" : " "} {l.text}
</span>{/each}</pre>
                      </div>
                    {/if}
                  </div>
                {:else}
                  <div class="wiki-render article">
                    <div class="wiki-title">
                      {activeWikiPage}
                      {#if wikiRedirectedFrom}
                        <div class="wiki-redirect-note">↪ Redirected from <button class="wikilink" onclick={() => openWikiPage(wikiRedirectedFrom, { noRedirect: true })}>{wikiRedirectedFrom}</button></div>
                      {/if}
                    </div>
                    {#if myWikiPendingHere.length}
                      <div class="wiki-pending-note">
                        ⧗ Your edit is awaiting review: it publishes when an admin approves it, or automatically by {fmtTime(myWikiPendingHere[myWikiPendingHere.length - 1].expires_ts)}.
                      </div>
                    {/if}
                    {#if showWikiToc}
                      <nav class="wiki-toc" aria-label="Contents">
                        <div class="wiki-toc-h">
                          <span>contents</span>
                          <button class="wiki-toc-toggle" onclick={() => (wikiTocCollapsed = !wikiTocCollapsed)}>[{wikiTocCollapsed ? "show" : "hide"}]</button>
                        </div>
                        {#if !wikiTocCollapsed}
                          <ol>
                            {#each wikiToc as t}
                              <li class={`toc-lv${t.level}`}>
                                <button class="wiki-toc-link" onclick={() => scrollToWikiHeading(t.id)}><span class="toc-num">{t.num}</span>{t.text}</button>
                              </li>
                            {/each}
                          </ol>
                        {/if}
                      </nav>
                    {/if}
                    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                    <div class="wiki-body" bind:this={wikiEl} use:richClicks onclick={onWikiBodyClick}>{@html renderWiki(wikiBody, wikiFormat)}</div>
                    {#if backlinks.length}
                      <div class="wiki-backlinks">
                        <h4>What links here</h4>
                        <ul>
                          {#each backlinks as b}
                            <li><button class="wikilink" onclick={() => openWikiPage(b)}>{b}</button></li>
                          {/each}
                        </ul>
                      </div>
                    {/if}
                  </div>
                {/if}
              </div>
            {:else}
              <div class="wiki-empty-state">
                <p class="muted">Select a page on the left, or create one.</p>
                <p class="muted small">Link pages with <code>[[Page Name]]</code>: a <span class="wikilink missing">red link</span> creates its page.
                  Each page is written in <strong>Markdown</strong> or <strong>Wikitext</strong> (the <code>md / wiki</code> switch in Edit mode),
                  and 3+ headings get an automatic contents box.</p>
              </div>
            {/if}
          </div>
        {:else if view === "profile"}
          <h2>Your profile</h2>
          {@render profileEditor()}
        {:else if view === "events"}
          <h2>Events</h2>
          <div class="events-tab tab-pane">
            <form class="event-form" onsubmit={(e) => { e.preventDefault(); createEvent(); }}>
              <input bind:value={evTitle} maxlength="120" placeholder="Event title" />
              <div class="event-times">
                <label><span class="muted small">Starts</span><input type="datetime-local" bind:value={evStart} /></label>
                <label><span class="muted small">Ends (optional)</span><input type="datetime-local" bind:value={evEnd} /></label>
              </div>
              <textarea bind:value={evBody} rows="2" maxlength="1024" placeholder="Details (optional)"></textarea>
              <div class="ev-image-row">
                {#if evImage}
                  <span class="ev-image-pick">
                    {#if mediaUrls[evImage]}
                      <img class="ev-image-preview" src={mediaUrls[evImage]} alt="The event's poster" />
                    {:else}
                      <span class="muted small">Image attached</span>
                    {/if}
                    <button type="button" class="ghost small" onclick={() => (evImage = "")}>Remove image</button>
                  </span>
                {:else}
                  <label class="ev-image-add ghost small">
                    {evImageBusy ? "Uploading…" : "＋ Add an image"}
                    <input
                      type="file"
                      accept="image/*"
                      disabled={evImageBusy}
                      onchange={(e) => { const t = e.currentTarget; void pickEventImage(t.files).then(() => (t.value = "")); }}
                    />
                  </label>
                  <span class="muted small">Shown on the event and on every link to it.</span>
                {/if}
              </div>
              <button disabled={!evTitle.trim() || !evStart}>Create event</button>
            </form>
            <h3 class="ev-h"><span>Upcoming: {upcomingEvents.length}</span></h3>
            <ul class="event-list">
              {#each upcomingEvents as e (e.id)}
                <li class="event-row" class:flash={flashEventId === e.id}>
                  <div class="ev-when">{fmtEventWhen(e)}</div>
                  <div class="ev-main">
                    <div class="ev-title">{e.title}</div>
                    {#if e.body}<div class="ev-body">{e.body}</div>{/if}
                    <div class="ev-meta">by {@render nameTag(e.author)}</div>
                  </div>
                  {#if e.image && mediaUrls[e.image]}
                    <img class="ev-poster" src={mediaUrls[e.image]} alt={`Poster for ${e.title}`} />
                  {/if}
                  {#if e.author === myFp || canModerate}
                    {#if confirmDeleteEventId === e.id}
                      <button class="ghost small danger-btn" onclick={() => deleteEvent(e.id)}>Confirm</button>
                    {:else}
                      <button class="ghost small danger-btn" onclick={() => (confirmDeleteEventId = e.id)}>Delete</button>
                    {/if}
                  {/if}
                </li>
              {:else}
                <li class="muted small">Nothing scheduled yet.</li>
              {/each}
            </ul>
            {#if pastEvents.length}
              <h3 class="ev-h"><span>Past: {pastEvents.length}</span></h3>
              <ul class="event-list past">
                {#each pastEvents as e (e.id)}
                  <li class="event-row">
                    <div class="ev-when">{fmtEventWhen(e)}</div>
                    <div class="ev-main">
                      <div class="ev-title">{e.title}</div>
                      <div class="ev-meta">by {@render nameTag(e.author)}</div>
                    </div>
                    {#if e.author === myFp || canModerate}
                      {#if confirmDeleteEventId === e.id}
                        <button class="ghost small danger-btn" onclick={() => deleteEvent(e.id)}>Confirm</button>
                      {:else}
                        <button class="ghost small danger-btn" onclick={() => (confirmDeleteEventId = e.id)}>Delete</button>
                      {/if}
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {:else if view === "downloads"}
          <h2>Downloads</h2>
          <div class="downloads-tab tab-pane">
            {#if downloadList.length === 0}
              <p class="muted">No downloads yet. Open a file and click <strong>↓ Download</strong> to start one.</p>
            {:else}
              <div class="dl-toolbar">
                <span class="muted small">{activeDownloads} active · {downloadList.length} total</span>
              </div>
              <ul class="dl-list">
                {#each downloadList as d (d.cid)}
                  <li class="dl-item">
                    <div class="dl-item-main">
                      <span class="dl-item-name">{d.name}</span>
                      <span class="muted small">
                        {#if d.provider}from {nameOf(d.provider)}{:else}shared by {nameOf(d.author)}{/if}
                      </span>
                    </div>
                    <div class="dl-item-status {d.status}">
                      {#if d.status === "downloading"}{Math.round(d.progress * 100)}%
                      {:else if d.status === "queued"}Queued
                      {:else if d.status === "done"}✓ Done
                      {:else}✗ Failed{/if}
                    </div>
                    {#if d.status === "downloading" || d.status === "queued"}
                      <progress class="dl-item-bar" value={d.progress} max="1"></progress>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}
        {/if}
        {#if error}
          <div class="error-toast" role="alert">
            <span>{error}</span>
            <button class="error-x" aria-label="Dismiss" title="Dismiss" onclick={() => (error = "")}>✕</button>
          </div>
        {/if}
      </section>

      {#if !dmHome && cur && !cur.isDm}
        <aside class="members-col" aria-label="Members">
          <h3><span>Members · {onlineCount}/{members}</span></h3>
          {#if roster.length > 6}
            <input class="list-search" bind:value={rosterFilter} placeholder="Search members…" />
          {/if}
          {#if !filteredRoster.length}
            <p class="muted small">No matching members.</p>
          {/if}
          {#if onlineRoster.length}
            <h3><span>online: {onlineRoster.length}</span></h3>
            <ul>
              {#each onlineRoster as m (m.fingerprint)}
                {@render memberRow(m, true)}
                {@render companionRows(m.fingerprint)}
              {/each}
            </ul>
          {/if}
          {#if offlineRoster.length}
            <h3><span>offline: {offlineRoster.length}</span></h3>
            <ul>
              {#each offlineRoster as m (m.fingerprint)}
                {@render memberRow(m, false)}
                {@render companionRows(m.fingerprint)}
              {/each}
            </ul>
          {/if}
        </aside>
      {/if}
      {/if}
    </div>

    <footer class="statusbar">
      <span class="seg"><span class="sb-dot"></span><span class="ok-t">node online</span></span>
      {#if cur && !dmHome && !inboxView}
        <span class="seg">peers <span><span class="ok-t">{Math.max(onlineCount - 1, 0)}</span>/{Math.max(members - 1, 0)}</span></span>
      {/if}
      <button class="seg sb-lock" title="Lock now (Ctrl+L): clears everything on screen and asks for your passphrase again. The node stays online." onclick={lockScreen}>
        {@render icoLock()} vault <span class="ok-t">unlocked</span>
      </button>
      {#if rendezvous.trim()}<span class="seg">rendezvous <span class="ok-t">set</span></span>{/if}
      {#if activeDownloads}<span class="seg"><span class="warn-t">⇣ {activeDownloads} transfer{activeDownloads === 1 ? "" : "s"}</span></span>{/if}
      <span class="sb-spacer"></span>
      {#if myFp}<span class="seg" title="Your fingerprint on this server: click a member and compare out of band to verify">id {myFp.slice(0, 4)}·{myFp.slice(4, 8)}</span>{/if}
    </footer>

    <!--
      The voice stage. Two shapes of the same dock: a collapsed bar (glanceable) and the expanded
      stage (per-peer control). Mute is the one state that must never be ambiguous, so it gets a
      danger treatment plus an empty meter in both shapes.
    -->
    {#if inCall && !stageOpen && !focusOpen}
      <div class="call-bar">
        <span class="call-dot">{@render icoSpeaker()}</span>
        <span class="call-title">Voice · #{callChannelName}</span>
        <span class="call-status">{callStatusText}</span>
        <div class="call-avatars">
          {@render avatarTag(myFp)}
          {#each callParticipants as fp}{@render avatarTag(fp)}{/each}
        </div>
        {@render micMeter()}
        {#if videoAnnounced}
          <button class="ghost focus-chip" title="Open the video focus view" onclick={openFocus}>{@render icoCam()}<span class="stage-label">FOCUS</span></button>
        {/if}
        <button class="ghost small btn-ico stage-mute" class:muted={callMuted} title={callMuted ? "Unmute" : "Mute"} onclick={toggleMute}>{#if callMuted}{@render icoMicOff()} Muted{:else}{@render icoMic()} Mute{/if}</button>
        <button class="call-hangup btn-ico" title="Leave voice" onclick={leaveVoice}>{@render icoHangup()} Leave</button>
        <button class="ghost stage-chev" title="Open the voice stage" aria-label="Open the voice stage" onclick={() => (stageOpen = true)}>{@render icoChevUp()}</button>
      </div>
    {/if}

    {#if inCall && stageOpen && !focusOpen}
      <div class="stage">
        <header class="stage-head">
          <span class="stage-live"></span>
          <span class="stage-label">VOICE · #{callChannelName}</span>
          <span class="stage-spacer"></span>
          {#if callParticipants.length === 0}
            <span class="stage-tally solo">solo</span>
          {:else}
            <span class="stage-tally" class:partial={linksUp < callParticipants.length} title={callStatusText}>
              {linksUp}/{callParticipants.length} links up
            </span>
          {/if}
          {#if videoAnnounced}
            <button class="ghost focus-chip" title="Open the video focus view" onclick={openFocus}>{@render icoCam()}<span class="stage-label">FOCUS</span></button>
          {/if}
          <button class="ghost stage-chev" title="Collapse to the call bar" aria-label="Collapse to the call bar" onclick={() => (stageOpen = false)}>{@render icoChevDown()}</button>
        </header>

        {#if callParticipants.length === 0}
          <div class="stage-solo">
            <span class="stage-label">alone in #{callChannelName}</span>
            <p class="stage-solo-hint">You're the only one here. The channel pill lights up for others; the piano below is yours to noodle on.</p>
          </div>
        {:else}
          <ul class="stage-peers">
            {#each callParticipants as fp (fp)}
              {@const link = linkState(callPeerStates[fp] ?? "new")}
              {@const vol = peerVolumes[fp] ?? 1}
              <li class="stage-peer">
                <div class="stage-row">
                  <span class="stage-av" class:talking={speaking[fp]}>{@render catEars(fp)}{@render avatarTag(fp)}</span>
                  <span class="stage-nm">{@render nameTag(fp)}</span>
                  <span class="stage-spacer"></span>
                  {#if (remoteHeld[fp] ?? []).length > 0}
                    <span class="stage-playing" title="Playing right now">{@render icoNote()}</span>
                  {/if}
                  {#if peerMeta[fp]?.mic}
                    <span class="stage-permute" title="Their microphone is muted">{@render icoMicOff()}</span>
                  {/if}
                  {#if peerMeta[fp]?.inst}
                    <span class="stage-chip struck" title="They have muted incoming instruments: they can't hear you play">INST</span>
                  {/if}
                  <span class="stage-link {link}" title={`Link: ${callPeerStates[fp] ?? "new"}`}>{link === "est" ? "EST" : link === "neg" ? "NEG" : "LOST"}</span>
                </div>
                <!-- Per-peer trim lives one row down: it is fiddly, so it only appears on hover/focus. -->
                <div class="stage-trim">
                  <span class="stage-trim-ico">{@render icoSpeaker()}</span>
                  <input
                    class="stage-vol"
                    type="range"
                    min="0"
                    max="1"
                    step="0.05"
                    value={vol}
                    aria-label={`Volume for ${nameOf(fp)}`}
                    oninput={(e) => setPeerVolume(fp, Number(e.currentTarget.value))}
                  />
                  <span class="stage-pct">{Math.round(vol * 100)}%</span>
                  <button
                    class="ghost stage-tog"
                    class:on={voiceMutedPeers[fp]}
                    aria-pressed={!!voiceMutedPeers[fp]}
                    title={voiceMutedPeers[fp] ? "Hear their voice again" : "Mute their voice for you only"}
                    onclick={() => toggleVoicePeer(fp)}
                  >{@render icoMicOff()}</button>
                  <button
                    class="ghost stage-tog"
                    class:on={instMutedPeers[fp]}
                    aria-pressed={!!instMutedPeers[fp]}
                    title={instMutedPeers[fp] ? "Hear their instrument again" : "Mute their instrument for you only"}
                    onclick={() => toggleInstPeer(fp)}
                  >{@render icoNote()}</button>
                </div>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="stage-self">
          <div class="stage-row">
            <span class="stage-av" class:talking={speaking.me}>{@render avatarTag(myFp)}</span>
            <span class="stage-nm">{@render nameTag(myFp)}</span>
            <span class="stage-fp">{myFp.slice(0, 4)}·{myFp.slice(4, 8)}</span>
            <span class="stage-spacer"></span>
            {@render micMeter()}
          </div>
          <div class="stage-acts">
            <button class="ghost stage-act" class:muted={callMuted} title={callMuted ? "Unmute" : "Mute your microphone"} onclick={toggleMute}>
              {#if callMuted}{@render icoMicOff()}{:else}{@render icoMic()}{/if}
              <span class="stage-act-lbl">{callMuted ? "Muted" : "Mute"}</span>
            </button>
            <button class="ghost stage-act" class:muted={callDeafened} title={callDeafened ? "Hear the room again" : "Deafen: stop hearing everyone"} onclick={toggleDeafen}>
              {@render icoSpeaker()}
              <span class="stage-act-lbl">{callDeafened ? "Deafened" : "Deafen"}</span>
            </button>
            <!-- Camera and share share one video slot, so lighting one always dims the other. -->
            <button class="ghost stage-act" class:on={myVideo === "cam"} aria-pressed={myVideo === "cam"} title={myVideo === "cam" ? "Stop your camera" : "Send your camera"} onclick={() => (myVideo === "cam" ? stopVideo() : void startVideo("cam"))}>
              {@render icoCam()}
              <span class="stage-act-lbl">Cam</span>
            </button>
            <button class="ghost stage-act" class:on={myVideo === "screen"} aria-pressed={myVideo === "screen"} title={myVideo === "screen" ? "Stop sharing your screen" : "Share your screen"} onclick={() => (myVideo === "screen" ? stopVideo() : void startVideo("screen"))}>
              {@render icoScreen()}
              <span class="stage-act-lbl">Share</span>
            </button>
            <button class="ghost stage-act" class:on={instOpen} aria-expanded={instOpen} title="Instrument drawer" onclick={toggleInstDrawer}>
              {@render icoNote()}
              <span class="stage-act-lbl">Inst</span>
            </button>
            <button class="stage-act stage-leave" title="Leave voice" onclick={leaveVoice}>
              {@render icoHangup()}
              <span class="stage-act-lbl">Leave</span>
            </button>
          </div>
          <div class="stage-devs">
            <label class="stage-dev">
              <span class="stage-label">IN</span>
              <select value={micDev} onchange={(e) => setMicDevice(e.currentTarget.value)}>
                <option value="">System default</option>
                {#each audioIns as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
              </select>
            </label>
            {#if sinkSupported}
              <label class="stage-dev">
                <span class="stage-label">OUT</span>
                <select value={spkDev} onchange={(e) => setSpkDevice(e.currentTarget.value)}>
                  <option value="">System default</option>
                  {#each audioOuts as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
                </select>
              </label>
            {/if}
          </div>
        </div>

        <!-- The deck sits between what you do and what you play: it is the room's, not yours. -->
        {@render jukeDock()}

        <!-- The drawer itself is the shared instDrawer snippet; the fold strip below owns its state. -->
        {#if instOpen}
          {@render instDrawer()}
        {/if}
        <div class="stage-fold">
          <button class="ghost stage-fold-btn" aria-expanded={instOpen} title={instOpen ? "Close the instruments" : "Open the instruments"} onclick={toggleInstDrawer}>
            {#if instOpen}{@render icoChevDown()}{:else}{@render icoChevUp()}{/if}
            <span class="stage-label">INSTRUMENTS</span>
          </button>
          <span class="stage-spacer"></span>
          <button class="ghost stage-rx" class:off={instRxMuted} title={instRxMuted ? "You are not hearing anyone's instrument" : "You hear everyone's instrument"} onclick={toggleInstRx}>
            <span class="stage-rx-dot"></span>
            <span class="stage-label">{instRxMuted ? "INST MUTED" : "HEARING ALL"}</span>
          </button>
        </div>
      </div>
    {/if}

    <!--
      The focus view. Video is the one payload that cannot share the window with chat, so it takes
      the whole thing: one tile per person, self included, and every control the dock had. Exiting
      hands the window back and latches focusDismissed, so the same live video never grabs it again.
    -->
    {#if inCall && focusOpen}
      <div class="focus">
        <header class="focus-head">
          <span class="focus-live"></span>
          <span class="stage-label">VOICE · #{callChannelName} · FOCUS</span>
          <span class="stage-spacer"></span>
          {#if callParticipants.length === 0}
            <span class="stage-tally solo">solo</span>
          {:else}
            <span class="stage-tally" class:partial={linksUp < callParticipants.length} title={callStatusText}>
              {linksUp}/{callParticipants.length} links up
            </span>
          {/if}
          <span class="focus-e2e" title="Every frame rides the same end-to-end encrypted peer link as the voice">MLS·E2E</span>
          <button class="ghost focus-exit" title="Leave focus: back to chat and the voice dock" aria-label="Leave focus" onclick={exitFocus}>{@render icoFocusOut()}</button>
        </header>

        <div class="focus-grid" style={`--focus-cols:${focusCols}`}>
          {#each focusTiles as fp (fp)}
            {@const me = fp === myFp}
            {@const vid = me ? (myVideo === "screen" ? 2 : myVideo === "cam" ? 1 : 0) : peerMeta[fp]?.vid ?? 0}
            {@const stream = me ? localVideoStream : remoteStreams[fp] ?? null}
            {@const held = me ? callHeld : remoteHeld[fp] ?? []}
            {@const micOff = me ? callMuted : !!peerMeta[fp]?.mic}
            <div class="focus-tile" class:talking={speaking[me ? "me" : fp]}>
              {#if vid > 0 && stream}
                <!-- Always muted: voice arrives on the per-peer <audio> elements, and a second
                     unmuted path here would double every voice in the room. -->
                <!-- svelte-ignore a11y_media_has_caption -->
                <video class="focus-vid" class:contain={vid === 2} autoplay playsinline muted use:srcObject={stream}></video>
              {:else}
                <span class="focus-face">{@render catEars(me ? "me" : fp)}{@render avatarTag(fp)}</span>
              {/if}
              <span class="focus-name">
                <span class="focus-nm">{me ? "you" : nameOf(fp)}</span>
                {#if micOff}<span class="focus-nm-mute" title="Microphone muted">{@render icoMicOff()}</span>{/if}
                {#if held.length}<span class="focus-nm-note" title="Playing right now">{@render icoNote()}</span>{/if}
                {#if vid === 2}<span class="focus-nm-share">SHARING</span>{/if}
              </span>
            </div>
          {/each}
        </div>

        <!-- Mesh reality, stated once and only where it bites: every sender uploads per peer. -->
        {#if focusTiles.length > 5}
          <p class="focus-warn">mesh video is per-peer upload: large rooms will strain connections</p>
        {/if}

        <div class="focus-bar">
          <button class="ghost focus-btn" class:muted={callMuted} title={callMuted ? "Unmute" : "Mute your microphone"} aria-label={callMuted ? "Unmute" : "Mute"} onclick={toggleMute}>
            {#if callMuted}{@render icoMicOff()}{:else}{@render icoMic()}{/if}
          </button>
          <button class="ghost focus-btn" class:muted={callDeafened} title={callDeafened ? "Hear the room again" : "Deafen: stop hearing everyone"} aria-label="Deafen" onclick={toggleDeafen}>
            {@render icoSpeaker()}
          </button>
          <button class="ghost focus-btn" class:on={myVideo === "cam"} aria-pressed={myVideo === "cam"} title={myVideo === "cam" ? "Stop your camera" : "Send your camera"} aria-label="Camera" onclick={() => (myVideo === "cam" ? stopVideo() : void startVideo("cam"))}>
            {@render icoCam()}
          </button>
          <button class="ghost focus-btn" class:on={myVideo === "screen"} aria-pressed={myVideo === "screen"} title={myVideo === "screen" ? "Stop sharing your screen" : "Share your screen"} aria-label="Share your screen" onclick={() => (myVideo === "screen" ? stopVideo() : void startVideo("screen"))}>
            {@render icoScreen()}
          </button>
          <button class="ghost focus-btn" class:on={instOpen} aria-expanded={instOpen} title="Instrument drawer" aria-label="Instruments" onclick={toggleInstDrawer}>
            {@render icoNote()}
          </button>
          <button class="focus-btn focus-leave" title="Leave voice" aria-label="Leave voice" onclick={leaveVoice}>
            {@render icoHangup()}
          </button>
        </div>

        <div class="focus-dock juke-dock-slot">{@render jukeDock()}</div>

        {#if instOpen}
          <div class="focus-dock">{@render instDrawer()}</div>
        {/if}
      </div>
    {/if}

    <!--
      Add from share: the only way into the queue. Everything here is a listing this server already
      circulates, so queueing is a reference, never an upload. Sits above both call surfaces
      because it is opened from a dock that floats over the window.
    -->
    {#if inCall && jukePickerOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay juke-pick-over" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) jukePickerOpen = false; }}>
        <div class="overlay-card juke-pick">
          <header class="overlay-head">
            <h2>Add from share</h2>
            <button class="ghost" title="Close (Esc)" onclick={() => (jukePickerOpen = false)}>✕</button>
          </header>
          <div class="overlay-body">
            {#if jukeAudioFiles.length === 0}
              <p class="juke-pick-empty">no audio in this server's share yet: drop a file in chat or the Files surface to share it</p>
            {:else}
              <ul class="juke-pick-list">
                {#each jukeAudioFiles as f (f.cid + "|" + f.path)}
                  {@const days = jukeExpiryDays(f.cid)}
                  <li class="juke-pick-row">
                    <span class="juke-ext">{jukeExt(f)}</span>
                    <span class="juke-pick-main">
                      <span class="juke-pick-nm" title={f.name}>{f.name}</span>
                      <span class="juke-pick-sub">{fmtSize(f.size)} · shared by {nameOf(f.author)}</span>
                    </span>
                    <!-- Held is certain; anything short of it is a pull we will attempt, and only a
                         pull nobody can serve fails, which it does at play time and not here. -->
                    {#if f.held >= f.total}
                      <span class="juke-chip ok" title="Every chunk is already on this device">HELD</span>
                    {:else}
                      <span class="juke-chip info" title="Missing chunks are pulled from a peer when it plays">FETCHABLE</span>
                    {/if}
                    {#if days >= 0}
                      <span class="juke-chip warn" title="This listing drops out of circulation soon">EXPIRES {days}D</span>
                    {/if}
                    <button class="ghost juke-q" title={`Queue ${f.name}`} onclick={() => { void jukeAddTrack(f.cid, f.name); jukePickerOpen = false; }}>＋ QUEUE</button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    {#if voiceAlert}
      <div class="call-incoming">
        <span class="btn-ico">{@render icoPhone()} Voice call in <strong>#{voiceAlert.name}</strong></span>
        <button onclick={() => voiceAlert && joinVoice(voiceAlert.channel, voiceAlert.server, voiceAlert.name)}>Join</button>
        <button class="ghost" onclick={() => (voiceAlert = null)}>Dismiss</button>
      </div>
    {/if}

    {#if profileCard}
      {@const fp = profileCard}
      {@const p = profiles[fp]}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) profileCard = null; }}>
        <div class="overlay-card profile-card">
          <header class="overlay-head">
            <h2>Profile</h2>
            <button class="ghost" onclick={() => (profileCard = null)}>✕</button>
          </header>
          <div class="overlay-body">
            {#if p?.banner}
              <img class="pc-banner" src={imgSrc(p.banner)} alt="" />
            {/if}
            <div class="pc-top">
              {#if p?.avatar}
                <img class="avatar lg" src={imgSrc(p.avatar)} alt="" />
              {:else}
                <span class="avatar lg fallback" style={`background:${p?.color || "#4f8cff"}`}>{nameOf(fp).slice(0, 1).toUpperCase()}</span>
              {/if}
              <div class="pc-id">
                <div class="pc-name">{@render nameTag(fp)}</div>
                <div class="pc-meta">
                  {#if roles[fp] && roles[fp] !== "member"}<span class="role-badge {roles[fp]}">{roles[fp]}</span>{/if}
                  {#if fp === myFp}<span class="you-badge">you</span>{/if}
                  {#if fp !== myFp && verifiedFps.has(fp)}<span class="vf-check" title="You verified this member out of band">✓ verified</span>{/if}
                  {#if badges[fp]}
                    {@const b = badges[fp]}
                    <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""} title="Badge assigned by a server admin">{b.label}</span>
                  {/if}
                  <span class="muted small">{fp === myFp || onlineMembers.has(fp) ? "online" : "offline"}</span>
                </div>
              </div>
            </div>
            {#if p?.description}
              <p class="pc-desc">{p.description}</p>
            {:else}
              <p class="muted small">No description yet.</p>
            {/if}
            <div class="pc-actions">
              {#if fp === myFp}
                <button onclick={() => { profileCard = null; switchView("profile"); }}>Edit your profile</button>
              {:else}
                {#if !cur?.isDm && onlineMembers.has(fp)}
                  <button onclick={() => { const t = fp; profileCard = null; startDmWithMember(t); }}>👋 Add friend</button>
                {/if}
                <button class="ghost" onclick={() => { const t = fp; profileCard = null; verifyFor = t; }}>✓ Verify identity</button>
                <button class="ghost" onclick={() => copyText(fp)}>Copy fingerprint</button>
              {/if}
            </div>
          </div>
        </div>
      </div>
    {/if}

    {#if verifyFor}
      {@const vfp = verifyFor}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) verifyFor = null; }}>
        <div class="overlay-card verify-card">
          <header class="overlay-head">
            <h2>Verify {nameOf(vfp)}</h2>
            <button class="ghost" onclick={() => (verifyFor = null)}>✕</button>
          </header>
          <div class="overlay-body">
            <p class="muted small">
              Compare these fingerprints over a channel you already trust: a voice call, video,
              or in person. If both match, you're talking to the real device: no relay or network
              position can forge a fingerprint. Marking someone verified is a note for yourself:
              it's stored only on this device and never shared.
            </p>
            <div class="vf-block">
              <span class="vf-label">their fingerprint: {nameOf(vfp)} reads this to you</span>
              <code class="vf-fp">{fmtFp(vfp)}</code>
            </div>
            <div class="vf-block">
              <span class="vf-label">your fingerprint: you read this to them</span>
              <code class="vf-fp">{fmtFp(myFp)}</code>
            </div>
            <div class="pc-actions">
              {#if verifiedFps.has(vfp)}
                <button class="ghost" onclick={() => setVerified(vfp, false)}>Remove verified mark</button>
              {:else}
                <button onclick={() => { setVerified(vfp, true); verifyFor = null; }}>✓ They match: mark verified</button>
              {/if}
              <button class="ghost" onclick={() => copyText(`you: ${fmtFp(myFp)}\nthem (${nameOf(vfp)}): ${fmtFp(vfp)}`)}>Copy both</button>
            </div>
          </div>
        </div>
      </div>
    {/if}

    {#if scanOpen}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeScan(null); }}>
        <div class="overlay-card scan-card">
          <header class="overlay-head">
            <h2>Scan a code</h2>
            <button class="ghost" onclick={() => closeScan(null)}>✕</button>
          </header>
          <div class="overlay-body">
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={scanVideoEl} class="scan-video" playsinline muted></video>
            <p class="muted small">Point the camera at the QR code. Esc cancels.</p>
          </div>
        </div>
      </div>
    {/if}

    {#if showLinkDevice}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeLinkDevice(); }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>Link a device</h2>
            <button class="ghost" onclick={() => closeLinkDevice()}>✕</button>
          </header>
          <div class="overlay-body">
            {#if !linkBundle}
              <p class="muted small">
                On the new device, choose <em>Link this device</em> on its start screen and bring
                its pairing code here (paste, or read it across the room).
              </p>
              <textarea class="invite-code" rows="3" bind:value={linkBlob} placeholder="Paste the new device's pairing code…"></textarea>
              <div class="pc-actions">
                <button class="ghost small" disabled={linkBusy || !linkBlob.trim()} onclick={linkRead}>Read pairing code</button>
                <button class="ghost small" disabled={linkBusy || scanOpen} onclick={() => scanQr((t) => { if (t) { linkBlob = t; linkRead(); } })}>⛶ Scan QR</button>
                <button class="ghost small" disabled={linkBusy || soundBusy !== ""} onclick={() => listenForSound((t) => { linkBlob = t; linkRead(); })}>{soundBusy === "listen" ? "Listening…" : "🎙 Listen for sound"}</button>
              </div>
              {#if linkInfo}
                <div class="grant-box">
                  <div class="vf-label">grant device access?</div>
                  <div class="grant-sas">{linkInfo.deviceId.slice(0, 8)}</div>
                  <p class="muted small">
                    <strong>The check:</strong> the new device shows this same 8-character device
                    code on its screen: confirm they match before accepting.
                    <br />After you deliver the grant, the new device will also show code
                    <span class="fp">{fmtSas(linkInfo.sas)}</span>: verify it there as the final
                    step. Context (advisory only): pairing code pasted locally, just now.
                  </p>
                  <p class="muted small">
                    Accepting grants access to
                    <strong>{linkInfo.servers.length - linkInfo.dmCount} server{linkInfo.servers.length - linkInfo.dmCount === 1 ? "" : "s"}{linkInfo.dmCount ? ` and ${linkInfo.dmCount} DM${linkInfo.dmCount === 1 ? "" : "s"}` : ""}</strong>:
                    {linkInfo.servers.join(", ")}.
                  </p>
                  <label class="field">
                    <span class="muted small">Name for the new device (shown on its messages)</span>
                    <input bind:value={linkName} maxlength="24" placeholder="phone / laptop / deck…" />
                  </label>
                  <label class="field">
                    <span class="muted small">
                      Transport passphrase (min 8 characters): seals the grant for the trip; type
                      it again on the new device. Not your vault passphrase.
                    </span>
                    <input type="password" bind:value={linkPass} placeholder="transport passphrase" />
                  </label>
                  <div class="pc-actions">
                    <button disabled={linkBusy || linkPass.length < 8 || !linkName.trim()} onclick={linkMint}>✓ Codes match: grant access</button>
                    <button class="ghost" onclick={() => closeLinkDevice(true)}>Decline</button>
                  </div>
                </div>
              {/if}
            {:else}
              <p class="muted small">
                Grant minted for this device's servers. Carry it to the new device (paste it there
                with the transport passphrase). It is sealed: but treat it like a key until used.
              </p>
              <textarea class="invite-code" rows="4" readonly value={linkBundle}></textarea>
              {#if linkBundle.length <= QR_MAX_CHARS}
                <canvas class="qr-canvas" use:qr={linkBundle}></canvas>
              {:else}
                <p class="muted small">Too large for a QR: copy/paste it.</p>
              {/if}
              <div class="pc-actions">
                <button class="ghost" onclick={() => copyText(linkBundle)}>Copy grant</button>
                <button class="ghost" onclick={() => closeLinkDevice()}>Done</button>
              </div>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    {#if spaceOpen}
      <!-- The 360 server space. One fixed camera, rotation only: the CSS cube renders the
           backdrop while the icons are JS-projected with the same focal length, so the two
           layers always agree. Pointer story: drag looks, hold grows a lasso, click opens. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="space-view"
        class:sp-carrying={!!spaceCarried}
        data-backdrop={spaceBackdropEff}
        bind:this={spaceRoot}
        bind:clientWidth={spaceVw}
        bind:clientHeight={spaceVh}
        onpointerdown={onSpaceDown}
        onpointermove={onSpaceMove}
        onpointerup={onSpaceUp}
        onpointercancel={onSpaceUp}
        onclickcapture={onSpaceClickCapture}
      >
        <div class="sp-scene" style={`perspective:${spaceF}px`}>
          <!-- translateZ(f) first: CSS puts the eye f in front of the scene plane, so the cube
               must slide forward to centre on it. Without this the backdrop pans at roughly
               half the icons' angular rate (the icons' projection assumes an eye-centred cube). -->
          <div class="sp-cube" style={`transform: translateZ(${spaceF}px) rotateX(${spaceCam.pitch}deg) rotateY(${spaceCam.yaw}deg)`}>
            {#each [0, 90, 180, 270] as fy (fy)}
              <div
                class="sp-face sp-wall"
                style={`width:${2 * spaceF + 2}px; height:${2 * spaceF + 2}px; margin:${-spaceF - 1}px 0 0 ${-spaceF - 1}px; transform: rotateY(${-fy}deg) translateZ(${-spaceF}px);${
                  spaceBackdropEff === "custom"
                    ? ` background-image:url(${spaceState.custom}); background-size:400% 200%; background-repeat:repeat-x; background-position:${panoPos(fy)};`
                    : ""
                }`}
              >
                {#if spaceBackdropEff !== "custom"}
                  {@render spaceWall(spaceBackdropEff, fy)}
                {/if}
              </div>
            {/each}
            <div class="sp-face sp-ceil" style={`width:${2 * spaceF + 2}px; height:${2 * spaceF + 2}px; margin:${-spaceF - 1}px 0 0 ${-spaceF - 1}px; transform: rotateX(-90deg) translateZ(${-spaceF}px)`}></div>
            <div class="sp-face sp-floor" style={`width:${2 * spaceF + 2}px; height:${2 * spaceF + 2}px; margin:${-spaceF - 1}px 0 0 ${-spaceF - 1}px; transform: rotateX(90deg) translateZ(${-spaceF}px)`}></div>
          </div>
        </div>

        <svg class="sp-reticle" viewBox="0 0 40 40" aria-hidden="true">
          <circle cx="20" cy="20" r="9" />
          <path d="M20 4v8M20 28v8M4 20h8M28 20h8" />
        </svg>

        <div class="sp-icons">
          {#each spacePlaced as it (it.s.id)}
            <button
              class="sp-srv"
              class:sp-unread={it.s.unread.length > 0 || it.s.dot}
              class:sp-carried={it.carried}
              style={`left:${spaceVw / 2 + it.x}px; top:${spaceVh / 2 + it.y}px; --sp-s:${it.scale.toFixed(3)};${spaceAccents[it.s.id] ? ` --sp-a:${spaceAccents[it.s.id]};` : ""}`}
              data-name={it.s.name}
              onclick={() => spaceIconClick(it.s.id)}
              use:contextMenu={() => spaceServerMenu(it.s)}
            >
              {#if serverIcons[it.s.id] && appearance.icons !== "flat"}
                <img class="rail-img" src={imgSrc(serverIcons[it.s.id])} alt="" />
              {:else}
                {monogram(it.s.name)}
              {/if}
              {#if it.s.unread.length}
                <span class="rail-badge">{it.s.unread.length}</span>
              {/if}
            </button>
          {/each}
        </div>

        {#if spaceLasso}
          <div class="sp-lasso" style={`left:${spaceVw / 2 + spaceLasso.x}px; top:${spaceVh / 2 + spaceLasso.y}px; width:${spaceLasso.r * 2}px; height:${spaceLasso.r * 2}px`}></div>
        {/if}

        <div class="sp-hud">
          <div class="sp-hud-line">orbit · {spaceBackdropEff === "custom" ? "custom" : (SPACE_BACKDROP_TILES.find((b) => b.id === spaceBackdropEff)?.name ?? spaceBackdropEff)}</div>
          <div class="sp-hud-sub">yaw {Math.round(spaceCam.yaw)}° · pitch {Math.round(spaceCam.pitch)}°</div>
          {#if spaceCarried}
            <div class="sp-hud-carry">carrying {Object.keys(spaceCarried).length} · click to drop · esc cancels</div>
          {/if}
        </div>

        <div class="sp-keys">
          <span class="sp-key"><b>[drag]</b> look</span>
          <span class="sp-key"><b>[hold]</b> lasso</span>
          <button class="sp-key sp-key-btn" class:active={spaceTray} onclick={() => (spaceTrayPinned = !spaceTrayPinned)}><b>[t]</b> tray</button>
          <button class="sp-key sp-key-btn" onclick={toggleSpace}><b>[esc]</b> exit</button>
        </div>

        {#if spaceTray}
          <div class="sp-tray">
            <div class="sp-tray-head">
              <span class="sp-micro">server tray</span>
              <span class="sp-chip">unplaced · {spaceUnplaced.length}</span>
              <span class="sp-tray-hint">tap a server: it flies to where you aim</span>
            </div>
            {#if spaceUnplaced.length}
              <div class="sp-tray-row">
                {#each spaceUnplaced as s (s.id)}
                  <button class="sp-tray-item" onclick={() => placeFromTray(s.id)}>
                    <span class="sp-disc" style={spaceAccents[s.id] ? `--sp-a:${spaceAccents[s.id]}` : ""}>
                      {#if serverIcons[s.id] && appearance.icons !== "flat"}
                        <img class="rail-img" src={imgSrc(serverIcons[s.id])} alt="" />
                      {:else}
                        {monogram(s.name)}
                      {/if}
                    </span>
                    <span class="sp-tray-name">{s.name}</span>
                  </button>
                {/each}
              </div>
            {:else}
              <p class="sp-tray-empty">Every server is placed. Hold anywhere to lasso and rearrange.</p>
            {/if}
          </div>
        {/if}
      </div>
    {/if}

    {#if showSettings}
      <div class="stx" role="dialog" aria-label="Settings">
        <div class="stx-nav-zone">
          <nav class="stx-nav">
            <label class="stx-search">
              <input bind:value={setSearch} placeholder="Search settings" />
            </label>
            {#each ["Account", "App", "Connection"] as cat (cat)}
              {@const pages = filterPages(USER_SET_PAGES, setSearch).filter((p) => p.cat === cat)}
              {#if pages.length}
                <div class="stx-cat">{cat}</div>
                {#each pages as p (p.id)}
                  <button type="button" class="stx-item" class:active={settingsPage === p.id} onclick={() => (settingsPage = p.id)}>{p.label}</button>
                {/each}
              {/if}
            {/each}
            <div class="stx-foot">
              <span>MEWTUAL v{APP_VERSION}</span>
              {#if myFp}<span>FP {fmtFp(myFp.slice(0, 16).toUpperCase())}</span>{/if}
              <span>[CTRL+L] LOCK SESSION</span>
            </div>
          </nav>
        </div>
        <div class="stx-content-zone">
          <div class="stx-content">
            {#if settingsPage === "profile"}
              <div class="stx-crumb">SETTINGS // ACCOUNT // MY PROFILE</div>
              <h1>My Profile</h1>
              {#if cur && !cur.isDm}
                <p class="muted small">Editing your identity on <strong>{cur.name}</strong>: profiles are per server, by design. Other servers never see this one.</p>
              {:else}
                <p class="muted small">Open a server to edit the profile you use there: profiles are per server, by design.</p>
              {/if}
              {@render profileEditor()}
            {:else if settingsPage === "devices"}
              <div class="stx-crumb">SETTINGS // ACCOUNT // DEVICES</div>
              <h1>Devices</h1>
              <section class="set-section">
                <p class="muted small">
                  Link another device to your identity. The new device gets its own key: nothing
                  is copied: and nothing at all happens until you approve it here on this device.
                </p>
                <button class="ghost" onclick={() => (showLinkDevice = true)}>⛓ Link a new device…</button>
              </section>
              <section class="set-section">
                <p class="muted small">Your linked devices are listed per server (each server sees its own identity): find them under a server's Settings → Devices.</p>
              </section>
            {:else if settingsPage === "vault"}
              <div class="stx-crumb">SETTINGS // ACCOUNT // VAULT &amp; LOCK</div>
              <h1>Vault &amp; Lock</h1>
              <section class="set-section">
                <p class="muted small">
                  Everything you are lives in an encrypted vault on this machine, sealed by the
                  secret you chose at setup (passphrase, spell, or melody). Locking clears the
                  screen and asks for it again; the node stays online underneath.
                </p>
                <button class="ghost" onclick={lockScreen}>Lock now [Ctrl+L]</button>
              </section>
              <section class="set-section">
                <p class="muted small">Changing the secret re-seals the vault, which is not wired up yet: it is on the list.</p>
              </section>
            {:else if settingsPage === "verify"}
              <div class="stx-crumb">SETTINGS // ACCOUNT // VERIFICATION</div>
              <h1>Verification</h1>
              {#if myFp}
                <section class="set-section">
                  <h3>Your fingerprint on {cur?.name ?? "this server"}</h3>
                  <p class="stx-fp-big">{fmtFp(myFp.toUpperCase())}</p>
                  <div class="invite-actions">
                    <button class="ghost small" onclick={() => navigator.clipboard?.writeText(myFp)}>Copy fingerprint</button>
                  </div>
                  <p class="muted small">Read it to someone over a call, or compare in person. If theirs matches what their profile claims, mark them verified from their context menu.</p>
                </section>
                <section class="set-section">
                  <h3>People you have verified here</h3>
                  {#if roster.some((m) => !m.you && verifiedFps.has(m.fingerprint))}
                    <ul class="role-list">
                      {#each roster.filter((m) => !m.you && verifiedFps.has(m.fingerprint)) as m (m.fingerprint)}
                        <li>
                          {@render avatarTag(m.fingerprint)}
                          {@render nameTag(m.fingerprint)}
                          <span class="vf-check">✓</span>
                          <button class="ghost small" onclick={() => (verifyFor = m.fingerprint)}>Review…</button>
                        </li>
                      {/each}
                    </ul>
                  {:else}
                    <p class="muted small">Nobody yet. Verified marks are local to you and this server: nobody is told.</p>
                  {/if}
                </section>
              {:else}
                <p class="muted small">Open a server first: fingerprints are per-server identities.</p>
              {/if}
            {:else if settingsPage === "appearance"}
              <div class="stx-crumb">SETTINGS // APP // APPEARANCE</div>
              <h1>Appearance</h1>
              <section class="set-section">
                <h3>Theme</h3>
                <p class="muted small">
                  Presets restyle the whole app. Whatever you pick, colours keep their jobs:
                  green = presence, gold = mentions, red = danger.
                </p>
                <div class="preset-row">
                  {#each PRESETS as p (p.id)}
                    <button
                      type="button"
                      class="preset-btn"
                      class:active={appearance.preset === p.id}
                      onclick={() => (appearance = { ...appearance, preset: p.id, accent: "" })}
                    >
                      <span class="preset-sw" style={`background:${p.sw}`}></span>{p.name}
                    </button>
                  {/each}
                </div>
                <div class="field" style="margin-top:8px">
                  <span class="muted small">Accent override: keep the preset's mood, swap the highlight colour</span>
                  <div class="accent-row">
                    {#each ACCENT_CHOICES as a (a)}
                      <button
                        type="button"
                        class="accent-sw"
                        class:active={appearance.accent === a}
                        style={`background:${a}`}
                        aria-label={`Accent colour ${a}`}
                        title={a}
                        onclick={() => (appearance = { ...appearance, accent: appearance.accent === a ? "" : a })}
                      ></button>
                    {/each}
                    <input
                      type="color"
                      class="accent-custom"
                      title="Custom accent colour"
                      aria-label="Custom accent colour"
                      value={appearance.accent || "#977df2"}
                      oninput={(e) => (appearance = { ...appearance, accent: e.currentTarget.value })}
                    />
                  </div>
                </div>
              </section>
              <section class="set-section">
                <h3>Text</h3>
                <div class="stx-duo">
                  <label class="field">
                    <span class="muted small">Chat text size: {appearance.scale || 100}%</span>
                    <input
                      type="range"
                      min="70"
                      max="140"
                      step="2"
                      value={appearance.scale || 100}
                      oninput={(e) => (appearance = { ...appearance, scale: +e.currentTarget.value })}
                    />
                  </label>
                  <div class="field">
                    <span class="muted small">Timestamps</span>
                    <div class="stx-seg">
                      {#each [["", "AUTO"], ["12", "12H"], ["24", "24H"]] as [id, lbl] (id)}
                        <button type="button" class:on={appearance.clock === id} onclick={() => (appearance = { ...appearance, clock: id })}>{lbl}</button>
                      {/each}
                    </div>
                  </div>
                </div>
              </section>
              {#if liveryActive && activeServerId !== null && !cur?.isDm}
                <section class="set-section">
                  <h3>Livery</h3>
                  <label class="toggle">
                    <input
                      type="checkbox"
                      checked={!liveryOptOut}
                      onchange={() => setLiveryOptOut(!liveryOptOut)}
                    />
                    <span>
                      Follow this server's livery{livery.preset
                        ? ` (${PRESETS.find((p) => p.id === livery.preset)?.name ?? livery.preset})`
                        : ""}
                    </span>
                  </label>
                  <p class="muted small">Opting out is yours alone; nobody is told.</p>
                </section>
              {/if}
              <section class="set-section">
                <h3>Interface</h3>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.density === "compact"}
                    onchange={() => (appearance = { ...appearance, density: appearance.density === "compact" ? "" : "compact" })}
                  />
                  <span>Compact density: tighter rows, smaller text, more on screen</span>
                </label>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.chrome !== "clean"}
                    onchange={() => (appearance = { ...appearance, chrome: appearance.chrome === "clean" ? "terminal" : "clean" })}
                  />
                  <span>Terminal chrome: scanlines &amp; glow on the frame</span>
                </label>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.motion !== "off"}
                    onchange={() => (appearance = { ...appearance, motion: appearance.motion === "off" ? "" : "off" })}
                  />
                  <span>Hover motion: icons lift and turn under the pointer</span>
                </label>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.flat}
                    onchange={() => (appearance = { ...appearance, flat: !appearance.flat })}
                  />
                  <span>Flatten messages: ignore other members' custom bubble backgrounds</span>
                </label>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.icons === "flat"}
                    onchange={() => (appearance = { ...appearance, icons: appearance.icons === "flat" ? "" : "flat" })}
                  />
                  <span>Flat server icons: monograms instead of uploaded images</span>
                </label>
              </section>
            {:else if settingsPage === "space"}
              <div class="stx-crumb">SETTINGS // APP // SERVER SPACE</div>
              <h1>Server Space</h1>
              <section class="set-section">
                <p class="muted small">The 360° room your servers hang in (Ctrl+O). Backdrop:</p>
                <div class="space-set-row">
                  {#each SPACE_BACKDROP_TILES as b (b.id)}
                    <button type="button" class="ghost small" class:active={spaceState.backdrop === b.id} onclick={() => setSpaceBackdrop(b.id)}>{b.name}</button>
                  {/each}
                  {#if spaceState.custom}
                    <button type="button" class="ghost small" class:active={spaceState.backdrop === "custom"} onclick={() => setSpaceBackdrop("custom")}>Custom</button>
                  {/if}
                  <label class="ghost small space-file">
                    {spaceState.custom ? "Replace image…" : "Custom image…"}
                    <input type="file" accept="image/*" onchange={(e) => loadSpacePano(e.currentTarget.files)} />
                  </label>
                  <button type="button" class="ghost small" onclick={() => { spaceState.placements = {}; saveSpace(); }}>Forget placements</button>
                </div>
                <p class="muted small">A custom backdrop is one equirectangular (2:1) image. Backdrop and placements stay on this device, like desktop icon positions.</p>
              </section>
            {:else if settingsPage === "notifications"}
              <div class="stx-crumb">SETTINGS // APP // NOTIFICATIONS</div>
              <h1>Notifications</h1>
              <section class="set-section">
                <label class="toggle">
                  <input type="checkbox" checked={soundOn} onchange={toggleSound} />
                  <span>Play a sound for new messages</span>
                </label>
                <button class="ghost small" onclick={playNotify} disabled={!soundOn}>Test sound</button>
              </section>
              <section class="set-section">
                <p class="muted small">Voice-call notifications are per server: each server's Settings → Overview has the toggle.</p>
              </section>
            {:else if settingsPage === "voice"}
              <div class="stx-crumb">SETTINGS // APP // VOICE &amp; CALLS</div>
              <h1>Voice &amp; Calls</h1>
              <section class="set-section">
                <h3>Devices</h3>
                <p class="muted small">Microphone and output pickers live on the call stage (they swap live, mid-call) and are remembered here between calls.</p>
              </section>
              <section class="set-section">
                <h3>NAT traversal</h3>
                <p class="muted small">
                  Voice is peer-to-peer + end-to-end encrypted. To connect across networks, peers use a
                  <strong>STUN</strong> server to find their public address; <strong>TURN</strong> relays the
                  (still encrypted) audio when a direct path can't be made. Blank STUN for LAN-only.
                </p>
                <label class="field">
                  <span class="muted small">STUN server(s): space/comma separated</span>
                  <input bind:value={callStun} placeholder="stun:stun.l.google.com:19302" />
                </label>
                <label class="field">
                  <span class="muted small">TURN server (optional, for strict NATs)</span>
                  <input bind:value={callTurn} placeholder="turn:your-host:3478" />
                </label>
                <div class="field row">
                  <label class="field" style="flex:1">
                    <span class="muted small">TURN user</span>
                    <input bind:value={callTurnUser} />
                  </label>
                  <label class="field" style="flex:1">
                    <span class="muted small">TURN credential</span>
                    <input type="password" bind:value={callTurnCred} />
                  </label>
                </div>
                <button class="ghost small" onclick={saveCallSettings}>Save call settings</button>
              </section>
            {:else if settingsPage === "chatmedia"}
              <div class="stx-crumb">SETTINGS // APP // CHAT &amp; MEDIA</div>
              <h1>Chat &amp; Media</h1>
              <section class="set-section">
                <h3>Message formatting</h3>
                <p class="muted small">Type these in any message. Bold/italic also have Ctrl+B / Ctrl+I.</p>
                <ul class="format-help">
                  <li><code>**bold**</code> → <strong>bold</strong></li>
                  <li><code>*italic*</code> → <em>italic</em></li>
                  <li><code>~~strike~~</code> → <s>strike</s></li>
                  <li><code>`code`</code> → <code>code</code> (inline) · <code>```</code> for a block</li>
                  <li><code>||spoiler||</code> → a blacked-out spoiler you click to reveal</li>
                  <li><code>&gt; quote</code> → a block quote</li>
                  <li><code>@</code> then a name → mention a member (notifies them)</li>
                  <li><code>:name:</code> → a custom emoji (added in a server's Settings → Emoji), or use the 😀 picker</li>
                  <li><code>[[Page]]</code> → link to a wiki page</li>
                  <li><code>- item</code> / <code>1. item</code> → bullet / numbered lists</li>
                </ul>
              </section>
            {:else if settingsPage === "keybinds"}
              <div class="stx-crumb">SETTINGS // APP // KEYBINDS</div>
              <h1>Keybinds</h1>
              <section class="set-section">
                <ul class="stx-keys">
                  <li><kbd>Ctrl+K</kbd><span>Quick switcher: channels, surfaces, servers, DMs</span></li>
                  <li><kbd>Ctrl+1…7</kbd><span>Surfaces: chat, files, status, wiki, profile, downloads, events</span></li>
                  <li><kbd>Ctrl+B / Ctrl+I</kbd><span>Bold / italic in the composer</span></li>
                  <li><kbd>Ctrl+Shift+F</kbd><span>Search with the filter panel open</span></li>
                  <li><kbd>Ctrl+L</kbd><span>Lock the session</span></li>
                  <li><kbd>Ctrl+O</kbd><span>The 360° server space</span></li>
                  <li><kbd>Alt+← / →</kbd><span>Back / forward through where you have been</span></li>
                  <li><kbd>T</kbd> <span>(held, in the space) the tray of unplaced servers</span></li>
                  <li><kbd>Z / X</kbd><span>Piano octave down / up (lock screen and instrument drawer)</span></li>
                  <li><kbd>Esc</kbd><span>Close the topmost thing, one layer at a time</span></li>
                </ul>
                <p class="muted small">Remapping is not wired up yet: it is on the list.</p>
              </section>
            {:else if settingsPage === "network"}
              <div class="stx-crumb">SETTINGS // CONNECTION // NETWORK</div>
              <h1>Network</h1>
              <section class="set-section">
                <p class="muted small">Reachability (LAN address / relay) is chosen when you found a server.</p>
                <label class="field">
                  <span class="muted small">
                    Default rendezvous address: pre-filled when you found a server, so people can
                    join with just the invite (no address needed). Pasting a joiner invite that names
                    a rendezvous is discovered automatically.
                  </span>
                  <input bind:value={rendezvous} placeholder="/ip4/…/tcp/…/p2p/… (optional)" />
                </label>
              </section>
            {:else if settingsPage === "updates"}
              <div class="stx-crumb">SETTINGS // CONNECTION // UPDATES</div>
              <h1>Updates</h1>
              <section class="set-section">
                <p class="muted small">
                  Mewtual looks for a new release on launch and offers it once. It never installs
                  anything on its own, and a skipped version stays skipped: check here to get it back.
                  Only official builds are wired to the release feed: a copy you built yourself
                  updates the way you built it.
                </p>
                <div class="invite-actions">
                  <button class="ghost" disabled={updateBusy} onclick={() => checkForUpdate(true)}>Check for updates</button>
                  <span class="muted small">Current version: {APP_VERSION}</span>
                </div>
              </section>
            {/if}
          </div>
          {#if settingsPage === "appearance"}
            <aside class="stx-prev">
              <div class="stx-ph"><i></i>LIVE PREVIEW</div>
              <div class="stx-pcard">
                <div class="stx-pcap">CHROME</div>
                <div class="stx-mini">
                  <div class="stx-mini-rail"><i class="on"></i><i></i><i></i></div>
                  <div class="stx-mini-side"><i class="on" style="width:90%"></i><i style="width:70%"></i><i style="width:80%"></i><i style="width:55%"></i></div>
                  <div class="stx-mini-chat"><i class="nm"></i><i style="width:80%"></i><i style="width:60%"></i></div>
                </div>
              </div>
              <div class="stx-pcard">
                <div class="stx-pcap">MESSAGE</div>
                {@render previewLog()}
              </div>
              <div class="stx-pcard">
                <div class="stx-pcap">CONTROLS</div>
                <div class="stx-pctl">
                  <button class="primary small" type="button">Send</button>
                  <button class="ghost small" type="button">Cancel</button>
                  <span class="stx-pdot"></span>
                </div>
              </div>
              <p class="muted small stx-pnote">Updates as you tweak: theme, accent, text size, clock.</p>
            </aside>
          {:else if settingsPage === "profile"}
            <aside class="stx-prev">
              <div class="stx-ph"><i></i>LIVE PREVIEW</div>
              <div class="stx-pcard">
                <div class="stx-pcap">PROFILE CARD</div>
                <div class="stx-prof">
                  {#if pBanner}
                    <img class="stx-pbanner" src={imgSrc(pBanner)} alt="" />
                  {:else}
                    <div class="stx-pbanner"></div>
                  {/if}
                  {#if pAvatar}
                    <img class="avatar lg stx-pav" src={imgSrc(pAvatar)} alt="" />
                  {:else}
                    <span class="avatar lg fallback stx-pav" style={`background:${pColor}`}>{(pName || displayName).slice(0, 1).toUpperCase()}</span>
                  {/if}
                  <div class="stx-prof-body">
                    <span class="stx-prof-name">{@render styledName(pName || displayName, pColor, pFont, pEffect)}</span>
                    {#if myFp}<div class="stx-pfp">FP {fmtFp(myFp.slice(0, 16).toUpperCase())}</div>{/if}
                  </div>
                </div>
              </div>
              <div class="stx-pcard">
                <div class="stx-pcap">IN CHAT</div>
                {@render previewLog()}
              </div>
              <p class="muted small stx-pnote">Exactly what members of this server see, card and chat both.</p>
            </aside>
          {/if}
          <button type="button" class="stx-esc" onclick={() => (showSettings = false)} title="Close (Esc)">
            <span class="stx-esc-ring">✕</span>
            <span>ESC</span>
          </button>
        </div>
      </div>
    {/if}

    {#if showServerSettings}
      <div class="stx" role="dialog" aria-label="Server settings">
        <div class="stx-nav-zone">
          <nav class="stx-nav">
            <div class="stx-srv-head">
              {#if activeServerId !== null && serverIcons[activeServerId] && appearance.icons !== "flat"}
                <img class="avatar" src={imgSrc(serverIcons[activeServerId])} alt="" />
              {:else}
                <span class="avatar fallback">{monogram(cur?.name ?? "")}</span>
              {/if}
              <span class="srv-meta"><b>{cur?.name ?? ""}</b> <span class="role-badge {myRole}">{myRole}</span></span>
            </div>
            <label class="stx-search">
              <input bind:value={setSearch} placeholder="Search settings" />
            </label>
            {#each ["Overview", "People", "Content", "Voice", "Danger"] as cat (cat)}
              {@const pages = filterPages(SRV_SET_PAGES, setSearch).filter((p) => p.cat === cat)}
              {#if pages.length}
                <div class="stx-cat">{cat}</div>
                {#each pages as p (p.id)}
                  <button type="button" class="stx-item" class:active={serverSettingsPage === p.id} class:danger={p.danger} onclick={() => (serverSettingsPage = p.id)}>{p.label}</button>
                {/each}
              {/if}
            {/each}
            <div class="stx-foot">
              <span>{roster.length} MEMBERS</span>
              {#if canModerate}<span>YOU CAN MODERATE</span>{/if}
            </div>
          </nav>
        </div>
        <div class="stx-content-zone">
          <div class="stx-content">
            {#if serverSettingsPage === "overview"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // OVERVIEW</div>
              <h1>Overview</h1>
              <section class="set-section">
              <p>{cur?.name ?? ":"} <span class="role-badge {myRole}">{myRole}</span></p>
              <form class="rename-row" onsubmit={(e) => { e.preventDefault(); renameServer(); }}>
                <input bind:value={serverNameDraft} placeholder="Server name" />
                <button class="ghost small" disabled={!serverNameDraft.trim() || serverNameDraft.trim() === cur?.name}>Rename</button>
              </form>
              <p class="muted small">The name is your own label for this server (not shared with other members).</p>
              <label class="toggle">
                <input type="checkbox" checked={acceptCallsHere} onchange={toggleAcceptCalls} />
                <span>Notify me of voice calls on this server</span>
              </label>
              </section>
            {:else if serverSettingsPage === "calls"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // CALLS &amp; RELAY</div>
              <h1>Calls &amp; Relay</h1>
              <section class="set-section">
              <div class="field">
                <span class="muted">Shared voice relay (TURN)</span>
                <p class="muted small">Optional. A TURN server for voice calls that can't hole-punch (symmetric NAT).
                  It's folded into invites, so people you invite inherit it: set it once for everyone.
                  Media stays end-to-end encrypted; the relay only forwards ciphertext.</p>
                <input bind:value={srvTurn} onchange={saveSrvTurn} placeholder="turn:your-host:3478" />
                <div class="turn-creds">
                  <label><span class="muted small">Username</span><input bind:value={srvTurnUser} onchange={saveSrvTurn} /></label>
                  <label><span class="muted small">Credential</span><input type="password" bind:value={srvTurnCred} onchange={saveSrvTurn} /></label>
                </div>
              </div>
              </section>
            {:else if serverSettingsPage === "livery"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // LIVERY</div>
              <h1>Livery</h1>
              <section class="set-section">
              {#if canModerate}
                <p class="muted small">
                  Publish a colour scheme members see while they're on this server. Anyone can opt
                  out in their own Appearance settings, and green/gold/red keep their meanings
                  (presence / mentions / danger) under any livery.
                </p>
                <div class="field">
                  <span class="muted small">Server icon: shown on everyone's rail (they can prefer monograms)</span>
                  <div class="avatar-row">
                    {#if livery.icon}
                      <img class="avatar lg" src={imgSrc(livery.icon)} alt="" />
                    {/if}
                    <label class="upload-btn">
                      {livery.icon ? "Replace icon" : "Upload icon"}
                      <input type="file" accept="image/png,image/jpeg,image/webp" onchange={(e) => loadServerIcon(e.currentTarget.files)} />
                    </label>
                    {#if livery.icon}
                      <button class="ghost small" disabled={busy} onclick={() => setServerIcon("")}>Remove icon</button>
                    {/if}
                  </div>
                </div>
                <div class="preset-row">
                  {#each PRESETS as p (p.id)}
                    <button
                      type="button"
                      class="preset-btn"
                      class:active={liveryDraft.preset === p.id}
                      onclick={() => (liveryDraft = { ...liveryDraft, preset: p.id })}
                    >
                      <span class="preset-sw" style={`background:${p.sw}`}></span>{p.name}
                    </button>
                  {/each}
                </div>
                <div class="field" style="margin-top:8px">
                  <span class="muted small">Accent (optional)</span>
                  <div class="accent-row">
                    {#each ACCENT_CHOICES as a (a)}
                      <button
                        type="button"
                        class="accent-sw"
                        class:active={liveryDraft.accent === a}
                        style={`background:${a}`}
                        aria-label={`Accent colour ${a}`}
                        title={a}
                        onclick={() => (liveryDraft = { ...liveryDraft, accent: liveryDraft.accent === a ? "" : a })}
                      ></button>
                    {/each}
                    <input
                      type="color"
                      class="accent-custom"
                      title="Custom accent colour"
                      aria-label="Custom accent colour"
                      value={liveryDraft.accent || "#977df2"}
                      oninput={(e) => (liveryDraft = { ...liveryDraft, accent: e.currentTarget.value })}
                    />
                  </div>
                </div>
                <div class="field" style="margin-top:8px">
                  <span class="muted small">Ground tint: wash the room in your own colours, background and sidebars separately.</span>
                  <div class="grad-maker">
                    <span class="muted small tint-lbl">Background</span>
                    <input type="color" value={liveryTintBgC} aria-label="Background tint colour" oninput={(e) => { liveryTintBgC = e.currentTarget.value; if (draftTinted) applyTint(); }} />
                    <input type="range" min="0" max="60" step="2" value={liveryTintBgS} aria-label="Background tint intensity" oninput={(e) => { liveryTintBgS = +e.currentTarget.value; if (draftTinted) applyTint(); }} />
                    <span class="muted small">{liveryTintBgS}%</span>
                  </div>
                  <div class="grad-maker">
                    <span class="muted small tint-lbl">Sidebars</span>
                    <input type="color" value={liveryTintSideC} aria-label="Sidebar tint colour" oninput={(e) => { liveryTintSideC = e.currentTarget.value; if (draftTinted) applyTint(); }} />
                    <input type="range" min="0" max="60" step="2" value={liveryTintSideS} aria-label="Sidebar tint intensity" oninput={(e) => { liveryTintSideS = +e.currentTarget.value; if (draftTinted) applyTint(); }} />
                    <span class="muted small">{liveryTintSideS}%</span>
                  </div>
                  <div class="grad-maker">
                    {#if draftTinted}
                      <button type="button" class="ghost small" onclick={clearTint}>Clear tint</button>
                    {:else}
                      <button type="button" class="ghost small" onclick={applyTint}>Apply tint</button>
                    {/if}
                  </div>
                  <span class="muted small">Intensity is how far the colour sinks in. A tint mixes into the default dark grounds and stands in for the preset's own, so the preview on the right is the truth: text tokens stay untouched, and green, gold and red keep their jobs.</span>
                </div>
                <div class="field" style="margin-top:8px">
                  <span class="muted small">Corners</span>
                  <div class="cat-row">
                    {#each Object.keys(LIVERY_RADIUS) as rid (rid)}
                      <button
                        type="button"
                        class="preset-btn cat-tile"
                        class:active={(liveryDraft.tokens["radius"] ?? "soft") === rid}
                        onclick={() => setDraftToken("radius", rid === "soft" ? "" : rid)}
                      >{rid}</button>
                    {/each}
                  </div>
                </div>
                <div class="field">
                  <span class="muted small">Interface font</span>
                  <div class="cat-row">
                    {#each Object.keys(LIVERY_FONTS) as fid (fid)}
                      <button
                        type="button"
                        class="preset-btn cat-tile"
                        class:active={(liveryDraft.tokens["font"] ?? "system") === fid}
                        style={`font-family:${LIVERY_FONTS[fid]}`}
                        onclick={() => setDraftToken("font", fid === "system" ? "" : fid)}
                      >{fid}</button>
                    {/each}
                  </div>
                </div>
                <div class="field">
                  <span class="muted small">Background pattern</span>
                  <div class="cat-row">
                    {#each LIVERY_PATTERNS as pid (pid)}
                      <button
                        type="button"
                        class="preset-btn cat-tile pat-{pid}"
                        class:active={(liveryDraft.tokens["pattern"] ?? "none") === pid}
                        onclick={() => setDraftToken("pattern", pid === "none" ? "" : pid)}
                      >{pid}</button>
                    {/each}
                  </div>
                </div>
                <div class="field">
                  <span class="muted small">Custom cursor: a small image members' pointers become here (they can opt out of the whole livery)</span>
                  <div class="avatar-row">
                    {#if livery.cursor}
                      <img class="cursor-preview" src={"data:image/png;base64," + livery.cursor} alt="" />
                    {/if}
                    <label class="upload-btn">
                      {livery.cursor ? "Replace cursor" : "Upload cursor"}
                      <input type="file" accept="image/png,image/gif,image/webp" onchange={(e) => loadServerCursor(e.currentTarget.files)} />
                    </label>
                    {#if livery.cursor}
                      <button class="ghost small" disabled={busy} onclick={() => setServerCursor("")}>Remove cursor</button>
                    {/if}
                  </div>
                </div>
                <div class="invite-actions">
                  <button class="ghost small" disabled={busy} onclick={publishLivery}>Publish livery</button>
                  {#if liveryActive}
                    <button class="ghost small danger-btn" disabled={busy} onclick={removeLivery}>Remove livery</button>
                  {/if}
                </div>
              {:else}
                <p class="muted small">
                  {liveryActive
                    ? "This server publishes a livery. You can opt out in Settings → Appearance."
                    : "No livery published. Owners and admins can set one."}
                </p>
              {/if}
              </section>
            {:else if serverSettingsPage === "members"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // MEMBERS</div>
              <h1>Members</h1>
              <section class="set-section">
              <ul class="role-list">
                {#each roster as m}
                  {@const r = roles[m.fingerprint] ?? "member"}
                  <li>
                    {@render avatarTag(m.fingerprint)}
                    {@render nameTag(m.fingerprint)}
                    <span class="role-badge {r}">{r}</span>
                    {#if badges[m.fingerprint]}
                      {@const b = badges[m.fingerprint]}
                      <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""}>{b.label}</span>
                    {/if}
                    {#if myRole === "owner" && !m.you && r !== "owner"}
                      {#if r === "admin"}
                        <button class="ghost small" onclick={() => setAdmin(m.fingerprint, false)}>Remove admin</button>
                      {:else}
                        <button class="ghost small" onclick={() => setAdmin(m.fingerprint, true)}>Make admin</button>
                      {/if}
                      {#if confirmRemoveFp === m.fingerprint}
                        <button class="ghost small danger-btn" onclick={() => removeMember(m.fingerprint)}>Confirm</button>
                      {:else}
                        <button class="ghost small danger-btn" onclick={() => (confirmRemoveFp = m.fingerprint)}>Remove</button>
                      {/if}
                    {/if}
                  </li>
                {/each}
              </ul>
              <p class="muted small">
                The owner is the founder (the MLS committer). Member removal is owner-only and
                protocol-enforced. Admins can invite newcomers: the owner serializes each
                admission, so it completes when the owner is next online: and a demotion is
                replay-proof (a removed admin can't re-grant itself).
              </p>
              {#if myRole !== "owner"}
                <p class="muted small">Only the owner can change roles.</p>
              {/if}
              </section>
            {:else if serverSettingsPage === "badges"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // BADGES</div>
              <h1>Badges</h1>
              <section class="set-section">
              <p class="muted small">Little chips admins pin next to a member's name. Role names are reserved: a badge can never impersonate one.</p>
              <ul class="role-list">
                {#each roster as m}
                  <li>
                    {@render avatarTag(m.fingerprint)}
                    {@render nameTag(m.fingerprint)}
                    {#if badges[m.fingerprint]}
                      {@const b = badges[m.fingerprint]}
                      <span class="cust-badge" style={b.color ? `--badge-c:${b.color}` : ""}>{b.label}</span>
                    {/if}
                    {#if canModerate}
                      <button class="ghost small" onclick={() => (badgeEditFp === m.fingerprint ? (badgeEditFp = "") : openBadgeEditor(m.fingerprint))}>
                        {badges[m.fingerprint] ? "Edit badge" : "Badge…"}
                      </button>
                    {/if}
                  </li>
                  {#if badgeEditFp === m.fingerprint && canModerate}
                    <li class="badge-editor">
                      <input
                        class="badge-label"
                        bind:value={badgeLabelDraft}
                        maxlength="24"
                        placeholder="Badge text (e.g. ARTIST): role names are taken"
                        onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); saveBadge(m.fingerprint, badgeLabelDraft, badgeColorDraft); } else if (e.key === "Escape") { e.preventDefault(); badgeEditFp = ""; } }}
                      />
                      <input type="color" class="accent-custom" title="Badge colour" aria-label="Badge colour" bind:value={badgeColorDraft} />
                      <button class="ghost small" disabled={!badgeLabelDraft.trim()} onclick={() => saveBadge(m.fingerprint, badgeLabelDraft, badgeColorDraft)}>Save</button>
                      {#if badges[m.fingerprint]}
                        <button class="ghost small danger-btn" onclick={() => saveBadge(m.fingerprint, "", "")}>Remove badge</button>
                      {/if}
                    </li>
                  {/if}
                {/each}
              </ul>
              </section>
            {:else if serverSettingsPage === "sdevices"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // DEVICES</div>
              <h1>Devices</h1>
              {#if myRole === "owner" && Object.keys(deviceMap).length}
                <section class="set-section">
                <h3><span>Linked devices</span></h3>
                <ul class="dev-panel">
                  {#each Object.entries(deviceMap) as [cfp, d] (cfp)}
                    <li title={cfp}>
                      {@render nameTag(d.origin)}
                      <span class="dev-tag">· {d.name}</span>
                      <span class="fp small">{cfp.slice(0, 8)}</span>
                      <span class="muted small">{onlineMembers.has(cfp) ? "online" : "offline"}</span>
                      {#if d.origin === myFp}
                        {#if confirmRevokeFp === cfp}
                          <button class="ghost small danger-btn" onclick={() => revokeDevice(cfp)}>Confirm revoke</button>
                        {:else}
                          <button class="ghost small danger-btn" onclick={() => (confirmRevokeFp = cfp)}>Revoke</button>
                        {/if}
                      {/if}
                    </li>
                  {/each}
                </ul>
                <p class="muted small">
                  Revoke removes one of your own linked devices for good: its access ends and the
                  same grant can't re-add it. Losing your original (founding) device means you can't
                  add or revoke devices here; recover by having the server owner re-admit you.
                </p>
                </section>
              {:else if myRole === "owner"}
                <p class="muted small">No linked devices yet: members pair extras from Settings → Devices on their own machines.</p>
              {:else}
                <p class="muted small">Only the owner sees the device roster. Pair your own extra device from Settings → Devices.</p>
              {/if}

            {:else if serverSettingsPage === "invites"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // INVITES</div>
              <h1>Invites</h1>
              {#if cur?.invite || canInvite}
              <section class="set-section">
                <p class="muted small">Single-use: share it with one person to join this server. Generate a fresh
                  one anytime (after a restart, or once the last one was used).</p>
                {#if myRole === "admin"}
                  <p class="muted small">As an admin, the newcomer is admitted once the owner is next online.</p>
                {/if}
                {#if cur?.invite}
                  <textarea class="invite-code" readonly rows="3" value={wrapInvite(cur.invite, cur.id)}></textarea>
                  {#if wrapInvite(cur.invite, cur.id).length <= QR_MAX_CHARS}
                    <canvas class="qr-canvas" use:qr={wrapInvite(cur.invite, cur.id)}></canvas>
                  {/if}
                  <div class="invite-actions">
                    <button onclick={copyInvite}>{copied ? "Copied!" : "Copy invite"}</button>
                    {#if canInvite}
                      <button class="ghost" disabled={mintingInvite} onclick={generateInvite}>
                        {mintingInvite ? "Generating…" : "Generate new invite"}
                      </button>
                    {/if}
                  </div>
                {:else if canInvite}
                  <button disabled={mintingInvite} onclick={generateInvite}>
                    {mintingInvite ? "Generating…" : "Generate an invite"}
                  </button>
                {/if}
              </section>
              {:else}
                <p class="muted small">Nothing to mint right now: invites appear here once you can create one.</p>
              {/if}
            {:else if serverSettingsPage === "emoji"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // EMOJI &amp; STICKERS</div>
              <h1>Emoji &amp; Stickers</h1>
              <section class="set-section">
              <p class="muted small">Type <code>:code:</code> in chat to use one. Shared with the whole server.</p>
              {#if Object.keys(emojiMap).length}
                <div class="emoji-grid manage">
                  {#each Object.keys(emojiMap) as code}
                    <span class="emoji-pick" title={":" + code + ":"}>
                      {#if emojiUrls[code]}<img src={emojiUrls[code]} alt={code} />{:else}<span class="muted">:{code}:</span>{/if}
                    </span>
                  {/each}
                </div>
              {/if}
              <form class="emoji-add" onsubmit={(e) => e.preventDefault()}>
                <input bind:value={newEmojiCode} placeholder="code (e.g. catjam)" />
                <select bind:value={newEmojiSize} title="Display size">
                  <option value={0}>Emoji</option>
                  <option value={48}>Medium</option>
                  <option value={96}>Large</option>
                  <option value={160}>Sticker</option>
                </select>
                <label class="upload-btn">
                  {uploading ? "…" : "Upload image"}
                  <input type="file" accept="image/*" disabled={uploading || !newEmojiCode.trim()}
                    onchange={(e) => { addEmoji(e.currentTarget.files); e.currentTarget.value = ''; }} />
                </label>
              </form>
              <p class="muted small">Size sets how big <code>:code:</code> renders in messages (reactions stay small).</p>
              </section>
            {:else if serverSettingsPage === "leave"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // LEAVE</div>
              <h1>Leave Server</h1>
              {#if activeServerId !== null}
                <section class="set-section danger">
                  <p class="muted small">Leaving forgets this server on this device. Coming back needs a fresh invite.</p>
                  <button class="ghost leave" onclick={() => { const id = activeServerId; showServerSettings = false; if (id !== null) leaveServer(id); }}>
                    Leave this server
                  </button>
                </section>
              {/if}
            {/if}
          </div>
          {#if serverSettingsPage === "livery"}
            {@const draftPattern = liveryDraft.tokens["pattern"]}
            <aside
              class="stx-prev"
              data-preset={liveryDraft.preset || null}
              data-livery-pattern={draftPattern && draftPattern !== "none" ? draftPattern : null}
              style={liveryDraftVars()}
            >
              <div class="stx-ph"><i></i>AS MEMBERS SEE IT</div>
              <div class="stx-pcard">
                <div class="stx-pcap">CHROME</div>
                <div class="stx-mini">
                  <div class="stx-mini-rail">
                    {#if livery.icon}<img class="mini-ico" src={imgSrc(livery.icon)} alt="" />{:else}<i class="on"></i>{/if}
                    <i></i><i></i>
                  </div>
                  <div class="stx-mini-side"><i class="on" style="width:90%"></i><i style="width:70%"></i><i style="width:80%"></i><i style="width:55%"></i></div>
                  <div class="stx-mini-chat"><i class="nm"></i><i style="width:80%"></i><i style="width:60%"></i></div>
                </div>
              </div>
              <div class="stx-pcard">
                <div class="stx-pcap">MESSAGE</div>
                {@render previewLog()}
              </div>
              <div class="stx-pcard">
                <div class="stx-pcap">CONTROLS</div>
                <div class="stx-pctl">
                  <button class="primary small" type="button">Send</button>
                  <button class="ghost small" type="button">Cancel</button>
                  <span class="stx-pdot"></span>
                </div>
              </div>
              <p class="muted small stx-pnote">Rendered with your draft before you publish. Anyone can opt out in their own Appearance.</p>
            </aside>
          {/if}
          <button type="button" class="stx-esc" onclick={() => (showServerSettings = false)} title="Close (Esc)">
            <span class="stx-esc-ring">✕</span>
            <span>ESC</span>
          </button>
        </div>
      </div>
    {/if}

    {#if lightbox}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="lightbox" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeLightbox(); }}>
        <div class="lightbox-bar">
          <span class="lightbox-name" title={lightboxFile?.name ?? lightbox.alt}>
            {lightboxFile?.name || lightbox.alt || "image"}
          </span>
          {#if lightboxFile}
            <span class="lightbox-size muted">{fmtSize(lightboxFile.size)}</span>
          {/if}
          <span class="lightbox-spacer"></span>
          <button class="ghost small" onclick={() => (lightboxZoom = !lightboxZoom)}>
            {lightboxZoom ? "Fit to window" : "Actual size"}
          </button>
          <button class="ghost small" onclick={() => lightbox && openFileRef(lightbox.cid)}>Properties</button>
          {#if lightboxFile}
            <button class="ghost small" onclick={() => lightboxFile && downloadFile(lightboxFile)}>Download</button>
          {/if}
          <span class="lightbox-div" aria-hidden="true"></span>
          <button class="ghost small" aria-label="Close" title="Close (Esc)" onclick={closeLightbox}>✕</button>
        </div>
        <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
        <div
          class="lightbox-stage"
          class:zoomed={lightboxZoom}
          role="presentation"
          onclick={(e) => { if (e.target === e.currentTarget) closeLightbox(); }}
        >
          <button
            class="lightbox-img"
            type="button"
            title={lightboxZoom ? "Click to fit to the window" : "Click to view at actual size"}
            onclick={() => (lightboxZoom = !lightboxZoom)}
          >
            <img src={lightbox.url} alt={lightbox.alt || lightboxFile?.name || "image"} />
          </button>
        </div>
      </div>
    {/if}

    {#if fileInfo}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeFileInfo(); }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2 class="file-info-title">📄 {fileInfo.name}</h2>
            <button class="ghost" onclick={closeFileInfo}>✕</button>
          </header>
          <div class="overlay-body file-info">
            {#if previewKind}
              <div class="file-preview">
                {#if fileInfoPreviewError}
                  <p class="muted small">Preview unavailable: the file isn't downloaded yet and no peer is sharing it right now.</p>
                {:else if !fileInfoPreview}
                  <p class="muted small">Loading preview…</p>
                {:else if previewKind === "image"}
                  <img src={fileInfoPreview} alt={fileInfo.name} />
                {:else if previewKind === "video"}
                  <!-- svelte-ignore a11y_media_has_caption -->
                  <video controls src={fileInfoPreview}></video>
                {:else if previewKind === "audio"}
                  <audio controls src={fileInfoPreview}></audio>
                {/if}
              </div>
            {/if}

            <dl class="file-meta">
              <dt>Availability</dt>
              <dd>
                {#if fileInfoAvail === null}
                  <span class="muted">checking…</span>
                {:else if fileInfoAvail}
                  <span class="avail yes">● Available on this device</span>
                {:else}
                  <span class="avail no">○ Not downloaded: fetched from a peer on demand</span>
                {/if}
              </dd>
              <dt>Uploaded by</dt>
              <dd>{nameOf(fileInfo.author)}</dd>
              <dt>Size</dt>
              <dd>{fmtSize(fileInfo.size)}</dd>
              <dt>Type</dt>
              <dd>{fileInfo.mime || "unknown"}</dd>
              <dt>Folder</dt>
              <dd>{fileInfo.path === "" ? "(root)" : fileInfo.path}</dd>
              <dt>Circulates until</dt>
              <dd>
                <span class="expiry {fileInfoExpiry.kind}">
                  {#if fileInfoExpiry.kind === "pinned"}📌{/if}{fileInfoExpiry.text}
                </span>
              </dd>
              <dt>Used in</dt>
              <dd>
                {#if fileInfoUsage === null}
                  <span class="muted">checking…</span>
                {:else if fileInfoUsage.wiki_pages.length === 0 && fileInfoUsage.status_count === 0 && fileInfoUsage.chat_count === 0 && fileInfoUsage.event_count === 0}
                  <span class="muted">nowhere yet</span>
                {:else}
                  <span class="usage-list">
                    {#each fileInfoUsage.wiki_pages as page (page)}
                      <button class="wikilink" onclick={() => openUsageWikiPage(page)}>{page}</button>
                    {/each}
                    {#if fileInfoUsage.chat_count > 0}
                      <span class="usage-count">{fileInfoUsage.chat_count} chat message{fileInfoUsage.chat_count === 1 ? "" : "s"}</span>
                    {/if}
                    {#if fileInfoUsage.status_count > 0}
                      <span class="usage-count">{fileInfoUsage.status_count} status post{fileInfoUsage.status_count === 1 ? "" : "s"}</span>
                    {/if}
                    {#if fileInfoUsage.event_count > 0}
                      <span class="usage-count">{fileInfoUsage.event_count} event{fileInfoUsage.event_count === 1 ? "" : "s"}</span>
                    {/if}
                  </span>
                {/if}
              </dd>
              <dt>Address</dt>
              <dd class="cid" title={fileInfo.cid}>{fileInfo.cid.slice(0, 16)}…</dd>
            </dl>
            <p class="muted small expiry-note">
              Expiry only stops the file being <strong>auto-circulated</strong> to members. Nothing is deleted from
              anyone's device, and the file stays fetchable by address for as long as any member still holds it.
            </p>

            {#if activeServerId !== null && downloads[dlKey(activeServerId, fileInfo.cid)] && (downloads[dlKey(activeServerId, fileInfo.cid)].status === "downloading" || downloads[dlKey(activeServerId, fileInfo.cid)].status === "queued")}
              {@const di = downloads[dlKey(activeServerId, fileInfo.cid)]}
              <label class="dl-progress">
                <span class="muted small">
                  {di.status === "queued" ? "Queued…" : `Downloading… ${Math.round(di.progress * 100)}%`}
                </span>
                <progress value={di.progress} max="1"></progress>
              </label>
            {/if}
            <div class="file-info-actions">
              <button class="primary" onclick={() => fileInfo && downloadFile(fileInfo)}>↓ Download</button>
              {#if canSetExpiry}
                <button class="ghost" disabled={fileInfoExpiryBusy} onclick={toggleKeepForever}>
                  {fileInfoExpiryBusy ? "Saving…" : keptForever ? "Restore 30-day expiry" : "♾ Keep forever"}
                </button>
              {/if}
              {#if myRole === "owner" || myRole === "admin"}
                {#if confirmDeleteCid === fileInfo.cid}
                  <button class="danger-btn" disabled={fileInfoBusy} onclick={() => fileInfo && removeFile(fileInfo)}>
                    {fileInfoBusy ? "Deleting…" : "Confirm delete"}
                  </button>
                  <button class="ghost" onclick={() => (confirmDeleteCid = "")}>Cancel</button>
                {:else}
                  <button class="ghost danger-text" onclick={() => fileInfo && (confirmDeleteCid = fileInfo.cid)}>Delete</button>
                {/if}
              {/if}
            </div>
            {#if myRole === "owner" || myRole === "admin"}
              <p class="muted small">Deleting unlists the file for everyone. Members who already downloaded it keep their copy.</p>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    {#if showFeedback}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) showFeedback = false; }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>💬 Send feedback</h2>
            <button class="ghost" onclick={() => (showFeedback = false)}>✕</button>
          </header>
          <div class="overlay-body feedback">
            <div class="seg fb-seg">
              <button class:active={feedbackKind === "bug"} onclick={() => (feedbackKind = "bug")}>🐞 Bug report</button>
              <button class:active={feedbackKind === "feature"} onclick={() => (feedbackKind = "feature")}>✨ Feature request</button>
            </div>
            <label class="fb-label" for="fb-title">Title</label>
            <input
              id="fb-title"
              class="fb-text"
              bind:value={feedbackTitle}
              maxlength="120"
              placeholder={feedbackKind === "bug" ? "Short summary of the problem" : "Short summary of the idea"}
            />
            <label class="fb-label" for="fb-text">
              {feedbackKind === "bug"
                ? "What went wrong? Steps to reproduce, and what you expected to happen."
                : "What would you like Mewtual to do?"}
            </label>
            <textarea id="fb-text" class="fb-text" bind:value={feedbackText} rows="7" placeholder="Describe it here…"></textarea>
            <p class="muted small">
              Filing opens a prefilled issue on the
              <strong>{feedbackKind === "bug" ? "bug tracker" : "feature request tracker"}</strong>
              in your browser: review it and press Submit there. Mewtual sends nothing on its own and holds no GitHub
              account of yours, so nothing is posted until you submit it. Your app version and environment are included
              to help debugging. No GitHub account? Copy the report and send it to the maintainer instead.
            </p>
            <div class="file-info-actions">
              <button class="primary" disabled={!feedbackText.trim()} onclick={openFeedbackIssue}>
                {feedbackOpened ? "✓ Opened in your browser" : feedbackKind === "bug" ? "🐞 File on GitHub" : "✨ File on GitHub"}
              </button>
              <button class="ghost" disabled={!feedbackText.trim()} onclick={copyFeedback}>
                {feedbackCopied ? "✓ Copied to clipboard" : "Copy report"}
              </button>
            </div>
          </div>
        </div>
      </div>
    {/if}

    {#if showWikiHelp}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) showWikiHelp = false; }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>Wiki formatting</h2>
            <button class="ghost" onclick={() => (showWikiHelp = false)}>✕</button>
          </header>
          <div class="overlay-body wiki-help">
            <p>Each page is written in <strong>Markdown</strong> or <strong>Wikitext</strong>: pick per page with the
              <code>md / wiki</code> switch in Edit mode. The choice is a page property shared with every member.
              Pages with 3+ headings get an automatic <strong>Contents</strong> box.</p>
            <h3>Link to another page (both formats)</h3>
            <p><code>[[Getting Started]]</code>, or with display text: <code>[[Getting Started|the guide]]</code>.
              Click a link to open it; a <span class="wikilink missing">red link</span> means the page doesn't exist
              yet: click it to create it.</p>
            <h3>Embed an image / video / audio (both formats)</h3>
            <p>In Edit mode, <strong>drag a file onto the editor</strong> or use the 📎 button. It's stored in the
              fileshare under <code>wiki/&lt;page&gt;/</code> and shown inline.</p>
            <h3>Infobox (both formats)</h3>
            <p>The summary card that floats at the top right of a page. Write one block, anywhere on the
              page, with the <code>▤</code> toolbar button; <code>title</code>, <code>image</code> and
              <code>caption</code> are the card's own chrome, every other line is a row, and a line with an
              empty value becomes a section band. One infobox per page.</p>
            <pre class="wiki-help-block">{`{{Infobox
| title   = Whiskers
| image   = (use 📎 or + insert to place a file here)
| caption = At the cafe
| Species = Cat
| Owner   = [[Alice]]
| Details =
| Age     = 4
}}`}</pre>
            <h3>Markdown pages</h3>
            <ul>
              <li><code>**bold**</code>, <code>*italic*</code>, <code>`code`</code></li>
              <li><code># Heading</code>, <code>## Subheading</code></li>
              <li><code>- bullet</code> lists, <code>1. numbered</code> lists</li>
              <li><code>&gt; quote</code>, <code>---</code> divider, <code>[text](https://link)</code></li>
            </ul>
            <h3>Wikitext pages</h3>
            <ul>
              <li><code>'''bold'''</code>, <code>''italic''</code>, <code>'''''both'''''</code></li>
              <li><code>== Heading ==</code>, <code>=== Subheading ===</code></li>
              <li><code>* bullet</code> / <code># numbered</code> lists; nest by repeating (<code>**</code>, <code>##</code>)</li>
              <li><code>; term : definition</code>, <code>:</code> indent, <code>----</code> divider</li>
              <li><code>[https://link label]</code> external link</li>
              <li><code>{"{|"}</code> … <code>{"|}"}</code> table, with <code>|-</code> rows, <code>!</code> header cells, <code>|+</code> caption</li>
              <li><code>&lt;nowiki&gt;…&lt;/nowiki&gt;</code> shows markup literally</li>
            </ul>
            <h3>Page tools</h3>
            <ul>
              <li><strong>Contents box</strong>: automatic at 3+ headings; force with <code>__TOC__</code>, suppress with <code>__NOTOC__</code>.</li>
              <li><strong>Sections</strong>: hover a heading in Read mode for a per-section <em>edit</em> jump.</li>
              <li><strong>Redirects</strong>: a page whose first line is <code>#REDIRECT [[Target]]</code> forwards readers there.</li>
              <li><strong>Rename / delete</strong>: in the page header (rename doesn't rewrite links: old links go red).</li>
              <li><strong>What links here</strong>: pages linking to the open page, listed under it.</li>
            </ul>
          </div>
        </div>
      </div>
    {/if}

    {#if showQuickSwitch}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay qs-overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeQuickSwitch(); }}>
        <div class="qs-card">
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="qs-input"
            bind:value={quickQuery}
            oninput={() => (quickIdx = 0)}
            onkeydown={onQuickKey}
            placeholder="Jump to a channel, surface, server or DM…"
            aria-label="Quick switcher"
            autofocus
          />
          <ul class="qs-list">
            {#each quickResults as item, i}
              <li>
                <button
                  type="button"
                  class="qs-item"
                  class:active={i === quickIdx}
                  onmouseenter={() => (quickIdx = i)}
                  onclick={() => runQuick(item)}
                >
                  <span class="qs-label">{item.label}</span>
                  <span class="qs-hint">{item.hint}</span>
                </button>
              </li>
            {:else}
              <li class="qs-empty muted">Nothing matches.</li>
            {/each}
          </ul>
        </div>
      </div>
    {/if}

    {#if toasts.length || updateAvail}
      <div class="toast-stack" aria-live="polite">
        {#if updateAvail}
          <div class="toast update-card" role="status">
            <div class="upd-head">
              <span class="upd-title">Mewtual {updateAvail.version} is available</span>
              <span class="muted small">you have {APP_VERSION}</span>
            </div>
            {#if updateAvail.notes}
              <p class="upd-notes">{updateAvail.notes}</p>
            {/if}
            {#if updateBusy}
              <div class="upd-progress" role="progressbar" aria-valuenow={updatePct} aria-valuemin="0" aria-valuemax="100">
                <span style={`width:${updatePct}%`}></span>
              </div>
              <span class="muted small">Downloading{updatePct ? ` ${updatePct}%` : ""}: Mewtual restarts when it is done.</span>
            {:else}
              <div class="upd-actions">
                <button class="primary small" onclick={installUpdate}>Update and restart</button>
                <button class="ghost small" onclick={() => dismissUpdate(false)}>Later</button>
                <button class="ghost small" onclick={() => dismissUpdate(true)}>Skip this version</button>
              </div>
            {/if}
          </div>
        {/if}
        {#each toasts as t (t.id)}
          <div class="toast {t.kind}" role="status">
            <span class="toast-text">{t.text}</span>
            <button class="toast-x" aria-label="Dismiss" title="Dismiss" onclick={() => dismissToast(t.id)}>✕</button>
          </div>
        {/each}
      </div>
    {/if}

    {#if menu}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div
        class="ctx-backdrop"
        role="presentation"
        onclick={() => (menu = null)}
        oncontextmenu={(e) => { e.preventDefault(); menu = null; }}
      ></div>
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="ctx-menu"
        bind:this={menuEl}
        role="menu"
        tabindex="-1"
        style="left:{menu.x}px; top:{menu.y}px"
        onkeydown={onMenuKey}
      >
        {#each menu.items as item}
          {#if "divider" in item}
            <div class="ctx-divider"></div>
          {:else}
            <button
              class="ctx-item"
              class:danger={item.danger}
              role="menuitem"
              tabindex="-1"
              disabled={item.disabled}
              onclick={() => { const keep = item.onSelect(); if (keep !== true) menu = null; }}
            >
              {#if item.icon}<span class="ctx-icon">{item.icon}</span>{/if}<span class="ctx-label">{item.label}</span>
            </button>
          {/if}
        {/each}
      </div>
    {/if}
  {/if}
</main>
