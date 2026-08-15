<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { renderMessage, renderWiki } from "./render";
  import { refLabel, fileMarker, statusMarker, wikiMarker, eventMarker, insertInto } from "./refs";
  import QRCode from "qrcode";
  import jsQR from "jsqr";
  import { decodeAudio, encodeAudio, MAX_AUDIO_PAYLOAD } from "./audiocode";
  import {
    type MelodyEvent, NOTE_NAMES, noteName, DUR_MAX_MS, DUR_NAMES, durClass, normalizeEvent,
    encodeMelody, melodyBits as bitsOf, chordName, PC_SHARP, TREBLE_LINES, BASS_LINES, yOf,
    STAFF_TOP, STAFF_BOT, HEAD_RX, HEAD_RY, buildSheet, scoreText,
  } from "./melody";

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
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string; description: string; bubble: string; avatar: string };
  type UiFile = { name: string; size: number; mime: string; cid: string; author: string; path: string; held: number; total: number };
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
  let showSettings = $state(false); // the personal/app Settings overlay
  let showServerSettings = $state(false); // the per-server (admin) Settings overlay
  let serverNameDraft = $state("");

  function openServerSettings(id: number | null = null) {
    if (id !== null && id !== activeServerId) switchServer(id);
    serverNameDraft = cur?.name ?? "";
    // The draft never carries the images: set_livery ignores them (set_server_icon /
    // set_server_cursor own those fields).
    liveryDraft = { preset: livery.preset, accent: livery.accent, tokens: { ...livery.tokens }, icon: "", cursor: "" };
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
  let feedbackText = $state("");
  let feedbackCopied = $state(false);
  // Notification-sound preference (wired to actual playback in 10g), persisted locally.
  let soundOn = $state(typeof localStorage !== "undefined" ? localStorage.getItem("catcoms.sound") !== "off" : true);
  function toggleSound() {
    soundOn = !soundOn;
    try { localStorage.setItem("catcoms.sound", soundOn ? "on" : "off"); } catch { /* ignore */ }
  }

  // Appearance: the whole theme is a token map in app.css; these choices only flip
  // data-attributes / one CSS variable on <html>, so they can never fork the layout.
  // Semantic colours (green=presence, gold=mentions, red=danger) are constant in every preset.
  type Appearance = { preset: string; accent: string; density: string; chrome: string; flat: boolean; icons: string };
  const APPEARANCE_KEY = "catcoms.appearance";
  const APPEARANCE_DEFAULT: Appearance = { preset: "", accent: "", density: "", chrome: "terminal", flat: true, icons: "" };
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
    const followLivery = liveryActive && !liveryOptOut && !dmHome && !inboxView && activeServerId !== null;
    const preset = followLivery ? livery.preset : appearance.preset;
    const accent = followLivery ? livery.accent : appearance.accent;
    set("preset", preset);
    set("density", appearance.density);
    set("chrome", appearance.chrome === "clean" ? "clean" : "terminal");
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
    try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(appearance)); } catch { /* best-effort */ }
  });

  // Persistence (9f): a passphrase gate. On launch the app is locked until the user enters
  // their passphrase, which unlocks the on-disk vault and reloads their servers (or, on
  // first run, sets the passphrase and starts empty).
  let locked = $state(true);
  let passphrase = $state("");
  let unlocking = $state(false);

  // --- Unlock minigames -----------------------------------------------------------------
  // Input surfaces ONLY: every method deterministically encodes to a scheme-prefixed string
  // that feeds the SAME vault KDF ("unlock" invoke): the vault crypto is untouched, and a
  // passphrase remains the recommended, highest-entropy option. The scheme prefix means the
  // same finger pattern on different games can never collide into the same secret.
  type UnlockMethod = "pass" | "spell" | "melody";
  let unlockMethod = $state<UnlockMethod>("pass");
  // Spell lock: glyphs picked in order from a fixed catalog, encoded by INDEX (stable even
  // if the glyph art changes): "spell:v1:3-17-9-…". ~4.6 bits per pick.
  const SPELL_GLYPHS = ["🐱", "🌙", "⭐", "🔥", "❄️", "🍀", "🗝️", "🕯️", "💀", "🌿", "🍄", "⚡", "🌊", "🪶", "🔔", "🫧", "🦴", "🌸", "☄️", "🐟", "🧶", "🪙", "🎐", "🕸️"];
  let spellSeq = $state<number[]>([]);
  let spellSecret = $derived(spellSeq.length ? `spell:v1:${spellSeq.join("-")}` : "");
  let spellBits = $derived(Math.round(spellSeq.length * Math.log2(SPELL_GLYPHS.length)));
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
  const voices = new Map<number, { osc: OscillatorNode; gain: GainNode }>();
  function startTone(note: number) {
    try {
      synthCtx ??= new AudioContext();
      if (synthCtx.state === "suspended") void synthCtx.resume();
      stopTone(note);
      const o = synthCtx.createOscillator();
      const g = synthCtx.createGain();
      o.type = "triangle";
      o.frequency.value = noteHz(note);
      const t = synthCtx.currentTime;
      g.gain.setValueAtTime(0.0001, t);
      g.gain.exponentialRampToValueAtTime(0.16, t + 0.012); // quick attack
      g.gain.exponentialRampToValueAtTime(0.07, t + 0.4); // settle to a sustain that holds
      o.connect(g).connect(synthCtx.destination);
      o.start();
      o.stop(t + 8); // hard backstop so a lost note-off can never leave a drone
      voices.set(note, { osc: o, gain: g });
    } catch {
      /* no audio output: the note still registers */
    }
  }
  function stopTone(note: number) {
    const v = voices.get(note);
    if (!v || !synthCtx) return;
    voices.delete(note);
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
    for (const n of [...voices.keys()]) if (!heldNotes.includes(n)) stopTone(n);
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
            if (!locked || unlockMethod !== "melody") return;
            const status = d[0] & 0xf0;
            // Note-off is either 0x80 or a 0x90 with zero velocity: controllers disagree.
            if (status === 0x90 && d[2] > 0) noteOn(d[1]);
            else if (status === 0x80 || (status === 0x90 && d[2] === 0)) noteOff(d[1]);
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
    return unlockMethod === "pass" ? passphrase : unlockMethod === "spell" ? spellSecret : melodySecret;
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
  let activeWikiPage = $state("");
  let wikiBody = $state("");
  let newWikiPage = $state("");
  let wikiDirty = $state(false); // unsaved edits in the open page (avoid clobbering on live updates)
  let wikiEdit = $state(false); // edit (textarea) vs read (rendered) mode
  let wikiEl = $state<HTMLDivElement | undefined>(undefined); // rendered-page container (media resolve)
  let showWikiHelp = $state(false);

  // Pages whose body links to the open page ([[Open Page]]).
  let backlinks = $derived.by(() => {
    if (!activeWikiPage) return [] as string[];
    const needle = `[[${activeWikiPage}]]`.toLowerCase();
    return Object.entries(wikiMap)
      .filter(([name, body]) => name !== activeWikiPage && (body ?? "").toLowerCase().includes(needle))
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
  // The name-style picker's choices (font face / text effect / colour). Ids are the opaque
  // strings stored in the profile; the tiles preview each one live.
  const NAME_FONTS: { id: string; label: string }[] = [
    { id: "system", label: "System" },
    { id: "serif", label: "Serif" },
    { id: "mono", label: "Mono" },
    { id: "script", label: "Script" },
    { id: "caps", label: "Small caps" },
  ];
  const NAME_EFFECTS: { id: string; label: string }[] = [
    { id: "none", label: "Solid" },
    { id: "gradient", label: "Gradient" },
    { id: "neon", label: "Neon" },
    { id: "rainbow", label: "Rainbow" },
    { id: "wave", label: "Wave" },
    { id: "pulse", label: "Pulse" },
  ];
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
  type UiEvent = { id: string; title: string; body: string; start_ts: number; end_ts: number; author: string };
  let events = $state<UiEvent[]>([]);
  let evTitle = $state("");
  let evBody = $state("");
  let evStart = $state("");
  let evEnd = $state("");
  let confirmDeleteEventId = $state("");
  async function refreshEvents() {
    if (activeServerId === null) {
      events = [];
      return;
    }
    try {
      events = await invoke<UiEvent[]>("get_events", { server: activeServerId });
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
      await invoke("create_event", { server: activeServerId, title: evTitle, body: evBody, startTs, endTs });
      evTitle = ""; evBody = ""; evStart = ""; evEnd = "";
      await refreshEvents();
    } catch (e) {
      error = String(e);
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
    inboxView = false;
    switchServer(n.server).then(() => switchView(n.kind === "event" ? "events" : "status"));
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
      if (first?.server !== undefined && first.server !== null) switchServer(first.server);
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
    const id = effect.toLowerCase().replace(/[^a-z0-9-]/g, "");
    return id && id !== "none" ? `fx-${id}` : "";
  }
  function colorStyle(color: string): string {
    return color ? `color:${color}` : "";
  }
  function fmtTime(ts: number): string {
    return ts ? new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
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
    void view; // switching tabs destroys + recreates this DOM (fresh, unresolved placeholders)
    void inboxView; // returning from the inbox recreates the chat DOM too
    tick().then(() => {
      resolveMedia(messagesEl);
      resolveEmoji(messagesEl);
      resolveMedia(statusEl);
      resolveEmoji(statusEl);
    });
  });

  // Resolve embeds + emoji + mark missing [[links]] in the rendered wiki page (read mode).
  $effect(() => {
    void wikiBody;
    void wikiPages;
    void wikiEdit;
    void files;
    void emojiUrls;
    void view; // re-resolve after a tab switch recreates the wiki DOM
    if (!wikiEdit) {
      tick().then(() => {
        resolveMedia(wikiEl);
        resolveEmoji(wikiEl);
        resolveWikiLinks(wikiEl);
      });
    }
  });

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
    try {
      const reloaded = await invoke<Reloaded[]>("unlock", { passphrase: secret });
      passphrase = "";
      spellSeq = [];
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
      error = String(e);
    } finally {
      unlocking = false;
    }
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
    } catch (e) {
      error = String(e);
    }
  }

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
      statuses = await invoke<Msg[]>("get_statuses", { server: activeServerId });
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
    node.addEventListener("click", h);
    node.addEventListener("contextmenu", c);
    return {
      destroy: () => {
        node.removeEventListener("click", h);
        node.removeEventListener("contextmenu", c);
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
    const h = (e: MouseEvent) => openMenu(e, make());
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
  $effect(() => {
    void draft;
    if (composerEl) {
      composerEl.style.height = "auto";
      composerEl.style.height = `${Math.min(composerEl.scrollHeight, 140)}px`;
    }
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
        { label: "Open file details", icon: "ⓘ", onSelect: () => openFileRef(cid) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](file:${cid})`) },
        { label: "Copy address (CID)", icon: "#", onSelect: () => copyText(cid) },
      ]);
    } else if (el.hasAttribute("data-status-id")) {
      const id = el.getAttribute("data-status-id") ?? "";
      const label = chipLabel(el) || "status";
      openMenu(e, [
        { label: "Open status", icon: "⊞", onSelect: () => openStatusRef(id) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](status:${id})`) },
      ]);
    } else if (el.hasAttribute("data-event-id")) {
      const id = el.getAttribute("data-event-id") ?? "";
      const label = chipLabel(el) || "event";
      openMenu(e, [
        { label: "Open event", icon: "⧗", onSelect: () => openEventRef(id) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[${refLabel(label)}](event:${id})`) },
      ]);
    } else if (el.hasAttribute("data-wikilink")) {
      const page = el.getAttribute("data-wikilink") ?? "";
      openMenu(e, [
        { label: "Open page", icon: "⊞", onSelect: () => { view = "wiki"; openWikiPage(page); } },
        { label: "Post link to chat", icon: "➦", onSelect: () => appendToDraft(`[[${page}]]`) },
        { label: "Copy link", icon: "⧉", onSelect: () => copyText(`[[${page}]]`) },
      ]);
    } else if (el.hasAttribute("data-emoji")) {
      const code = (el.getAttribute("data-emoji") ?? "").replace(/:/g, "");
      openMenu(e, [{ label: `Copy :${code}:`, icon: "⧉", onSelect: () => copyText(`:${code}:`) }]);
    } else {
      const cid = el.getAttribute("data-embed-cid") ?? "";
      openMenu(e, [{ label: "Copy address (CID)", icon: "#", onSelect: () => copyText(cid) }]);
    }
  }
  async function refreshWiki() {
    if (activeServerId === null) return;
    try {
      wikiPages = await invoke<string[]>("get_wiki_pages", { server: activeServerId });
      wikiMap = await invoke<Record<string, string>>("get_wiki_map", { server: activeServerId });
      // Reload the open page only if it still exists and the user isn't mid-edit.
      if (activeWikiPage && !wikiDirty && wikiPages.includes(activeWikiPage)) {
        wikiBody = await invoke<string>("get_wiki_page", { server: activeServerId, name: activeWikiPage });
      }
    } catch (e) {
      error = String(e);
    }
  }
  async function openWikiPage(name: string) {
    if (activeServerId === null) return;
    try {
      wikiBody = await invoke<string>("get_wiki_page", { server: activeServerId, name });
      activeWikiPage = name;
      wikiDirty = false;
      view = "wiki";
      // Existing pages open in read mode; a not-yet-created page (e.g. a [[link]] target) in edit.
      wikiEdit = !wikiPages.includes(name);
    } catch (e) {
      error = String(e);
    }
  }
  async function createWikiPage() {
    const name = newWikiPage.trim();
    if (!name || activeServerId === null) return;
    newWikiPage = "";
    try {
      if (!wikiPages.includes(name)) {
        await invoke("save_wiki_page", { server: activeServerId, name, body: "" });
        await refreshWiki();
      }
      await openWikiPage(name);
      wikiEdit = true;
    } catch (e) {
      error = String(e);
    }
  }

  // Embed media into the open wiki page: upload under wiki/<page>/, append a marker.
  async function wikiEmbed(fileList: FileList | null) {
    if (!fileList || fileList.length === 0 || activeServerId === null || !activeWikiPage) return;
    uploading = true;
    try {
      for (const file of Array.from(fileList)) {
        const cid = await invoke<string>("add_file", {
          server: activeServerId,
          name: file.name,
          mime: file.type || "application/octet-stream",
          path: `wiki/${activeWikiPage}`,
          data: await readBase64(file),
        });
        const alt = file.name.replace(/[[\]]/g, " ");
        wikiBody = wikiBody ? `${wikiBody}\n\n![${alt}](cid:${cid})` : `![${alt}](cid:${cid})`;
        wikiDirty = true;
      }
      await refreshFiles();
    } catch (e) {
      error = String(e);
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
      await invoke("save_wiki_page", { server: activeServerId, name: activeWikiPage, body: wikiBody });
      wikiDirty = false;
    } catch (e) {
      error = String(e);
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

  async function loadAvatar(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      pAvatar = await fileToSquareJpegB64(file, 128);
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
    try {
      await invoke("add_file", {
        server: activeServerId,
        name: file.name,
        mime: file.type || "application/octet-stream",
        path: folder,
        data: await readBase64(file),
      });
      await refreshFiles();
    } catch (e) {
      error = String(e);
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
      }
      await refreshFiles();
    } catch (e) {
      error = String(e);
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

  function buildMediaEl(mime: string, url: string, alt: string): HTMLElement {
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
      el.className = "embed-media";
    }
    el.src = url;
    return el;
  }

  function downloadChip(file: UiFile): HTMLElement {
    const b = document.createElement("button");
    b.className = "embed-chip";
    b.textContent = `📎 ${file.name}`;
    b.onclick = () => downloadFile(file);
    return b;
  }

  // Replace `[data-embed-cid]` placeholders (from the renderer) with media built in code from
  // the group's own content-addressed blobs: never via untrusted innerHTML, so a peer's text
  // can't inject a live tag or remote URL. Only media MIME types embed; others get a chip.
  async function resolveMedia(container: HTMLElement | undefined) {
    if (!container || activeServerId === null) return;
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
        let url = embedCache.get(cid);
        if (!url) {
          const base64 = await invoke<string>("download_file", { server: activeServerId, cid });
          url = `data:${mime};base64,${base64}`;
          embedCache.set(cid, url);
          // Bound the cache (each entry is a full decrypted blob): FIFO-evict the oldest.
          if (embedCache.size > 48) {
            const oldest = embedCache.keys().next().value;
            if (oldest !== undefined) embedCache.delete(oldest);
          }
        }
        span.replaceWith(buildMediaEl(mime, url, alt));
      } catch {
        span.replaceWith(downloadChip(file));
      }
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
    const id = activeServerId;
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

  // --- "+" insert picker: link/embed this server's own content into the message -----------------
  // Everything the group already holds is addressable from the composer: a fileshare file (inline
  // embed for media, a link chip otherwise), one of YOUR status posts, or a wiki page. Each inserts
  // a marker the shared renderer resolves: nothing here leaves the group or touches the network.
  type InsertTab = "files" | "status" | "wiki" | "events";
  let showInsert = $state(false);
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

  async function toggleInsert() {
    if (showInsert) {
      closeInsert();
      return;
    }
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

  // Insert at the composer caret (mirrors pickMention), leaving the caret just after it. The string
  // maths lives in refs.ts so it can be unit-tested away from the DOM.
  function insertAtCaret(insert: string) {
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
    else error = "That file is no longer in this server's file index.";
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
  }

  // --- Voice calls (full-mesh WebRTC; E2E via authenticated signalling + DTLS-SRTP) -------------
  // Each pair of participants connects directly (no server in the media path), so SRTP is end-to-end;
  // the SDP/ICE is exchanged over the members-only, signed KIND_CALL_SIGNAL push, so the DTLS
  // fingerprints can't be MITM'd. A future MLS-keyed frame layer (SFrame) is only needed for an SFU.
  type CallPeer = { fp: string; pc: RTCPeerConnection };
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
  async function ensureMic(): Promise<boolean> {
    if (localStream) return true;
    try {
      localStream = await navigator.mediaDevices.getUserMedia({ audio: true, video: false });
      return true;
    } catch {
      error = "Couldn't access the microphone (permission denied or no device).";
      return false;
    }
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
  }
  function createPeer(fp: string): RTCPeerConnection {
    const pc = new RTCPeerConnection({ iceServers: iceServers() });
    if (localStream) for (const t of localStream.getTracks()) pc.addTrack(t, localStream);
    pc.onicecandidate = (e) => {
      if (e.candidate) void sendSignal(fp, { callId: callChannel, type: "ice", candidate: e.candidate.toJSON() });
    };
    pc.ontrack = (e) => attachRemote(fp, e.streams[0]);
    pc.onconnectionstatechange = () => {
      callPeerStates = { ...callPeerStates, [fp]: pc.connectionState };
      if (pc.connectionState === "failed" || pc.connectionState === "closed") removePeer(fp);
    };
    callPeers[fp] = { fp, pc };
    callParticipants = Object.keys(callPeers);
    return pc;
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
    voiceAlert = null;
    alertedRooms.delete(roomKey(server, channel));
    recordPresence(server, channel, myFp);
    broadcast({ callId: channel, type: "hello" }); // announce + trigger existing members to offer
    clearInterval(pingTimer);
    pingTimer = setInterval(() => {
      if (callChannel && callServer !== null) {
        broadcast({ callId: callChannel, type: "voice-ping" });
        recordPresence(callServer, callChannel, myFp); // keep my own presence fresh
      }
    }, 5000);
  }
  function leaveVoice() {
    if (callChannel) broadcast({ callId: callChannel, type: "bye" });
    for (const fp of Object.keys(callPeers)) removePeer(fp);
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
  }
  function joinActiveVoice() {
    if (activeServerId !== null && cur?.active) joinVoice(cur.active, activeServerId, activeName());
  }
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
      if (type === "voice-ping") return; // presence only
    }
    // WebRTC negotiation: only for MY current room.
    if (!inCall || cid !== callChannel) return;
    if (type === "hello") {
      if (callPeers[fromFp]) return;
      const pc = createPeer(fromFp);
      await pc.setLocalDescription(await pc.createOffer());
      void sendSignal(fromFp, { callId: callChannel, type: "offer", sdp: pc.localDescription });
    } else if (type === "offer") {
      const pc = callPeers[fromFp]?.pc ?? createPeer(fromFp);
      await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
      await pc.setLocalDescription(await pc.createAnswer());
      void sendSignal(fromFp, { callId: callChannel, type: "answer", sdp: pc.localDescription });
    } else if (type === "answer") {
      const pc = callPeers[fromFp]?.pc;
      if (pc) await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
    } else if (type === "ice") {
      const pc = callPeers[fromFp]?.pc;
      if (pc && msg.candidate) {
        try { await pc.addIceCandidate(new RTCIceCandidate(msg.candidate as RTCIceCandidateInit)); } catch { /* stale */ }
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

  async function copyFeedback() {
    const report = [
      `Type: ${feedbackKind === "bug" ? "Bug report" : "Feature request"}`,
      `App: CatComs (desktop)`,
      `Environment: ${navigator.userAgent}`,
      ``,
      feedbackText.trim(),
    ].join("\n");
    try {
      await navigator.clipboard.writeText(report);
      feedbackCopied = true;
      setTimeout(() => (feedbackCopied = false), 2000);
    } catch (e) {
      error = String(e);
    }
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

  onMount(() => {
    const subs: Promise<UnlistenFn>[] = [
      listen<{ server: number; channel: string }>("channel-updated", (e) => {
        const { server, channel } = e.payload;
        // Any server's channel changed → the cross-server inbox may have a new entry (debounced).
        scheduleInboxReload();
        // A DM got a message → its activity stats changed; keep the friends sorting fresh.
        if (dmHome && servers.find((x) => x.id === server)?.isDm) refreshDmStats();
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
      if (locked && unlockMethod === "melody" && !e.ctrlKey && !e.metaKey && !e.altKey) {
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
          unlock();
          return;
        }
      }
      if (e.key === "Escape") {
        if (showQuickSwitch) closeQuickSwitch();
        else if (scanOpen) closeScan(null);
        else if (showLinkDevice) closeLinkDevice();
        else if (verifyFor) verifyFor = null;
        else if (menu) menu = null;
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
        } else if (e.key.toLowerCase() === "k") {
          e.preventDefault();
          openQuickSwitch();
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
      const note = keyNotes.get(e.key.toLowerCase());
      if (note === undefined) return;
      keyNotes.delete(e.key.toLowerCase());
      noteOff(note);
    };
    // Losing focus mid-hold would otherwise strand a sounding note and an open chord group.
    const onBlur = () => {
      keyNotes.clear();
      releaseAll();
      stopPlayback();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    // Keep relative presence times current.
    const tick = setInterval(() => (nowTick = Date.now()), 60_000);
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
      releaseAll();
      stopPlayback();
      clearInterval(tick);
      clearInterval(callCleanup);
      clearTimeout(inboxTimer);
      clearInterval(pingTimer);
      subs.forEach((p) => p.then((un) => un()));
    };
  });
</script>

{#snippet styledName(name: string, color: string, font: string, effect: string)}
  <span class="name {fontClass(font)} {fxClass(effect)}" style={colorStyle(color)}>{name}</span>
{/snippet}

{#snippet nameTag(fp: string)}
  {@const p = profiles[fp]}
  {@render styledName(nameOf(fp), p?.color ?? "", p?.font ?? "", p?.effect ?? "")}
{/snippet}

{#snippet avatarTag(fp: string)}
  {@const p = profiles[fp]}
  {#if p?.avatar}
    <img class="avatar" src={"data:image/jpeg;base64," + p.avatar} alt="" />
  {:else}
    <span class="avatar fallback" style={p?.color ? `background:${p.color}` : ""}>
      {nameOf(fp).slice(0, 1).toUpperCase()}
    </span>
  {/if}
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

{#snippet icoCat()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M5 10c0-1.2.25-2.3.72-3.25L4.6 2.9l4 1.9C9.7 4.3 10.8 4 12 4s2.3.3 3.4.8l4-1.9-1.12 3.85c.47.95.72 2.05.72 3.25 0 4.6-3.1 7.6-7 7.6s-7-3-7-7.6z" />
    <circle cx="9.2" cy="10.6" r="0.9" fill="currentColor" stroke="none" />
    <circle cx="14.8" cy="10.6" r="0.9" fill="currentColor" stroke="none" />
    <path d="M1.8 11.2l3.1.5M2 14.3l3-.6M22.2 11.2l-3.1.5M22 14.3l-3-.6" stroke-width="1.2" />
    <path d="M12 13.4l-.9 1.1h1.8z" fill="currentColor" stroke="none" />
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

<!--
  The sidebar's contextual block: what the sidebar shows depends on the surface selected in the
  content column's surface strip. Shared by the server and DM sidebars: `dm` suppresses the blocks
  that only make sense on a server (channels, the status feed's blurb).
-->
{#snippet contextNav(dm: boolean)}
  {#if view === "wiki"}
    <h3 class="ctx-h">
      <span>Pages</span>
      <button class="wiki-help-btn" title="Formatting help" onclick={() => (showWikiHelp = true)}>?</button>
    </h3>
    {#if wikiPages.length > 6}
      <input class="list-search" bind:value={wikiFilter} placeholder="Search pages…" />
    {/if}
    <ul class="channel-list wiki-pages">
      {#each filteredWikiPages as p}
        <li>
          <button
            class:active={p === activeWikiPage}
            onclick={() => openWikiPage(p)}
            use:contextMenu={() => wikiPageMenu(p)}
          >{p}</button>
        </li>
      {:else}
        <li class="muted small">{wikiFilter ? "No matching pages." : "No pages yet."}</li>
      {/each}
    </ul>
    <form class="new-page" onsubmit={(e) => { e.preventDefault(); createWikiPage(); }}>
      <input bind:value={newWikiPage} placeholder="+ new page" />
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

<main>
  {#if eclipseCaution && activeServerId !== null && !locked}
    <div class="eclipse-banner" role="status">
      ⚠ You may be isolated from this server: few members are reachable. Verify a member out of band.
    </div>
  {/if}
  {#if locked}
    <div class="start">
      <h1>CatComs</h1>
      <p class="muted">
        Unlock your servers: with a passphrase, a spell, or a tune. All three seal the
        same vault; pick the one you'll actually remember.
      </p>
      <div class="ul-tabs" role="tablist">
        <button type="button" role="tab" class:active={unlockMethod === "pass"} aria-selected={unlockMethod === "pass"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "pass"; }}>
          Passphrase <span class="ul-rec">recommended</span>
        </button>
        <button type="button" role="tab" class:active={unlockMethod === "spell"} aria-selected={unlockMethod === "spell"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "spell"; }}>Spell</button>
        <button type="button" role="tab" class:active={unlockMethod === "melody"} aria-selected={unlockMethod === "melody"} onclick={() => { unlockMethod = "melody"; initMidi(); }}>Melody</button>
      </div>
      {#if unlockMethod === "pass"}
        <label class="field">
          <span class="muted">Passphrase</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="password"
            bind:value={passphrase}
            onkeydown={(e) => e.key === "Enter" && passphrase && unlock()}
            placeholder="passphrase"
            autofocus
          />
        </label>
      {:else if unlockMethod === "spell"}
        <p class="muted small">
          Cast your unlock spell: the same glyphs, in the same order, every time. Memorable,
          but weaker than a good passphrase; longer spells are stronger.
        </p>
        <div class="spell-grid">
          {#each SPELL_GLYPHS as g, i (i)}
            <button type="button" class="spell-glyph" title={`glyph ${i + 1}`} onclick={() => (spellSeq = [...spellSeq, i])}>{g}</button>
          {/each}
        </div>
        <div class="ul-seq">
          {#if spellSeq.length}
            <span class="ul-seq-chips">{#each spellSeq as s, i (i)}<span>{SPELL_GLYPHS[s]}</span>{/each}</span>
            <button type="button" class="ghost small" title="Remove the last glyph" onclick={() => (spellSeq = spellSeq.slice(0, -1))}>⌫</button>
            <button type="button" class="ghost small" onclick={() => (spellSeq = [])}>Clear</button>
          {:else}
            <span class="muted small">Nothing cast yet.</span>
          {/if}
        </div>
        {#if spellSeq.length}
          <div class="ul-meter {bitsTier(spellBits)}">≈ {spellBits} bits{spellBits < 28 ? ": too short, add more glyphs" : spellBits < 44 ? ": okay; longer is stronger" : ": strong"}</div>
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
      <p class="muted small">
        First run: whatever you enter here <em>becomes</em> the vault secret: practice your
        sequence before committing, and prefer a passphrase for the strongest protection.
        There is no recovery.
      </p>
      <button onclick={() => unlock()} disabled={unlocking || !unlockSecret()}>
        {unlocking ? "Unlocking…" : "Unlock"}
      </button>
    </div>
  {:else if servers.length === 0 || showAdd}
    <div class="start">
      <h1>CatComs</h1>
      {#if showAdd && servers.length}
        <button class="ghost" onclick={() => (showAdd = false)}>← back</button>
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
      <details>
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
      <nav class="rail">
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
        {#each railServers as s}
          <button
            class="server-icon"
            class:active={s.id === activeServerId && !dmHome && !inboxView}
            title={s.name}
            onclick={() => switchServer(s.id)}
            use:contextMenu={() => serverMenu(s)}
          >
            {#if serverIcons[s.id] && appearance.icons !== "flat"}
              <img class="rail-img" src={"data:image/jpeg;base64," + serverIcons[s.id]} alt="" />
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
        <button class="server-icon feedback-btn" title="Send feedback (bug / feature request)" onclick={() => (showFeedback = true)}>FB</button>
        <button class="server-icon gear" title="Settings" onclick={() => (showSettings = true)}>{@render icoGear()}</button>
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
          <button class="ghost invite-quick" onclick={() => openServerSettings()}>＋ Invite someone</button>
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
          <h2>
            #{activeName()} <span class="muted">· {members} member{members === 1 ? "" : "s"}</span>
            <span class="chip ok" title="Messages in this group are end-to-end encrypted (MLS)">MLS · E2E</span>
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
            <button class="ghost icon-btn search-toggle" title="Search messages (Ctrl+F · Ctrl+Shift+F for filters)" onclick={() => openSearch()}>{@render icoSearch()}</button>
            {#if !(inCall && callChannel === cur?.active)}
              {@const n = roomMembers(activeServerId ?? -1, cur?.active ?? "").length}
              <button class="ghost small call-start btn-ico" title="Join this channel's voice room (E2E)" onclick={joinActiveVoice}>{@render icoPhone()} {n ? `Join voice (${n})` : "Voice"}</button>
            {/if}
            {#if pinnedMsgs.length}
              <button class="ghost small pinned-toggle btn-ico" class:active={showPinned} title="Pinned messages" onclick={() => (showPinned = !showPinned)}>{@render icoPin()} {pinnedMsgs.length}</button>
            {/if}
            {#if firstUnreadIdx >= 0}
              <button class="ghost small jump-unread" title="Jump to where you left off" onclick={() => scrollToMatch(firstUnreadIdx)}>↑ New</button>
            {/if}
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
                <li class="unread-divider" aria-hidden="true"><span>New messages</span></li>
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
              <li
                data-mi={mi}
                class:own={m.author === myFp}
                class:grouped
                class:has-bubble={!!bubble}
                class:search-match={showSearch && searchMatchSet.has(mi)}
                class:search-current={showSearch && searchCur?.ch === cur?.active && searchCur?.idx === mi}
                class:flash={!!m.id && m.id === flashId}
                style={bubble}
                use:contextMenu={() => messageMenu(m)}
              >
                <span class="t" title={new Date(m.ts).toLocaleString()}>
                  {#if tick}<span class="dtick {tick.cls}" title={tick.tip}>{tick.g}</span>{/if}{fmtTime(m.ts)}
                </span>
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
                  {@const ident = identityOf(m.author)}
                  <span class="author">
                    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
                    <span class="author-link" role="button" tabindex="0" onclick={() => showProfile(ident.fp)}>
                      {@render avatarTag(ident.fp)}
                      {@render nameTag(ident.fp)}
                    </span>
                    {#if ident.tag}<span class="dev-tag" title="Sent from this member's linked device">· {ident.tag}</span>{/if}
                    {#if m.author !== myFp && verifiedFps.has(m.author)}
                      <span class="vf-check" title="You verified this member out of band">✓</span>
                    {/if}
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
            {#if showInsert}
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
                  onkeydown={(e) => { if (e.key === "Escape") { e.preventDefault(); closeInsert(); composerEl?.focus(); } }}
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
                class:on={showInsert}
                title="Link or embed a file, one of your status posts, or a wiki page"
                aria-label="Insert a link or embed"
                aria-expanded={showInsert}
                onclick={toggleInsert}
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
            {#if activeWikiPage}
              <div class="wiki-editor">
                <div class="wiki-editor-head">
                  <h2>{activeWikiPage} {#if wikiDirty}<span class="muted">· unsaved</span>{/if}</h2>
                  <div class="wiki-mode">
                    <button class:active={!wikiEdit} onclick={() => (wikiEdit = false)}>Read</button>
                    <button class:active={wikiEdit} onclick={() => (wikiEdit = true)}>Edit</button>
                  </div>
                </div>
                {#if wikiEdit}
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="wiki-edit"
                    class:drag-over={dragOver}
                    ondragover={(e) => { e.preventDefault(); dragOver = true; }}
                    ondragleave={() => (dragOver = false)}
                    ondrop={onWikiDrop}
                  >
                    <textarea bind:value={wikiBody} oninput={() => (wikiDirty = true)} rows="18"
                      placeholder="Markdown. [[Page]] links to another page; drop or attach a file to embed it."></textarea>
                    <div class="wiki-edit-actions">
                      <label class="attach" title="Attach image / video / audio">
                        📎
                        <input type="file" accept="image/*,video/*,audio/*" multiple disabled={uploading}
                          onchange={(e) => { wikiEmbed(e.currentTarget.files); e.currentTarget.value = ''; }} />
                      </label>
                      <button onclick={saveWikiPage} disabled={!wikiDirty}>Save page</button>
                    </div>
                  </div>
                {:else}
                  <div class="wiki-render" bind:this={wikiEl} use:richClicks>{@html renderWiki(wikiBody)}</div>
                  {#if backlinks.length}
                    <div class="wiki-backlinks">
                      <h4>Linked from</h4>
                      <ul>
                        {#each backlinks as b}
                          <li><button class="wikilink" onclick={() => openWikiPage(b)}>{b}</button></li>
                        {/each}
                      </ul>
                    </div>
                  {/if}
                {/if}
              </div>
            {:else}
              <p class="muted wiki-empty">Select a page on the left, or create one. Use <code>[[Page Name]]</code> to link pages.</p>
            {/if}
          </div>
        {:else if view === "profile"}
          <h2>Your profile</h2>
          <div class="profile-tab tab-pane">
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
                  <button
                    type="button"
                    class="ns-tile"
                    class:active={pEffect === fx.id}
                    title={fx.label}
                    aria-label={fx.label}
                    aria-pressed={pEffect === fx.id}
                    onclick={() => (pEffect = fx.id)}
                  ><span class="name {fxClass(fx.id)}" style={colorStyle(pColor)}>{fx.label}</span></button>
                {/each}
              </div>
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
              </div>
            </div>
            <div class="field">
              <span class="muted">Avatar</span>
              <div class="avatar-row">
                {#if pAvatar}
                  <img class="avatar lg" src={"data:image/jpeg;base64," + pAvatar} alt="" />
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
            </div>
            <p class="preview">
              Preview: {@render styledName(pName || displayName, pColor, pFont, pEffect)}
            </p>
            <button onclick={saveProfile}>Save profile</button>
          </div>
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
      <span class="seg">vault <span class="ok-t">unlocked</span></span>
      {#if rendezvous.trim()}<span class="seg">rendezvous <span class="ok-t">set</span></span>{/if}
      {#if activeDownloads}<span class="seg"><span class="warn-t">⇣ {activeDownloads} transfer{activeDownloads === 1 ? "" : "s"}</span></span>{/if}
      <span class="sb-spacer"></span>
      {#if myFp}<span class="seg" title="Your fingerprint on this server: click a member and compare out of band to verify">id {myFp.slice(0, 4)}·{myFp.slice(4, 8)}</span>{/if}
    </footer>

    {#if inCall}
      <div class="call-bar">
        <span class="call-dot">{@render icoSpeaker()}</span>
        <span class="call-title">Voice · #{callChannelName}</span>
        <span class="call-status muted">{callStatusText}</span>
        <div class="call-avatars">
          {@render avatarTag(myFp)}
          {#each callParticipants as fp}{@render avatarTag(fp)}{/each}
        </div>
        <button class="ghost small btn-ico" class:active={callMuted} title={callMuted ? "Unmute" : "Mute"} onclick={toggleMute}>{#if callMuted}{@render icoMicOff()} Muted{:else}{@render icoMic()} Mute{/if}</button>
        <button class="call-hangup btn-ico" title="Leave voice" onclick={leaveVoice}>{@render icoHangup()} Leave</button>
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
            <div class="pc-top">
              {#if p?.avatar}
                <img class="avatar lg" src={"data:image/jpeg;base64," + p.avatar} alt="" />
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

    {#if showSettings}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) showSettings = false; }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>Settings</h2>
            <button class="ghost" onclick={() => (showSettings = false)}>✕</button>
          </header>
          <div class="overlay-body">
            <section class="set-section">
              <h3>Appearance</h3>
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
              {#if liveryActive && activeServerId !== null && !cur?.isDm}
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
              {/if}
            </section>

            <section class="set-section">
              <h3>Devices</h3>
              <p class="muted small">
                Link another device to your identity. The new device gets its own key: nothing
                is copied: and nothing at all happens until you approve it here on this device.
              </p>
              <button class="ghost" onclick={() => (showLinkDevice = true)}>⛓ Link a new device…</button>
            </section>

            <section class="set-section">
              <h3>Notifications</h3>
              <label class="toggle">
                <input type="checkbox" checked={soundOn} onchange={toggleSound} />
                <span>Play a sound for new messages</span>
              </label>
              <button class="ghost small" onclick={playNotify} disabled={!soundOn}>Test sound</button>
            </section>

            <section class="set-section">
              <h3>Calls (voice)</h3>
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
                <li><code>:name:</code> → a custom emoji (add them under Emoji below), or use the 😀 picker</li>
                <li><code>[[Page]]</code> → link to a wiki page</li>
                <li><code>- item</code> / <code>1. item</code> → bullet / numbered lists</li>
              </ul>
            </section>

            <section class="set-section">
              <h3>Network</h3>
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

            {#if activeServerId !== null}
              <section class="set-section">
                <h3>This server</h3>
                <button class="ghost" onclick={() => { showSettings = false; openServerSettings(); }}>Open server settings →</button>
              </section>
            {/if}
          </div>
        </div>
      </div>
    {/if}

    {#if showServerSettings}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) showServerSettings = false; }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>Server settings</h2>
            <button class="ghost" onclick={() => (showServerSettings = false)}>✕</button>
          </header>
          <div class="overlay-body">
            <section class="set-section">
              <h3>Server</h3>
              <p>{cur?.name ?? "—"} <span class="role-badge {myRole}">{myRole}</span></p>
              <form class="rename-row" onsubmit={(e) => { e.preventDefault(); renameServer(); }}>
                <input bind:value={serverNameDraft} placeholder="Server name" />
                <button class="ghost small" disabled={!serverNameDraft.trim() || serverNameDraft.trim() === cur?.name}>Rename</button>
              </form>
              <p class="muted small">The name is your own label for this server (not shared with other members).</p>
              <label class="toggle">
                <input type="checkbox" checked={acceptCallsHere} onchange={toggleAcceptCalls} />
                <span>Notify me of voice calls on this server</span>
              </label>
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

            <section class="set-section">
              <h3>Livery</h3>
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
                      <img class="avatar lg" src={"data:image/jpeg;base64," + livery.icon} alt="" />
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

            <section class="set-section">
              <h4 class="members-h4">Members &amp; roles</h4>
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
                    {#if canModerate}
                      <button class="ghost small" onclick={() => (badgeEditFp === m.fingerprint ? (badgeEditFp = "") : openBadgeEditor(m.fingerprint))}>
                        {badges[m.fingerprint] ? "Edit badge" : "Badge…"}
                      </button>
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
              <p class="muted small">
                The owner is the founder (the MLS committer). Member removal is owner-only and
                protocol-enforced. Admins can invite newcomers: the owner serializes each
                admission, so it completes when the owner is next online: and a demotion is
                replay-proof (a removed admin can't re-grant itself).
              </p>
              {#if myRole === "owner" && Object.keys(deviceMap).length}
                <h3 style="margin-top:10px"><span>Linked devices</span></h3>
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
              {/if}
              {#if myRole !== "owner"}
                <p class="muted small">Only the owner can change roles.</p>
              {/if}
            </section>

            {#if cur?.invite || canInvite}
              <section class="set-section">
                <h3>Invite someone</h3>
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
            {/if}

            <section class="set-section">
              <h3>Custom emoji</h3>
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

            {#if activeServerId !== null}
              <section class="set-section danger">
                <button class="ghost leave" onclick={() => { const id = activeServerId; showServerSettings = false; if (id !== null) leaveServer(id); }}>
                  Leave this server
                </button>
              </section>
            {/if}
          </div>
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
              <dt>Address</dt>
              <dd class="cid" title={fileInfo.cid}>{fileInfo.cid.slice(0, 16)}…</dd>
            </dl>

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
            <label class="fb-label" for="fb-text">
              {feedbackKind === "bug"
                ? "What went wrong? Steps to reproduce, and what you expected to happen."
                : "What would you like CatComs to do?"}
            </label>
            <textarea id="fb-text" class="fb-text" bind:value={feedbackText} rows="7" placeholder="Describe it here…"></textarea>
            <p class="muted small">
              CatComs is peer-to-peer with no servers, so feedback can't be sent automatically. Copy the report and
              share it with the maintainer (your issue tracker, email, or chat). Your environment is included to help debugging.
            </p>
            <div class="file-info-actions">
              <button class="primary" disabled={!feedbackText.trim()} onclick={copyFeedback}>
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
            <p>Wiki pages are written in <strong>Markdown</strong> and rendered in Read mode.</p>
            <h3>Link to another page</h3>
            <p>Wrap a page name in double brackets: <code>[[Getting Started]]</code>. Click a link to open it; a
              <span class="wikilink missing">red link</span> means the page doesn't exist yet: click it to create it.</p>
            <h3>Embed an image / video / audio</h3>
            <p>In Edit mode, <strong>drag a file onto the editor</strong> or use the 📎 button. It's stored in the
              fileshare under <code>wiki/&lt;page&gt;/</code> and shown inline.</p>
            <h3>Common Markdown</h3>
            <ul>
              <li><code>**bold**</code>, <code>*italic*</code>, <code>`code`</code></li>
              <li><code># Heading</code>, <code>## Subheading</code></li>
              <li><code>- bullet</code> lists, <code>1. numbered</code> lists</li>
              <li><code>&gt; quote</code>, <code>---</code> divider, <code>[text](https://link)</code></li>
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
