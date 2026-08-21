<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { onMount, tick, untrack } from "svelte";
  import { renderMessage, renderTextDocument, renderWiki, parseRedirect, tocDirective } from "./render";
  import {
    TEXT_PREVIEW_MAX_BYTES, decodeTextFile, lineCountLabel, textFileKind,
    type TextFileKind,
  } from "./textfile";
  import {
    MAX_SPEAKESE_BLIPS, SPEAKESE_STEP_SECONDS, TEXT_EFFECTS, TEXT_EFFECT_GROUPS,
    cherryBlossomShouldBurst, dismissTextEffectPalette, insertTextEffect, redTruthNoiseSample,
    redTruthSoundPlan, speakeseSoundPlan, textEffectHtml,
    type TextEffectPointerRegion,
  } from "./message-effects";
  import {
    DEFAULT_TEXT_EFFECT_KEYBINDS, effectForKeybind, keybindConflict, keybindFromEvent,
    sanitizeTextEffectKeybinds,
  } from "./text-effect-keybinds";
  import {
    CHAT_MESSAGE_FRAMES_ENABLED, DEFAULT_MESSAGE_FRAME, defaultMessageFrameLayer, encodeMessageFrame,
    messageFrameArrivalStyle, messageFrameLayerStyle,
    messageFramePosition, messageFrameScanGeometry, messageFrameStyle, parseMessageFrame, visibleMessageFrameMotion,
    visibleMessageFrameStyle, type MessageFrame, type MessageFrameArrival,
    type MessageFrameEasing, type MessageFrameEffectId, type MessageFrameEffectOptions,
    type MessageFrameMotion, type MessageFrameShape,
  } from "./message-frame";
  import {
    CHAT_INITIAL_ROWS, CHAT_WINDOW_STEP, CoalescedAsyncRefresh, SanitizedMessageCache, initialChatWindow,
    nearScrollBottom, reconcileChatWindow, revealNewer, revealOlder, windowAround,
    type ChatWindow,
  } from "./chat-performance";
  import { chatScopeKey, reconcileActiveChannel, scopeHoldsConversation } from "./chatscope";
  import {
    WIKI_REVIEW_UNKNOWN,
    mayEditWikiStructure,
    mayPublishLivery,
    moderationSurfaceOpen,
    scopeCurrent,
  } from "./viewscope";
  import { pastedImageUrl, safeRemoteUrl } from "./remote-media";
  import { scheduleNewsChime } from "./news-chime";
  import { acceptTickerReceipt, messageTickerId } from "./ticker";
  import {
    MAX_CUSTOM_TONE_BYTES, MAX_CUSTOM_TONE_SECONDS, NOTIFICATION_SOUND_KINDS,
    customToneError, customToneMime, defaultGlobalSoundPrefs, defaultServerSoundPrefs,
    parseGlobalSoundPrefs, parseServerSoundPrefs, resolveNotificationSound,
    type GlobalSoundPrefs, type NotificationSoundKind, type ServerSoundPrefs, type SoundOverride,
    type StoredTone, type ToneOverride,
  } from "./notification-sounds";
  import {
    completedDownload, downloadSavedNotice, guideSavedNotice, saveGroupDownload, saveSpaceGuide,
  } from "./native-download";
  import { bufferIce, heartbeatRecovery, isCurrentVoiceRoom } from "./voice-signaling";
  import {
    driftAction, fetchPhase, mediaKind, mediaUrl, nudgeRate,
    type FetchPhase, type MediaKind,
  } from "./jukebox";
  import {
    TRANSFER_CHUNK_BYTES, formatBytes, formatRate, sampleRate, transferPieces,
    type TransferPiece,
  } from "./transfer-visual";
  import { plainSummary } from "./wikitext";
  import { refLabel, fileMarker, statusMarker, wikiMarker, eventMarker, insertInto } from "./refs";
  import { buildWikiTree, visibleRows, ancestorsOf } from "./wikitree";
  import { extractInfobox, infoboxTemplate } from "./infobox";
  import {
    type Connectivity, type JoinAttempt, describeOutcome, formatConnectivity, formatJoinLog,
    automaticMappingUnavailable, connectivityReadout, connectivityStatus,
    reachabilityEventAffectsReport, reachabilitySummary,
    switchboardEventRefreshDecision, withOrderedConnectivity, withOrderedRefreshedInvite,
  } from "./joinlog";
  import { diffLines, diffStats, type DiffLine } from "./linediff";
  import {
    buildModerationGraph, buildModerationTimeline, filterModerationTimeline, openKickCases,
    selectTimelineRows, timelineIdentities, voteTally, warningMap,
    type ModerationEvent, type ModerationState, type TimelineMessage,
  } from "./moderation";
  import { planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity";
  import {
    type NameEffect, type NameEffectId, type NameEffectOptions, animatedEffect,
    decodeNameEffects, defaultNameEffect, effectConfigured, effectEnabled, effectOptions, encodeNameEffects,
    nameEffectClasses, nameEffectStyle,
  } from "./name-effects";
  import {
    type Placement, type ScreenPoint, type SpaceCluster, type SpaceState, angularOffsets,
    applyOffsets, autoArrangePlacements, clampPitch, defaultSpace, lassoCapturePath, parseSpace,
    placementCentre, project, separatePlacements, unproject, wrapYaw, yawDelta,
  } from "./space";
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
    MIDI_FIXES, MIDI_SETUP_STEPS, describeMidiMessage, deviceRows, isMonitorWorthy, isPortRouted,
    midiPortLabel, midiStatus, newMidiRouter, parseMidiMessage, pushMonitorLine, releaseAllNotes,
    routeMidi, routedDevices,
    type MidiDeviceRow, type MidiMonitorLine, type MidiPermission,
  } from "./midi";
  import {
    SIGIL_VIEW, SIGIL_C, R_INNER, R_OUTER, R_TEXT, R_EMOJI, NODE_R, LATTICE, nodeLabel, hitNode,
    appendHit, classifyGesture, encodeSigil, encodeSigilPath, segmentCount,
    sigilBits as sigilBitsOf, normalizeWord, MAX_SIGIL_EMOJI, SIGIL_COLORS, COLOR_NAMES,
    coloredCount, ringGlyphs, ringPoints, ringPathD,
  } from "./sigil";
  import {
    assistedJoinAction, joinReplyCandidateLabel, joinReplyIsExpired, joinReplyNeedsReplacement,
    withOrderedSwitchboardStatus,
  } from "./joinreply";

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
  type Found = { server: number; channel: string; channels?: Channel[]; is_dm: boolean };
  type Reloaded = { server: number; name: string; invite: string; channel: string; channels?: Channel[]; is_dm: boolean };

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
  // Bumped by every move that changes WHICH group the panes show. An async refresh captures it
  // on entry and re-checks after its awaits: a slow answer from the group you just left must
  // never paint over the one you moved to. Channel-scoped reads pair this with the channel id,
  // which switchTo changes without changing the group.
  let viewGeneration = 0;
  // True from the moment a group's panes are emptied until its first batch of reads lands. Panes
  // whose empty state makes a CLAIM ("No messages yet", "No matching members") must consult this:
  // an empty collection mid-switch means "not read yet", not "there is nothing".
  let groupLoading = $state(false);
  function beginViewSwitch(): number {
    return ++viewGeneration;
  }
  // Is the captured context still the one on screen? Both halves matter: the generation catches
  // a move away, and the server id catches a read issued for a different group at the same
  // generation (event-driven refreshes do not bump anything).
  function viewCurrent(gen: number, server: number | null): boolean {
    return scopeCurrent({ generation: gen, server }, { generation: viewGeneration, server: activeServerId });
  }
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
  let backupBusy = $state(false);
  let backupResult = $state<{ path: string; files: number; bytes: number; displayed: boolean; warning?: string } | null>(null);
  async function createBackup() {
    backupBusy = true;
    backupResult = null;
    try {
      backupResult = await invoke("create_backup");
    } catch (e) {
      error = String(e);
    } finally {
      backupBusy = false;
    }
  }
  type SetPage = { id: string; label: string; cat: string; danger?: boolean };
  const USER_SET_PAGES: SetPage[] = [
    { id: "guide", label: "Feature Guide", cat: "Help" },
    { id: "profile", label: "My Profile", cat: "Account" },
    { id: "devices", label: "Devices", cat: "Account" },
    { id: "vault", label: "Vault & Lock", cat: "Account" },
    { id: "backup", label: "Backup & Recovery", cat: "Account" },
    { id: "verify", label: "Verification", cat: "Account" },
    { id: "appearance", label: "Appearance", cat: "App" },
    { id: "space", label: "Server Space", cat: "App" },
    { id: "notifications", label: "Notifications", cat: "App" },
    { id: "voice", label: "Voice & Calls", cat: "App" },
    { id: "chatmedia", label: "Chat & Media", cat: "App" },
    { id: "keybinds", label: "Keybinds", cat: "App" },
    { id: "network", label: "Network", cat: "Connection" },
    { id: "diagnostics", label: "Diagnostics", cat: "Connection" },
    { id: "updates", label: "Updates", cat: "Connection" },
  ];
  const SRV_SET_PAGES: SetPage[] = [
    { id: "overview", label: "Overview", cat: "Overview" },
    { id: "notifications", label: "Notifications", cat: "Overview" },
    { id: "livery", label: "Livery", cat: "Overview" },
    { id: "members", label: "Members", cat: "People" },
    { id: "badges", label: "Badges", cat: "People" },
    { id: "sdevices", label: "Devices", cat: "People" },
    { id: "invites", label: "Invites", cat: "People" },
    { id: "joinlog", label: "Join Log", cat: "People" },
    { id: "emoji", label: "Emoji & Stickers", cat: "Content" },
    { id: "calls", label: "Calls & Relay", cat: "Voice" },
    { id: "leave", label: "Leave Server", cat: "Danger", danger: true },
  ];
  type FeatureTarget =
    | "dms"
    | "feedback"
    | "inbox"
    | "news"
    | "quick"
    | "space"
    | `surface:${"chat" | "files" | "status" | "wiki" | "profile" | "downloads" | "events" | "moderation" | "storage" | "connectivity"}`
    | `settings:${string}`
    | `server:${string}`;
  type FeatureGuideItem = {
    group: string;
    title: string;
    detail: string;
    where: string;
    shortcut?: string;
    target?: FeatureTarget;
  };
  const FEATURE_GUIDE_GROUPS = [
    "Conversation",
    "Knowledge & media",
    "People & trust",
    "Voice & play",
    "Community",
    "App & network",
  ];
  const FEATURE_GUIDE: FeatureGuideItem[] = [
    { group: "Conversation", title: "Channels & chat", detail: "Markdown messages, replies, reactions, editing, pins, mentions, unread marks and delivery evidence.", where: "Open a server → Chat", shortcut: "Ctrl+1", target: "surface:chat" },
    { group: "Conversation", title: "Search", detail: "Search one channel or the whole server; filter by people, dates, media, replies, reactions and more.", where: "Chat → magnifier, or press Ctrl+F", shortcut: "Ctrl+Shift+F", target: "surface:chat" },
    { group: "Conversation", title: "DMs & friends", detail: "Private 1:1 spaces, friend codes and authenticated in-server friend requests.", where: "Left rail → DMs", target: "dms" },
    { group: "Conversation", title: "Inbox", detail: "Mentions and replies gathered across every server and DM, with one-click jumps to the message.", where: "Left rail → Inbox", target: "inbox" },
    { group: "Knowledge & media", title: "Files & folders", detail: "Encrypted sharing, folders, previews, deduplication, circulation controls and usage tracking.", where: "Open a server → Files", shortcut: "Ctrl+2", target: "surface:files" },
    { group: "Knowledge & media", title: "Transfers", detail: "Track uploads and downloads, progress, availability and the peer serving a download.", where: "Open a server → Transfers", shortcut: "Ctrl+6", target: "surface:downloads" },
    { group: "Knowledge & media", title: "Storage health & repair", detail: "Save one integrity/inventory report per server session, inspect categories, pins and largest files, then explicitly re-fetch missing or unreadable data from authenticated peers.", where: "Server sidebar → Storage, or Transfers", target: "surface:storage" },
    { group: "Knowledge & media", title: "Durable history UX", detail: "Channel drafts and read positions survive restarts inside the encrypted vault, alongside the server's replicated history.", where: "Chat: automatic after unlock", target: "surface:chat" },
    { group: "Knowledge & media", title: "Wiki", detail: "Markdown or Wikitext pages, nested pages, backlinks, infoboxes, history, rollback and optional edit review.", where: "Open a server → Wiki", shortcut: "Ctrl+4", target: "surface:wiki" },
    { group: "Knowledge & media", title: "Announcements", detail: "Post short server announcements with the same rich text and media embeds as chat.", where: "Open a server → Announcements", shortcut: "Ctrl+3", target: "surface:status" },
    { group: "Knowledge & media", title: "News", detail: "Read recent announcements and upcoming events from every server in one feed.", where: "Left rail → Inbox → News", target: "news" },
    { group: "Knowledge & media", title: "Events", detail: "Create shared events with times, descriptions and optional artwork.", where: "Open a server → Events", shortcut: "Ctrl+7", target: "surface:events" },
    { group: "People & trust", title: "Profiles", detail: "Per-server names, bios, banners, animated avatars, plus studios for name effects, joined message frames and new-message motion.", where: "Profile, or Settings → My Profile", shortcut: "Ctrl+5", target: "settings:profile" },
    { group: "People & trust", title: "Identity verification", detail: "Compare cryptographic fingerprints out of band and keep a private verified mark.", where: "Settings → Verification", target: "settings:verify" },
    { group: "People & trust", title: "Linked devices", detail: "Grant another device access with a one-time ceremony carried by paste, QR or sound; revoke companions per server.", where: "Settings → Devices", target: "settings:devices" },
    { group: "People & trust", title: "Vault & lock", detail: "Lock the visible session, use a passphrase, sigil or played melody, and atomically change that local vault secret after authenticating the current one.", where: "Settings → Vault & Lock", shortcut: "Ctrl+L", target: "settings:vault" },
    { group: "Voice & play", title: "Voice, camera & screen share", detail: "Join a channel voice room, switch devices live, share a camera or screen and control each peer locally.", where: "Chat channel header → Join voice", target: "surface:chat" },
    { group: "Voice & play", title: "Instruments & jukebox", detail: "Play the call-stage instrument from screen, keyboard or MIDI and queue shared server audio for the room.", where: "Voice stage → Instruments / Jukebox", target: "surface:chat" },
    { group: "Voice & play", title: "MIDI controllers", detail: "Connect a music keyboard for the melody lock and the call instrument: live device list, per-port routing, a message monitor and setup help.", where: "Settings → Devices", target: "settings:devices" },
    { group: "Community", title: "Members, roles & badges", detail: "Inspect presence and devices; owners manage admins and removals, while moderators assign display badges.", where: "Right-click server → Server settings → Members / Badges", target: "server:members" },
    { group: "Community", title: "Moderation plane", detail: "Owners/admins inspect a per-user activity graph and signed detail timeline, issue warnings, and build evidence-backed kick cases; members vote from focused chat cards.", where: "Owner/admin: server sidebar → Moderation", target: "surface:moderation" },
    { group: "Community", title: "Invites", detail: "Generate single-use, device-bound invites; admin admissions are serialized by the owner to avoid group forks.", where: "Right-click server → Server settings → Invites", target: "server:invites" },
    { group: "Community", title: "Emoji & stickers", detail: "Upload server emoji at emoji, medium, large or sticker size and use them in messages or reactions.", where: "Server settings → Emoji & Stickers", target: "server:emoji" },
    { group: "Community", title: "Server livery", detail: "Publish a safe shared palette, icon, cursor, typography and background treatment; every member can opt out.", where: "Server settings → Livery", target: "server:livery" },
    { group: "App & network", title: "Quick switcher", detail: "Jump to channels, surfaces, servers and DMs without hunting through the rails.", where: "Anywhere in the unlocked app", shortcut: "Ctrl+K", target: "quick" },
    { group: "App & network", title: "Server Space", detail: "Arrange servers in a navigable 360-degree room; group them into interactive neighbourhoods, search, auto-arrange, or use a custom backdrop.", where: "Left rail → Orbit", shortcut: "Ctrl+O", target: "space" },
    { group: "App & network", title: "Appearance", detail: "Themes, accent, density, text scale, clock style, reduced motion, and local opt-outs for shared livery, message frames and arrivals.", where: "Settings → Appearance", target: "settings:appearance" },
    { group: "App & network", title: "Connectivity & diagnostics", detail: "Configure rendezvous defaults, inspect the latest connection attempt and opt into a privacy-labelled debug log.", where: "Settings → Network / Diagnostics", target: "settings:diagnostics" },
    { group: "App & network", title: "Connectivity assistant", detail: "See honest three-state connection evidence, live peer counts and concrete recovery suggestions without claiming unproven internet reachability.", where: "Server sidebar → Connectivity", target: "surface:connectivity" },
    { group: "App & network", title: "Backup & recovery", detail: "Export a coherent sealed vault copy while seeing the offline-guessing, metadata and old-secret exposure tradeoffs. Automated restore remains staged follow-up work.", where: "Settings → Backup & Recovery", target: "settings:backup" },
    { group: "App & network", title: "Signed updates", detail: "Check for a newer signed release and choose whether to install, defer or skip it.", where: "Settings → Updates", target: "settings:updates" },
    { group: "App & network", title: "Feedback", detail: "Open a pre-filled bug report or feature request for review before submitting, or copy it to send another way.", where: "Left rail → Feedback", target: "feedback" },
  ];
  let featureQuery = $state("");
  let filteredFeatures = $derived.by(() => {
    const q = featureQuery.trim().toLowerCase();
    if (!q) return FEATURE_GUIDE;
    return FEATURE_GUIDE.filter((item) =>
      `${item.group} ${item.title} ${item.detail} ${item.where} ${item.shortcut ?? ""}`.toLowerCase().includes(q),
    );
  });

  function openFeatureTarget(target: FeatureTarget) {
    if (target.startsWith("settings:")) {
      settingsPage = target.slice("settings:".length);
      setSearch = "";
      return;
    }
    if (target.startsWith("server:")) {
      if (!cur || cur.isDm || activeServerId === null) {
        toast("Open a server first to use its settings", "info", 3500);
        return;
      }
      showSettings = false;
      void openServerSettings(null, target.slice("server:".length));
      return;
    }
    if (target.startsWith("surface:")) {
      if (activeServerId === null) {
        toast("Open a server or DM first", "info", 3000);
        return;
      }
      if (target === "surface:moderation" && !canModerate) {
        toast("Moderation is available to this server's owner and admins", "info", 3500);
        return;
      }
      showSettings = false;
      switchView(target.slice("surface:".length) as Tab);
      return;
    }
    showSettings = false;
    if (target === "dms") enterDmHome();
    else if (target === "inbox") openInbox();
    else if (target === "news") {
      openInbox();
      inboxMode = "news";
      loadNews();
    } else if (target === "quick") openQuickSwitch();
    else if (target === "space" && !spaceOpen) toggleSpace();
    else if (target === "feedback") showFeedback = true;
  }
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
    const targetServer = id ?? activeServerId;
    // switchServer runs synchronously as far as its first await, so `cur` below already names the
    // target. It also empties `livery` on the way past and refills it a round-trip later, which is
    // why the editor draft is seeded from liveryLoaded rather than from `livery` directly.
    if (id !== null && id !== activeServerId) void switchServer(id);
    serverNameDraft = cur?.name ?? "";
    serverSettingsPage = page;
    setSearch = "";
    showServerSettings = true;
    // The draft never carries the images: set_livery ignores them (set_server_icon /
    // set_server_cursor own those fields). It is seeded ONLY from a livery we have read: the
    // wrench is reachable mid-switch, and seeding from an unread livery would present the default
    // theme as this server's own, one Publish away from erasing the real one for every member.
    liveryDraft = emptyLivery();
    liveryDraftFor = null;
    if (liveryLoaded && targetServer !== null) seedLiveryDraft(targetServer);
    if (page === "invites" && targetServer !== null) void refreshInviteFor(targetServer);
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
  type FeedbackOverlayComponent = (typeof import("./FeedbackOverlay.svelte"))["default"];
  let FeedbackOverlay = $state<FeedbackOverlayComponent | null>(null);
  let feedbackOverlayLoading = false;
  let feedbackOverlayError = $state("");
  async function loadFeedbackOverlay() {
    if (FeedbackOverlay || feedbackOverlayLoading) return;
    feedbackOverlayLoading = true;
    feedbackOverlayError = "";
    try {
      FeedbackOverlay = (await import("./FeedbackOverlay.svelte")).default;
    } catch (cause) {
      feedbackOverlayError = String(cause);
    } finally {
      feedbackOverlayLoading = false;
    }
  }
  $effect(() => {
    if (showFeedback && !FeedbackOverlay) void loadFeedbackOverlay();
  });
  // Notification sound policy is entirely local. The master preserves the original one-switch
  // behavior; category defaults and per-server overrides refine it without publishing preferences
  // to peers or changing any server document.
  let soundOn = $state(typeof localStorage !== "undefined" ? localStorage.getItem("catcoms.sound") !== "off" : true);
  const GLOBAL_SOUND_PREFS_KEY = "catcoms.sound.preferences.v1";
  const serverSoundPrefsKey = (server: number) => `catcoms.sound.server.${server}.v1`;
  function loadGlobalSoundPrefs(): GlobalSoundPrefs {
    try { return parseGlobalSoundPrefs(localStorage.getItem(GLOBAL_SOUND_PREFS_KEY)); }
    catch { return defaultGlobalSoundPrefs(); }
  }
  function readServerSoundPrefs(server: number): ServerSoundPrefs {
    try { return parseServerSoundPrefs(localStorage.getItem(serverSoundPrefsKey(server))); }
    catch { return defaultServerSoundPrefs(); }
  }
  let globalSoundPrefs = $state<GlobalSoundPrefs>(loadGlobalSoundPrefs());
  let serverSoundPrefs = $state<ServerSoundPrefs>(defaultServerSoundPrefs());
  const SOUND_LABELS: Record<NotificationSoundKind, { title: string; detail: string }> = {
    message: { title: "Messages", detail: "Ordinary message notifications" },
    mention: { title: "Mentions & replies", detail: "Messages specifically aimed at you" },
    news: { title: "News ticker", detail: "Announcement, wiki, event, and ticker headline cue" },
  };
  function toggleSound() {
    soundOn = !soundOn;
    try { localStorage.setItem("catcoms.sound", soundOn ? "on" : "off"); } catch { /* ignore */ }
  }
  function saveGlobalSoundPrefs() {
    try { localStorage.setItem(GLOBAL_SOUND_PREFS_KEY, JSON.stringify(globalSoundPrefs)); }
    catch { toast("Could not save sound settings: custom tones may be too large", "err", 4500); }
  }
  function loadServerSoundPreferences(server: number | null) {
    serverSoundPrefs = server === null ? defaultServerSoundPrefs() : readServerSoundPrefs(server);
  }
  function saveServerSoundPrefs() {
    if (activeServerId === null) return;
    try { localStorage.setItem(serverSoundPrefsKey(activeServerId), JSON.stringify(serverSoundPrefs)); }
    catch { toast("Could not save this server's sound settings", "err", 4500); }
  }
  function setGlobalSoundEnabled(kind: NotificationSoundKind, enabled: boolean) {
    globalSoundPrefs[kind].enabled = enabled;
    saveGlobalSoundPrefs();
  }
  function setGlobalToneMode(kind: NotificationSoundKind, tone: "default" | "custom") {
    if (tone === "custom" && !globalSoundPrefs[kind].custom) return;
    globalSoundPrefs[kind].tone = tone;
    saveGlobalSoundPrefs();
  }
  function setServerSoundEnabled(kind: NotificationSoundKind, enabled: SoundOverride) {
    serverSoundPrefs[kind].enabled = enabled;
    saveServerSoundPrefs();
  }
  function setServerToneMode(kind: NotificationSoundKind, tone: ToneOverride) {
    if (tone === "custom" && !serverSoundPrefs[kind].custom) return;
    serverSoundPrefs[kind].tone = tone;
    saveServerSoundPrefs();
  }
  function soundPolicy(kind: NotificationSoundKind, server: number | null) {
    const local = server === null
      ? null
      : server === activeServerId
        ? serverSoundPrefs
        : readServerSoundPrefs(server);
    return resolveNotificationSound(soundOn, globalSoundPrefs, local, kind);
  }

  // Appearance: the whole theme is a token map in app.css; these choices only flip
  // data-attributes / one CSS variable on <html>, so they can never fork the layout.
  // Semantic colours (green=presence, gold=mentions, red=danger) are constant in every preset.
  type Appearance = { preset: string; accent: string; density: string; chrome: string; flat: boolean; icons: string; motion: string; messageMotion: string; textEffects: string; clock: string; scale: number };
  const APPEARANCE_KEY = "catcoms.appearance";
  // clock: "" = the locale's habit, "12"/"24" force a convention. scale: whole-interface text
  // size in percent; clamped where applied, not where stored.
  const APPEARANCE_DEFAULT: Appearance = { preset: "", accent: "", density: "", chrome: "terminal", flat: false, icons: "", motion: "", messageMotion: "", textEffects: "", clock: "", scale: 100 };
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
  // Whether `livery` holds an answer we actually read for the active server, as opposed to the
  // empty value a switch leaves behind. An empty livery is byte-for-byte the payload that REMOVES
  // a server's branding for every member, so nothing may publish a draft without this.
  let liveryLoaded = $state(false);
  let liveryDraftFor = $state<number | null>(null); // which server liveryDraft was seeded from
  // Each server's published branding, remembered for the session: a revisit repaints once, to the
  // right brand, instead of default-then-brand a round-trip later.
  const liveryCache = new Map<number, Livery>();
  // Fill an unseeded livery editor from the loaded livery. Never overwrites a seeded draft: once
  // the editor has this server's values in it, the buffer belongs to the user.
  function seedLiveryDraft(server: number) {
    if (!showServerSettings || liveryDraftFor === server) return;
    liveryDraft = { preset: livery.preset, accent: livery.accent, tokens: { ...livery.tokens }, icon: "", cursor: "" };
    liveryDraftFor = server;
  }
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
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null || cur?.isDm) {
      livery = emptyLivery();
      liveryCursorUrl = "";
      liveryLoaded = server !== null; // a DM has no livery, and that is a read answer
      return;
    }
    try {
      const next = sanitizeLivery(await invoke<Livery>("get_livery", { server }));
      if (!viewCurrent(gen, server)) return; // a late theme repaints the app in the wrong brand
      // The brand lands before the cursor image decodes. Waiting on the decode would leave the app
      // unbranded for its duration, and a decode that never settles would hang the whole switch.
      liveryCache.set(server, next);
      livery = next;
      liveryLoaded = true;
      seedLiveryDraft(server);
      // Deliberately not awaited. The cursor is decoration that arrives when it arrives, whereas
      // this function sits inside the switch barrier: validateCursor resolves an image with no
      // timeout, so awaiting it would let one undecodable cursor hold the entire switch open.
      void validateCursor(next.cursor).then((cursor) => {
        if (viewCurrent(gen, server)) liveryCursorUrl = cursor;
      });
    } catch {
      if (!viewCurrent(gen, server)) return;
      livery = emptyLivery(); // failed/malformed reads degrade to "no livery", never an error
      liveryCursorUrl = "";
      liveryLoaded = false; // a failed read is not "this server has no theme": refuse to publish
    }
  }
  async function publishLivery() {
    if (activeServerId === null) return;
    if (!mayPublishLivery(liveryLoaded, liveryDraftFor, activeServerId)) {
      toast("Still reading this server's theme: try again in a moment", "info", 3000);
      return;
    }
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
    const reduced = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
    const textEffects = appearance.textEffects === "off"
      ? "off"
      : appearance.textEffects === "low" || reduced ? "low" : "full";
    set("text-effects", textEffects);
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
    // Scale the root rather than one chat token: menus, member names, dialogs and every other
    // rem-sized label now follow the same accessibility preference.
    const scale = Math.min(200, Math.max(70, appearance.scale || 100));
    if (scale === 100) el.style.removeProperty("font-size");
    else el.style.fontSize = `${scale}%`;
    try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(appearance)); } catch { /* best-effort */ }
  });

  // Persistence (9f): a passphrase gate. On launch the app is locked until the user enters
  // their passphrase, which unlocks the on-disk vault and reloads their servers (or, on
  // first run, sets the passphrase and starts empty).
  let locked = $state(true);
  let passphrase = $state("");
  let unlocking = $state(false);
  type VaultChangeStep = "" | "current" | "new" | "confirm";
  let vaultChangeStep = $state<VaultChangeStep>("");
  let vaultChangeCurrent = $state("");
  let vaultChangeFirst = $state("");
  let vaultChangeMismatch = $state(false);
  let vaultChangeBusy = $state(false);
  let vaultChangeError = $state("");
  let changingVaultSecret = $derived(vaultChangeStep !== "");

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
  let gateEntry = $derived((locked || changingVaultSecret) && (!inSetup || setupStep === "secret" || setupStep === "confirm"));

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
  function beginVaultSecretChange() {
    showSettings = false;
    error = "";
    vaultChangeCurrent = "";
    vaultChangeFirst = "";
    vaultChangeMismatch = false;
    vaultChangeError = "";
    vaultChangeStep = "current";
    clearUnlockEntry();
  }
  function cancelVaultSecretChange() {
    vaultChangeCurrent = "";
    vaultChangeFirst = "";
    vaultChangeMismatch = false;
    vaultChangeError = "";
    vaultChangeStep = "";
    clearUnlockEntry();
    settingsPage = "vault";
    showSettings = true;
  }
  async function submitVaultSecretChange() {
    const secret = unlockSecret();
    if (!secret || vaultChangeBusy) return;
    vaultChangeError = "";
    if (vaultChangeStep === "current") {
      vaultChangeCurrent = secret;
      vaultChangeStep = "new";
      clearUnlockEntry();
      return;
    }
    if (vaultChangeStep === "new") {
      if (secret === vaultChangeCurrent) {
        vaultChangeError = "Choose a different secret from the current one.";
        clearUnlockEntry();
        return;
      }
      vaultChangeFirst = secret;
      vaultChangeMismatch = false;
      vaultChangeStep = "confirm";
      clearUnlockEntry();
      return;
    }
    if (vaultChangeStep !== "confirm") return;
    if (secret !== vaultChangeFirst) {
      vaultChangeMismatch = true;
      clearUnlockEntry();
      return;
    }
    vaultChangeBusy = true;
    try {
      await invoke("change_vault_secret", {
        currentSecret: vaultChangeCurrent,
        newSecret: vaultChangeFirst,
      });
      vaultChangeCurrent = "";
      vaultChangeFirst = "";
      vaultChangeMismatch = false;
      vaultChangeStep = "";
      clearUnlockEntry();
      settingsPage = "vault";
      showSettings = true;
      toast("Vault secret changed. Existing backups still use their old secret.", "ok", 6500);
    } catch (e) {
      // A wrong current secret is intentionally indistinguishable from a damaged wrapper here.
      // Drop both transient strings and restart authentication; never leave them in the form.
      vaultChangeCurrent = "";
      vaultChangeFirst = "";
      vaultChangeMismatch = false;
      vaultChangeError = `The vault secret was not changed: ${e}`;
      vaultChangeStep = "current";
      clearUnlockEntry();
    } finally {
      vaultChangeBusy = false;
    }
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
  // Web MIDI (Chromium/WebView2): a connected controller feeds the same note handlers the
  // on-screen keys do, so you can literally play your unlock tune, or the call instrument.
  // Feature-detected; absent or refused is survivable and Settings says which it was.
  //
  // Everything below exists because "I plugged it in and nothing happened" was the old failure
  // mode, and it had four separate causes:
  //   * the request was one-shot. A dismissed prompt, a driver still enumerating, or a keyboard
  //     plugged in a minute after launch left the whole session deaf until the app restarted.
  //   * only the LAST input in the map kept a handler. Controllers routinely publish two or three
  //     ports and the keys come out of exactly one of them, so it was a coin toss.
  //   * ports reporting "disconnected" counted as the connected device, so the badge could name
  //     hardware that had already been unplugged.
  //   * nothing was recorded when no surface wanted the notes, so there was no way to tell
  //     "not listening" apart from "listening, and the controller is silent".
  // Parsing and routing themselves live in midi.ts so they can be tested; this half is the
  // browser plumbing and the surface routing.
  const MIDI_INPUT_KEY = "catcoms.midi.input";
  let midiSupported = $state(typeof navigator !== "undefined" && "requestMIDIAccess" in navigator);
  let midiAccess: MIDIAccess | null = null;
  let midiRequested = $state(false); // access has been asked for at least once this session
  let midiBusy = $state(false);
  let midiFailure = $state(""); // why the last request was rejected, "" when it was not
  let midiPermission = $state<MidiPermission>("unknown");
  let midiDevices = $state<MidiDeviceRow[]>([]);
  // "" routes every connected input, which is right for almost everyone. Pinning one port is the
  // escape hatch for a controller whose second or third port is the one carrying the keys.
  let midiInput = $state(
    typeof localStorage !== "undefined" ? localStorage.getItem(MIDI_INPUT_KEY) ?? "" : "",
  );
  let midiMonitor = $state<MidiMonitorLine[]>([]); // last few messages, for the Settings monitor
  let midiLastAt = $state(0); // when anything at all last arrived
  let midiRealtime = $state(0); // clock/sensing packets: a live cable that is sending no notes
  let midiSeq = 0;
  const midiRouter = newMidiRouter();
  let midiStat = $derived(
    midiStatus({
      supported: midiSupported,
      requested: midiRequested,
      busy: midiBusy,
      failure: midiFailure,
      permission: midiPermission,
      devices: midiDevices,
    }),
  );
  // The drawer and lock badges want one name: the first input actually allowed to play.
  let midiName = $derived(routedDevices(midiDevices)[0]?.label ?? "");

  /** Which surface MIDI notes belong to right now, or "" when nothing is listening for them. */
  function midiTarget(): "melody" | "instrument" | "" {
    if (locked && unlockMethod === "melody") return "melody";
    if (inCall && instOpen) return "instrument";
    return "";
  }
  function onMidiMessage(port: MIDIInput, event: MIDIMessageEvent) {
    const msg = parseMidiMessage(event.data);
    if (!msg) return;
    const routed = isPortRouted(port, midiInput);
    midiLastAt = Date.now();
    if (isMonitorWorthy(msg)) {
      midiMonitor = pushMonitorLine(midiMonitor, {
        seq: ++midiSeq,
        port: midiPortLabel(port),
        text: describeMidiMessage(msg),
        routed,
      });
    } else {
      midiRealtime++; // proof of life for a cable whose keys are not reaching us
    }
    // Filtering happens AFTER the monitor on purpose: watching the port you did not pin light up
    // is how anyone works out which of a controller's ports carries the keys. It is applied here
    // rather than by leaving that port unwired, so it stays listed, keeps proving itself, and can
    // be switched to without a rescan. The router only ever sees the routed port, so a port being
    // ignored cannot corrupt its held-note bookkeeping.
    if (!routed) return;
    // The router is fed even when no surface wants the notes, so held state stays truthful across
    // a surface change. Sustain is enabled ONLY for the call instrument: see routeMidi, where the
    // melody lock's secret would otherwise change under a held pedal.
    const target = midiTarget();
    for (const { note, on } of routeMidi(midiRouter, msg, target === "instrument")) {
      if (target === "melody") {
        if (on) noteOn(note);
        else noteOff(note);
      } else if (target === "instrument") {
        if (on) instNoteOn(note);
        else instNoteOff(note);
      }
    }
  }
  /** Lift everything MIDI believes is sounding: an unplug, a surface change, or the panic button. */
  function releaseMidiNotes() {
    const target = midiTarget();
    for (const { note } of releaseAllNotes(midiRouter)) {
      if (target === "melody") noteOff(note);
      else if (target === "instrument") instNoteOff(note);
    }
  }
  /** Wire every input we can see and rebuild the device list. Safe to call as often as we like. */
  function wireMidi() {
    const access = midiAccess;
    if (!access) {
      midiDevices = [];
      return;
    }
    const wasConnected = new Set(midiDevices.filter((d) => d.connected).map((d) => d.id));
    for (const input of access.inputs.values()) {
      // Assigning the handler implicitly opens the port. A port whose device is currently absent
      // stays pending and starts delivering by itself the moment that device returns, which is
      // what makes replugging work without a rescan.
      input.onmidimessage = (event: MIDIMessageEvent) => onMidiMessage(input, event);
    }
    midiDevices = deviceRows(access.inputs.values(), midiInput);
    // A controller yanked mid-note never gets to send its note-offs. Lift them here rather than
    // leaving a tone sounding until something else happens to clear it.
    const lost = [...wasConnected].some((id) => !midiDevices.some((d) => d.connected && d.id === id));
    if (lost) releaseMidiNotes();
  }
  /**
   * Ask for MIDI access, or just rescan when we already have it. Retryable on purpose: the old
   * one-shot guard is exactly why a controller connected after launch never worked.
   */
  async function initMidi(force = false): Promise<void> {
    if (!midiSupported || midiBusy) return;
    if (midiAccess && !force) {
      wireMidi();
      return;
    }
    midiBusy = true;
    midiFailure = "";
    try {
      const nav = navigator as Navigator & {
        requestMIDIAccess?: (options?: { sysex?: boolean }) => Promise<MIDIAccess>;
      };
      if (!nav.requestMIDIAccess) {
        midiSupported = false;
        return;
      }
      // sysex stays off: nothing here sends a device anything, and asking for it would turn a
      // routine permission into a much scarier one for no gain.
      const access = await nav.requestMIDIAccess({ sysex: false });
      midiAccess = access;
      access.onstatechange = () => wireMidi(); // hot-plug: ports appear and vanish under us
      wireMidi();
    } catch (e) {
      // Refused, or no MIDI subsystem at all. Deliberately not sticky: granting it later and
      // pressing Rescan has to work without restarting the app.
      midiFailure = String((e as Error)?.message ?? e);
    } finally {
      midiRequested = true;
      midiBusy = false;
      void refreshMidiPermission();
    }
  }
  /**
   * Track the permission separately from the request. It can be granted or revoked outside the
   * app, so knowing it lets the panel say "refused" instead of showing a vague failure, and lets
   * an already-granted permission wire itself up without prompting anyone.
   */
  async function refreshMidiPermission(): Promise<void> {
    try {
      const perms = navigator.permissions as Permissions | undefined;
      const status = await perms?.query({ name: "midi" } as PermissionDescriptor);
      if (!status) return;
      midiPermission = status.state as MidiPermission;
      status.onchange = () => {
        midiPermission = status.state as MidiPermission;
        if (status.state === "granted") void initMidi();
      };
    } catch {
      midiPermission = "unknown"; // the query is Chromium-only; not knowing is not a failure
    }
  }
  /**
   * Startup: if MIDI is already granted, connect without asking anyone anything. This is what
   * makes an already-plugged-in keyboard play the lock screen straight away, instead of only
   * after the instrument drawer has been opened once to trigger the old lazy request.
   */
  async function primeMidi(): Promise<void> {
    if (!midiSupported) return;
    await refreshMidiPermission();
    if (midiPermission === "granted") void initMidi();
  }
  // Opening Settings → Devices rescans an existing grant so the list is never stale by the time
  // it is looked at. It deliberately never prompts: an unasked permission stays behind the
  // explicit button, because a permission popup nobody asked for is how people end up denying it.
  // untrack because the rescan reads and rewrites `midiDevices`; without it the effect would
  // depend on its own output and re-run itself until Svelte gave up. Opening the page is the
  // trigger, nothing else.
  $effect(() => {
    if (showSettings && settingsPage === "devices") {
      untrack(() => {
        if (midiAccess) void initMidi();
      });
    }
  });
  function setMidiInput(id: string) {
    releaseMidiNotes(); // a note held on a port that is about to stop being routed would hang
    midiInput = id;
    try { localStorage.setItem(MIDI_INPUT_KEY, id); } catch { /* ignore */ }
    wireMidi(); // recompute which rows are routed
  }
  function unlockSecret(): string {
    return unlockMethod === "pass" ? passphrase : unlockMethod === "sigil" ? sigilSecret : melodySecret;
  }

  // --- M6: alternate carry channels for pairing blobs (QR + sound) -----------------------
  // Paste remains the baseline; QR and the acoustic channel are conveniences for the same
  // strings. QR fits both legs when small enough; sound is request-leg-sized only.
  const QR_MAX_CHARS = 2600; // v40-L capacity headroom; beyond this we say "use paste"
  type QrRenderer = { toCanvas: typeof import("qrcode").toCanvas };
  let qrCodeLoader: Promise<QrRenderer> | null = null;
  let qrDecoderLoader: Promise<typeof import("jsqr")> | null = null;
  function loadQrCode() {
    // QR is a pairing convenience rather than startup UI, so keep both sizeable codecs outside
    // the initial webview payload. Vite emits real async chunks for these dynamic imports.
    return (qrCodeLoader ??= import("qrcode").then((module) => {
      // qrcode uses `export =` typings while Vite's CommonJS interop supplies `default` at
      // runtime. Accept either shape so the lazy boundary works in tests and production builds.
      const compatible = module as unknown as QrRenderer & { default?: QrRenderer };
      return compatible.default ?? compatible;
    }));
  }
  function loadQrDecoder() {
    return (qrDecoderLoader ??= import("jsqr"));
  }
  // Svelte action: render `text` as a QR into the canvas (re-renders when text changes).
  function qr(canvas: HTMLCanvasElement, text: string) {
    let live = true;
    let generation = 0;
    const draw = async (t: string) => {
      const current = ++generation;
      if (!t || t.length > QR_MAX_CHARS) return;
      try {
        const QRCode = await loadQrCode();
        if (!live || current !== generation || !canvas.isConnected) return;
        await QRCode.toCanvas(canvas, t, { margin: 1, width: 220 });
      } catch {
        // Paste remains the baseline pairing path if the optional renderer cannot load.
      }
    };
    void draw(text);
    return {
      update(t: string) {
        void draw(t);
      },
      destroy() {
        live = false;
        generation += 1;
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
      // Load the decoder before asking for camera access; if the optional chunk fails, no media
      // stream is created and therefore none can be stranded outside `closeScan` cleanup.
      const { default: decodeQr } = await loadQrDecoder();
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
        const hit = decodeQr(img.data, img.width, img.height);
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
  type InvitePreview = {
    direct_routes: number;
    rendezvous_routes: number;
    switchboards: number;
    expires_at_ms: number;
  };
  let joinPreview = $state<InvitePreview | null>(null);
  let joinPreviewCode = $state("");
  let joinSwitchboardConsent = $state(false);
  type JoinReplyReady = { code: string; expires_at_ms: number; candidate_count: number };
  let joinReplyReady = $state<JoinReplyReady | null>(null);
  let joinReplyNow = $state(Date.now());
  let joinReplyExpired = $derived(
    joinReplyReady !== null && joinReplyIsExpired(joinReplyReady.expires_at_ms, joinReplyNow),
  );
  let joinReplyInput = $state("");
  let joinReplyApplying = $state(false);
  let joinReplyNeedsReplace = $state(false);
  let copied = $state(false);
  let newChannel = $state("");

  let messages = $state<Msg[]>([]);
  // The actor remains the source of truth for the complete channel history. Only a bounded slice
  // enters the DOM, which caps Svelte work, rich-media resolution, observers and layout cost in a
  // long-running room. Sanitized HTML is cached in memory only and wiped at the lock boundary.
  const messageRenderCache = new SanitizedMessageCache();
  let messageWindow = $state<ChatWindow>({ start: 0, end: 0 });
  let messageWindowScope = $state("");
  let chatStickToBottom = $state(true);
  let expandingMessageWindow = false;
  let renderedMessages = $derived(messages.slice(messageWindow.start, messageWindow.end));
  // Only rows inserted by a live update receive an arrival animation. A short-lived id set keeps
  // history loads and ordinary re-renders still, and also lets each sender choose their motion.
  let arrivalMessageIds = $state<Set<string>>(new Set());
  function markMessageArrivals(ids: string[]) {
    if (!ids.length) return;
    arrivalMessageIds = new Set([...arrivalMessageIds, ...ids]);
    setTimeout(() => {
      const next = new Set(arrivalMessageIds);
      for (const id of ids) next.delete(id);
      arrivalMessageIds = next;
    }, 900);
  }
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

  function activeMessageScope(): string {
    return activeServerId !== null && cur?.active ? chatScopeKey(activeServerId, cur.active) : "";
  }
  function messageDomKey(message: Msg, index: number): string {
    // Persisted messages have immutable ids. The fallback is only for legacy rows which predate
    // them; including the index avoids accidentally reusing DOM state between equal timestamps.
    return message.id || `legacy:${message.ts}:${message.author}:${index}`;
  }
  function renderedMessage(message: Msg): string {
    return messageRenderCache.render(
      activeMessageScope(),
      message.id || `legacy:${message.ts}:${message.author}`,
      message.text,
      message.edited,
      myMentionName,
      renderMessage,
    );
  }
  async function ensureMessageRendered(msgIdx: number) {
    if (msgIdx >= messageWindow.start && msgIdx < messageWindow.end) return;
    messageWindow = windowAround(msgIdx, messages.length, CHAT_INITIAL_ROWS);
    await tick();
  }
  async function scrollToMatch(msgIdx: number) {
    await ensureMessageRendered(msgIdx);
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
      else void scrollToMatch(h.idx); // a legacy message has no id: its index still holds
      return;
    }
    void scrollToMatch(h.idx);
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
    if (h && h.ch === (cur?.active ?? "")) void scrollToMatch(h.idx);
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
    { label: "Announcements", tab: "status" },
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

  // Jump-to-unread: per `server:channel`, the timestamp of the newest message you've seen. It and
  // composer drafts are sealed into the unlocked vault: neither sensitive text nor reading habits
  // fall back to plaintext browser storage.
  let readMarks = $state<Record<string, number>>({});
  let dividerTs = $state(Number.POSITIVE_INFINITY);
  let uiStateSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let uiStateReady = false;
  let uiStateSaveFailed = false;
  let uiStateLoadGeneration = 0;
  function scheduleUiStateSave() {
    if (!uiStateReady || locked) return;
    clearTimeout(uiStateSaveTimer);
    uiStateSaveTimer = setTimeout(() => {
      const json = JSON.stringify({ version: 1, drafts, readMarks });
      void invoke("save_ui_state", { json }).then(() => {
        uiStateSaveFailed = false;
      }).catch((e) => {
        console.warn("UI continuity save failed", e);
        if (!uiStateSaveFailed) toast("Draft/read-position save failed; this session is still usable", "err", 8000);
        uiStateSaveFailed = true;
      });
    }, 250);
  }
  async function loadUiContinuity(generation: number) {
    try {
      let next = sanitizeUiContinuity(JSON.parse(await invoke<string>("get_ui_state")));
      if (generation !== uiStateLoadGeneration || locked) return;
      // Older builds kept read positions in plaintext localStorage. Migrate that one app-owned
      // key only when the sealed record has none, and erase it only after the native save succeeds.
      // A failed save leaves the legacy copy recoverable for the next launch.
      try {
        const migration = planLegacyReadMarkMigration(next, localStorage.getItem("catcoms.readmarks"));
        if (migration.saveBeforeRemoval) {
          await invoke("save_ui_state", {
            json: JSON.stringify(migration.state),
          });
        }
        if (migration.removeLegacy) {
          localStorage.removeItem("catcoms.readmarks");
        }
        next = migration.state;
      } catch (migrationError) {
        console.warn("Legacy read-mark migration failed", migrationError);
      }
      if (generation !== uiStateLoadGeneration || locked) return;
      drafts = next.drafts;
      readMarks = next.readMarks;
    } catch (e) {
      if (generation !== uiStateLoadGeneration || locked) return;
      console.warn("UI continuity load failed", e);
      drafts = {};
      readMarks = {};
      error = `Durable history could not be authenticated and was not loaded: ${e}`;
    } finally {
      if (generation === uiStateLoadGeneration && !locked) uiStateReady = true;
    }
  }
  function chanKey(): string | null {
    if (activeServerId === null || !cur?.active) return null;
    return chatScopeKey(activeServerId, cur.active);
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
      scheduleUiStateSave();
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
  let messageFrameBreaks = $derived.by(() => {
    const breaks = new Set<number>();
    if (!CHAT_MESSAGE_FRAMES_ENABLED) return breaks;
    if (firstUnreadIdx >= 0) breaks.add(firstUnreadIdx);
    for (let i = 1; i < messages.length; i++) {
      if (!sameDay(messages[i - 1].ts, messages[i].ts)) breaks.add(i);
    }
    return breaks;
  });

  let draft = $state("");
  let sending = $state(false);
  let pendingSendNonce = 0;
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
  let editMessageEl = $state<HTMLTextAreaElement | undefined>();
  let announcementInputEl = $state<HTMLTextAreaElement | undefined>();
  let profileBioEl = $state<HTMLTextAreaElement | undefined>();
  let eventTitleEl = $state<HTMLInputElement | undefined>();
  let eventBodyEl = $state<HTMLTextAreaElement | undefined>();

  type TextEffectTarget = "chat" | "chat-edit" | "announcement" | "wiki" | "bio" | "event-title" | "event-body";
  const TEXT_EFFECT_KEYBINDS_KEY = "catcoms.text-effect-keybinds";
  function loadTextEffectKeybinds(): Record<string, string> {
    try {
      const stored = localStorage.getItem(TEXT_EFFECT_KEYBINDS_KEY);
      return stored === null
        ? { ...DEFAULT_TEXT_EFFECT_KEYBINDS }
        : sanitizeTextEffectKeybinds(JSON.parse(stored));
    } catch {
      return { ...DEFAULT_TEXT_EFFECT_KEYBINDS };
    }
  }
  let textEffectKeybinds = $state<Record<string, string>>(loadTextEffectKeybinds());
  let textEffectTarget = $state<TextEffectTarget | null>(null);
  let textEffectSelection = $state({ start: 0, end: 0 });
  let textEffectBubble = $state({ x: 0, y: 0 });
  let showTextEffectCatalog = $state(false);
  let textEffectQuery = $state("");
  let recordingTextEffect = $state("");
  let textEffectKeyError = $state("");
  let suppressTextEffectSelection = false;
  const QUICK_TEXT_EFFECT_IDS = [
    "shake", "wave", "sparkle", "speakese", "perfect-cherry-blossom", "red-truth",
    "flame", "gloom", "cyber", "crt", "censor", "pride/rainbow",
  ];
  let quickTextEffects = $derived(TEXT_EFFECTS.filter((effect) => QUICK_TEXT_EFFECT_IDS.includes(effect.id)));
  let filteredTextEffects = $derived.by(() => {
    const q = textEffectQuery.trim().toLowerCase();
    return q
      ? TEXT_EFFECTS.filter((effect) => `${effect.label} ${effect.description} ${effect.group} ${effect.id}`.toLowerCase().includes(q))
      : TEXT_EFFECTS;
  });

  function textEffectElement(target: TextEffectTarget): HTMLInputElement | HTMLTextAreaElement | undefined {
    if (target === "chat") return composerEl;
    if (target === "chat-edit") return editMessageEl;
    if (target === "announcement") return announcementInputEl;
    if (target === "wiki") return wikiTextarea;
    if (target === "bio") return profileBioEl;
    if (target === "event-title") return eventTitleEl;
    return eventBodyEl;
  }
  function textEffectValue(target: TextEffectTarget): string {
    if (target === "chat") return draft;
    if (target === "chat-edit") return editDraft;
    if (target === "announcement") return statusDraft;
    if (target === "wiki") return wikiBody;
    if (target === "bio") return pDescription;
    if (target === "event-title") return evTitle;
    return evBody;
  }
  function setTextEffectValue(target: TextEffectTarget, value: string) {
    if (target === "chat") draft = value;
    else if (target === "chat-edit") editDraft = value;
    else if (target === "announcement") statusDraft = value;
    else if (target === "wiki") { wikiBody = value; wikiDirty = true; }
    else if (target === "bio") pDescription = value;
    else if (target === "event-title") evTitle = value;
    else evBody = value;
  }
  function captureTextEffectSelection(target: TextEffectTarget): boolean {
    const input = textEffectElement(target);
    if (!input) return false;
    textEffectSelection = { start: input.selectionStart ?? 0, end: input.selectionEnd ?? 0 };
    return true;
  }
  function openTextEffectCatalog(target: TextEffectTarget) {
    if (!captureTextEffectSelection(target)) return;
    textEffectTarget = target;
    textEffectQuery = "";
    showTextEffectCatalog = true;
  }
  function textEffectTargetLabel(target: TextEffectTarget): string {
    return ({
      chat: "chat message",
      "chat-edit": "edited message",
      announcement: "announcement",
      wiki: "wiki prose",
      bio: "profile bio",
      "event-title": "event title",
      "event-body": "event details",
    } as Record<TextEffectTarget, string>)[target];
  }
  function textEffectSelectionAnchor(input: HTMLInputElement | HTMLTextAreaElement, start: number, end: number) {
    const rect = input.getBoundingClientRect();
    const computed = getComputedStyle(input);
    const mirror = document.createElement("div");
    const picked = document.createElement("span");
    const copied = [
      "font-family", "font-size", "font-style", "font-weight", "font-variant", "line-height",
      "letter-spacing", "text-transform", "text-indent", "word-spacing", "tab-size",
      "padding-top", "padding-right", "padding-bottom", "padding-left", "border-top-width",
      "border-right-width", "border-bottom-width", "border-left-width", "box-sizing",
    ];
    mirror.style.position = "fixed";
    mirror.style.left = "0";
    mirror.style.top = "0";
    mirror.style.width = `${rect.width}px`;
    mirror.style.visibility = "hidden";
    mirror.style.pointerEvents = "none";
    mirror.style.overflow = "hidden";
    mirror.style.whiteSpace = input instanceof HTMLInputElement ? "pre" : "pre-wrap";
    mirror.style.overflowWrap = input instanceof HTMLInputElement ? "normal" : "break-word";
    for (const prop of copied) mirror.style.setProperty(prop, computed.getPropertyValue(prop));
    mirror.append(document.createTextNode(input.value.slice(0, start)));
    picked.textContent = input.value.slice(start, end) || "\u200b";
    mirror.append(picked, document.createTextNode(input.value.slice(end)));
    document.body.append(mirror);
    const mirrorRect = mirror.getBoundingClientRect();
    const pickedRect = picked.getBoundingClientRect();
    const x = rect.left + pickedRect.left - mirrorRect.left + pickedRect.width / 2 - input.scrollLeft;
    const y = rect.top + pickedRect.top - mirrorRect.top - input.scrollTop - 8;
    mirror.remove();
    const halfPalette = Math.min(205, Math.max(0, (innerWidth - 18) / 2));
    return {
      x: Math.max(9 + halfPalette, Math.min(innerWidth - 9 - halfPalette, x)),
      y: Math.max(8, Math.min(rect.bottom - 8, y)),
    };
  }
  function onTextEffectSelection(target: TextEffectTarget) {
    if (suppressTextEffectSelection || !captureTextEffectSelection(target)) return;
    if (textEffectSelection.start === textEffectSelection.end) {
      if (!showTextEffectCatalog && textEffectTarget === target) textEffectTarget = null;
      return;
    }
    const input = textEffectElement(target);
    if (!input) return;
    textEffectBubble = textEffectSelectionAnchor(input, textEffectSelection.start, textEffectSelection.end);
    textEffectTarget = target;
    showTextEffectCatalog = false;
  }
  async function applyTextEffect(id: string, target = textEffectTarget) {
    if (!target) return;
    const wrapped = insertTextEffect(textEffectValue(target), textEffectSelection.start, textEffectSelection.end, id);
    setTextEffectValue(target, wrapped.value);
    showTextEffectCatalog = false;
    textEffectTarget = null;
    suppressTextEffectSelection = true;
    await tick();
    const input = textEffectElement(target);
    input?.focus();
    input?.setSelectionRange(wrapped.selectionStart, wrapped.selectionEnd);
    queueMicrotask(() => { suppressTextEffectSelection = false; });
  }
  function activeTextEffectTarget(): TextEffectTarget | null {
    const active = document.activeElement;
    for (const target of ["chat", "chat-edit", "announcement", "wiki", "bio", "event-title", "event-body"] as TextEffectTarget[]) {
      if (textEffectElement(target) === active) return target;
    }
    return null;
  }
  function setTextEffectKeybind(id: string, chord: string) {
    const conflict = keybindConflict(textEffectKeybinds, id, chord);
    if (conflict) { textEffectKeyError = conflict; return; }
    textEffectKeybinds = { ...textEffectKeybinds, [id]: chord };
    textEffectKeyError = "";
    recordingTextEffect = "";
    try { localStorage.setItem(TEXT_EFFECT_KEYBINDS_KEY, JSON.stringify(textEffectKeybinds)); } catch { /* best-effort */ }
  }
  function clearTextEffectKeybind(id: string) {
    const next = { ...textEffectKeybinds };
    delete next[id];
    textEffectKeybinds = next;
    recordingTextEffect = "";
    textEffectKeyError = "";
    try { localStorage.setItem(TEXT_EFFECT_KEYBINDS_KEY, JSON.stringify(next)); } catch { /* best-effort */ }
  }
  function resetTextEffectKeybinds() {
    textEffectKeybinds = { ...DEFAULT_TEXT_EFFECT_KEYBINDS };
    recordingTextEffect = "";
    textEffectKeyError = "";
    try { localStorage.removeItem(TEXT_EFFECT_KEYBINDS_KEY); } catch { /* best-effort */ }
  }
  function recordTextEffectKey(e: KeyboardEvent, id: string) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") { recordingTextEffect = ""; textEffectKeyError = ""; return; }
    const chord = keybindFromEvent(e);
    if (chord) setTextEffectKeybind(id, chord);
  }

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
  type Tab = "chat" | "files" | "status" | "wiki" | "profile" | "downloads" | "events" | "moderation" | "storage" | "connectivity";
  let view = $state<Tab>("chat");
  type StorageHealth = {
    listed_files: number; referenced_chunks: number; verified_chunks: number;
    missing_chunks: number; unreadable_chunks: number; invalid_manifests: number;
    verified_bytes: number; has_peers: boolean;
    checked_at_ms: number; unique_files: number; logical_bytes: number;
    local_estimated_bytes: number; pinned_files: number; pinned_logical_bytes: number;
    pinned_local_estimated_bytes: number;
    categories: Array<{ name: string; files: number; logical_bytes: number; local_estimated_bytes: number; pinned_files: number }>;
    largest_files: Array<{ name: string; path: string; cid: string; mime: string; logical_bytes: number; local_estimated_bytes: number; pinned: boolean; held: number; total: number }>;
  };
  let storageHealth = $state<StorageHealth | null>(null);
  // The Rust bridge is the authoritative once-per-process cache (and survives frontend HMR).
  // This mirror avoids even an IPC round-trip when revisiting a server in this frontend mount.
  const storageHealthCache = new Map<number, StorageHealth>();
  let storageChecking = $state(false);
  let storageRepairing = $state(false);
  let storageRepairNote = $state("");
  let storageCategoryMax = $derived(Math.max(1, ...(storageHealth?.categories ?? []).map((row) => row.local_estimated_bytes)));
  let moderation = $state<ModerationState>({ events: [], votes: [] });
  let moderationMessages = $state<TimelineMessage[]>([]);
  let moderationLoading = $state(false);
  let moderationReason = $state("");
  let moderationSelected = $state<Set<string>>(new Set());
  let moderationAnchor = $state("");
  // Batch deletion is irreversible at the product layer, so require a deliberate second click.
  // Any selection change disarms it, preventing a confirmation for one range applying to another.
  let moderationDeleteArmed = $state(false);
  let expandedWarnings = $state<Set<string>>(new Set());
  let caseTarget = $state("");
  let caseReason = $state("");
  let caseEvidence = $state<Set<string>>(new Set());
  let moderationUserFilter = $state("");
  let moderationTimeline = $derived(buildModerationTimeline(moderationMessages, moderation.events));
  let filteredModerationTimeline = $derived(filterModerationTimeline(moderationTimeline, moderationUserFilter));
  let moderationGraph = $derived(buildModerationGraph(filteredModerationTimeline));
  let moderationUsers = $derived.by(() => [...new Set(moderationTimeline.flatMap(timelineIdentities))]
    .sort((a, b) => nameOf(a).localeCompare(nameOf(b))));
  let moderationWarnings = $derived(warningMap(moderation.events));
  let moderationCases = $derived(openKickCases(moderation.events));
  let signedWarnings = $derived(moderation.events.filter((event) => event.kind === "warning" && event.signature_valid && event.authorized));

  function warningFor(channel: string, messageId: string): ModerationEvent | undefined {
    return moderationWarnings.get(`${channel}:${messageId}`);
  }
  function toggleWarning(id: string) {
    const next = new Set(expandedWarnings);
    if (next.has(id)) next.delete(id); else next.add(id);
    expandedWarnings = next;
  }
  function selectModerationRow(key: string, extend: boolean) {
    const result = selectTimelineRows(
      filteredModerationTimeline.map((row) => row.key), moderationSelected, key, moderationAnchor, extend,
    );
    moderationSelected = result.selected;
    moderationAnchor = result.anchor;
    moderationDeleteArmed = false;
  }
  function selectedModerationMessages(): TimelineMessage[] {
    return filteredModerationTimeline
      .filter((row) => row.kind === "message" && moderationSelected.has(row.key))
      .map((row) => (row as Extract<typeof row, { kind: "message" }>).message);
  }
  function setModerationUserFilter(identity: string) {
    moderationUserFilter = identity;
    moderationSelected = new Set();
    moderationAnchor = "";
    moderationDeleteArmed = false;
  }
  // Two reads of very different cost sit behind this one name. The case/vote state is small and
  // EVERYONE needs it, because a community vote renders in chat: it loads on every switch. The
  // message corpus behind the privileged timeline is every message in every channel, and only the
  // moderation surface reads it, so it loads with that surface. `withCorpus` defaults to whether
  // the surface is actually open, which is the right answer at every existing call site.
  async function refreshModeration(withCorpus = moderationSurfaceOpen(view, inboxView, spaceOpen)) {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null || cur?.isDm) {
      moderation = { events: [], votes: [] };
      moderationMessages = [];
      return;
    }
    const wantCorpus = withCorpus && canModerate;
    moderationLoading = true;
    try {
      const [state, channelRows] = await Promise.all([
        invoke<ModerationState>("get_moderation", { server }),
        wantCorpus
          ? Promise.all((cur?.channels ?? []).map(async (channel) => {
              const rows = await invoke<Msg[]>("get_messages", { server, channel: channel.id });
              return rows.map((message): TimelineMessage => ({
                id: message.id, author: message.author, text: message.text, ts: message.ts,
                channel: channel.id, channelName: channel.name,
              }));
            }))
          : Promise.resolve([] as TimelineMessage[][]),
      ]);
      if (!viewCurrent(gen, server)) return;
      moderation = state;
      // Only a corpus read may replace the corpus: a light refresh must not blank the timeline out
      // from under an open moderation surface. Losing moderator authority is the exception, and the
      // one case the old unconditional assignment used to cover: the corpus goes with the role.
      if (wantCorpus) moderationMessages = channelRows.flat();
      else if (!canModerate) moderationMessages = [];
    } catch (e) {
      if (viewCurrent(gen, server)) error = String(e);
    } finally {
      // A stale call must not clear the flag for the live one; clearServerView resets it on switch.
      if (viewCurrent(gen, server)) moderationLoading = false;
    }
  }
  async function warnModerationSelection() {
    // Captured once, not re-read per iteration: the channel and message ids come from THIS
    // server's corpus, so a switch mid-loop would address the rest of them to a different server.
    const server = activeServerId;
    if (server === null || !moderationReason.trim()) return;
    const selected = selectedModerationMessages();
    if (!selected.length) return;
    try {
      for (const message of selected) {
        await invoke("warn_message", {
          server, channel: message.channel,
          messageId: message.id, reason: moderationReason.trim(),
        });
      }
      moderationReason = "";
      moderationSelected = new Set();
      moderationDeleteArmed = false;
      await refreshModeration();
      if (view === "chat") await refresh();
    } catch (e) { error = String(e); }
  }
  async function deleteModerationSelection() {
    const server = activeServerId; // as above: the ids belong to this server's corpus
    if (server === null) return;
    const selected = selectedModerationMessages();
    if (!selected.length) return;
    if (!moderationDeleteArmed) {
      moderationDeleteArmed = true;
      return;
    }
    try {
      for (const message of selected) {
        await invoke("delete_message", {
          server, channel: message.channel, msgId: message.id,
        });
      }
      moderationSelected = new Set();
      moderationDeleteArmed = false;
      await refreshModeration();
      if (view === "chat") await refresh();
    } catch (e) { error = String(e); }
  }
  function toggleCaseEvidence(id: string) {
    const next = new Set(caseEvidence);
    if (next.has(id)) next.delete(id); else next.add(id);
    caseEvidence = next;
  }
  async function openKickCase() {
    if (activeServerId === null || !caseTarget || !caseReason.trim()) return;
    try {
      await invoke("create_kick_case", {
        server: activeServerId, target: caseTarget, reason: caseReason.trim(),
        evidenceIds: [...caseEvidence],
      });
      caseReason = "";
      caseEvidence = new Set();
      await refreshModeration();
    } catch (e) { error = String(e); }
  }
  async function voteKick(caseId: string, yes: boolean) {
    if (activeServerId === null) return;
    try {
      await invoke("cast_kick_vote", { server: activeServerId, caseId, yes });
      await refreshModeration();
    } catch (e) { error = String(e); }
  }
  async function resolveKick(caseId: string, remove: boolean) {
    if (activeServerId === null) return;
    try {
      await invoke("resolve_kick_case", { server: activeServerId, caseId, remove });
      await Promise.all([refreshModeration(), refreshMembers(), refreshRoles()]);
    } catch (e) { error = String(e); }
  }
  let wikiPages = $state<string[]>([]);
  let wikiFilter = $state("");
  let filteredWikiPages = $derived.by(() => {
    const q = wikiFilter.trim().toLowerCase();
    return q ? wikiPages.filter((p) => p.toLowerCase().includes(q)) : wikiPages;
  });
  let wikiMap = $state<Record<string, string>>({}); // name -> body (backlinks + link existence)
  // Which server wikiMap holds. Deliberately NOT $state: ensureWikiMap is called from the ref-card
  // effect, and a reactive read-then-write of this inside that effect would re-trigger it forever.
  let wikiMapFor: number | null = null;
  // The ref cards in chat summarise a page out of wikiMap, so it is not wiki-tab-only state. It is
  // every page's full body though, far too big for the switch barrier, so it loads once per server
  // the first time a pane that can render cards is live. The ref-card effect tracks wikiMap, so
  // the cards re-resolve themselves when it lands.
  async function ensureWikiMap() {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null || wikiMapFor === server) return;
    wikiMapFor = server; // claimed up front: the effect can re-run before the read returns
    try {
      const map = await invoke<Record<string, string>>("get_wiki_map", { server });
      if (viewCurrent(gen, server)) wikiMap = map;
    } catch {
      if (wikiMapFor === server) wikiMapFor = null; // let a later pass retry
    }
  }
  let wikiMeta = $state<Record<string, string>>({}); // name -> "md" | "wiki" (per-page format, shared)
  let activeWikiPage = $state("");
  let wikiBody = $state("");
  let newWikiPage = $state("");
  let wikiDirty = $state(false); // unsaved edits in the open page (avoid clobbering on live updates)
  let wikiEdit = $state(false); // edit (textarea) vs read (rendered) mode
  let wikiEl = $state<HTMLDivElement | undefined>(undefined); // rendered-page container (media resolve)
  let showWikiHelp = $state(false);
  type WikiHelpOverlayComponent = (typeof import("./WikiHelpOverlay.svelte"))["default"];
  let WikiHelpOverlay = $state<WikiHelpOverlayComponent | null>(null);
  let wikiHelpLoading = false;
  let wikiHelpLoadError = $state("");
  async function loadWikiHelpOverlay() {
    if (WikiHelpOverlay || wikiHelpLoading) return;
    wikiHelpLoading = true;
    wikiHelpLoadError = "";
    try {
      WikiHelpOverlay = (await import("./WikiHelpOverlay.svelte")).default;
    } catch (cause) {
      wikiHelpLoadError = String(cause);
    } finally {
      wikiHelpLoading = false;
    }
  }
  $effect(() => {
    // Warm this small help chunk after entering Wiki, while keeping it off the app startup path.
    if ((view === "wiki" || showWikiHelp) && !WikiHelpOverlay) void loadWikiHelpOverlay();
  });
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
  let wikiReviewDays = $state(WIKI_REVIEW_UNKNOWN); // server setting: 0 = edits publish immediately
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
  let pEffects = $state<NameEffect[]>([]);
  let collapsedEffects = $state<Record<string, boolean>>({});
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
    { id: "cute", label: "Cute" },
    { id: "gothic", label: "Gothic" },
  ];
  const NAME_FONT_IDS = new Set(NAME_FONTS.map((font) => font.id));
  const NAME_EFFECTS: { id: NameEffectId; label: string; description: string; group: "Fill" | "Motion" | "Finish" }[] = [
    { id: "gradient", label: "Gradient", description: "A custom multi-colour fill.", group: "Fill" },
    { id: "rainbow", label: "Rainbow", description: "A scrolling spectrum fill.", group: "Fill" },
    { id: "shimmer", label: "Shimmer", description: "A bright sweep across the letters.", group: "Fill" },
    { id: "candy", label: "Candy stripes", description: "Two-colour striped lettering.", group: "Fill" },
    { id: "wave", label: "Bounce", description: "The whole name gently bobs.", group: "Motion" },
    { id: "mexican", label: "Mexican wave", description: "Letters rise one after another.", group: "Motion" },
    { id: "pulse", label: "Pulse", description: "The name softly fades in and out.", group: "Motion" },
    { id: "wobble", label: "Wobble", description: "A playful side-to-side tilt.", group: "Motion" },
    { id: "sparkle", label: "Sparkle", description: "Small highlights twinkle around the name.", group: "Finish" },
    { id: "neon", label: "Neon", description: "A coloured glow around the letters.", group: "Finish" },
    { id: "outline", label: "Outline", description: "A coloured edge around each letter.", group: "Finish" },
    { id: "shadow", label: "Drop shadow", description: "A configurable shadow behind the name.", group: "Finish" },
    { id: "retro", label: "Retro", description: "A crisp offset copy of the name.", group: "Finish" },
    { id: "glitch", label: "Glitch", description: "A small red-and-blue colour fringe.", group: "Finish" },
    { id: "ghost", label: "Ghost", description: "Soft, translucent frosted text.", group: "Finish" },
    { id: "fire", label: "Fire glow", description: "A warm, flickering ember aura.", group: "Finish" },
    { id: "extrude", label: "3D extrude", description: "Layered depth behind the letters.", group: "Finish" },
  ];
  const EFFECT_GROUPS = ["Fill", "Motion", "Finish"] as const;
  const PUBLIC_EFFECT_IDS = new Set(NAME_EFFECTS.map((effect) => effect.id));
  let appliedEffects = $derived(pEffects.filter((effect) => PUBLIC_EFFECT_IDS.has(effect.id)));
  const prefersStill = typeof matchMedia !== "undefined" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  let fxMotionOff = $derived(appearance.motion === "off" || prefersStill);
  // Curated name colours that stay legible on the dark grounds (content, not theme).
  const NAME_COLORS = ["#977df2", "#6ca0d8", "#57c77a", "#d8a657", "#e0574b", "#e879c0", "#6ee7d8", "#c6c2d6"];
  // Message frames preserve the existing opaque profile value for wire compatibility; the new
  // renderer contains that colour inside the content column instead of painting the whole row.
  const BUBBLE_PRESETS: { code: string; label: string; value: string }[] = [
    { code: "OFF", label: "Open channel", value: "" },
    { code: "OCN", label: "Deep sea", value: "linear-gradient(135deg,#1a2980,#26415e)" },
    { code: "RED", label: "Redshift", value: "linear-gradient(135deg,#c31432,#5c1020)" },
    { code: "BIO", label: "Canopy", value: "linear-gradient(135deg,#134e5e,#1c7a4d)" },
    { code: "UV", label: "Ultraviolet", value: "linear-gradient(135deg,#41295a,#5d2a6e)" },
    { code: "THR", label: "Furnace", value: "linear-gradient(135deg,#8a3a12,#b34700)" },
    { code: "ROS", label: "Night rose", value: "linear-gradient(135deg,#7a1f3d,#3d1020)" },
    { code: "GRF", label: "Graphite", value: "#3a3f4b" },
  ];
  const GRAD_MAX_STOPS = 8;
  const BUB_GRAD_RE = /^linear-gradient\(135deg,(#[0-9a-fA-F]{6}),(#[0-9a-fA-F]{6})\)$/;
  let pBubA = $state("#41295a");
  let pBubB = $state("#1a2980");
  const customBubble = () => `linear-gradient(135deg,${pBubA},${pBubB})`;
  let pFrame = $derived(parseMessageFrame(pBubble));
  let framePreviewReplay = $state(0);
  let collapsedFrameEffects = $state<Record<string, boolean>>({});
  const FRAME_SHAPES: { id: MessageFrameShape; code: string; label: string; description: string }[] = [
    { id: "terminal", code: "TRM", label: "Terminal", description: "Clipped operator panel with a signal rail" },
    { id: "bracket", code: "BRK", label: "Brackets", description: "Open targeting corners with a quiet centre" },
    { id: "packet", code: "PKT", label: "Packet", description: "Squared data block with a header bus" },
    { id: "holo", code: "HOL", label: "Holo", description: "Soft projected glass with a luminous edge" },
    { id: "signal", code: "SIG", label: "Signal", description: "Minimal transmission rail and fading wash" },
  ];
  const FRAME_EFFECTS: { id: MessageFrameEffectId; code: string; label: string; description: string }[] = [
    { id: "scan", code: "SCN", label: "Scan", description: "The channel's shared sweep crosses this frame" },
    { id: "pulse", code: "PLS", label: "Pulse", description: "The signal edge breathes gently" },
    { id: "trace", code: "TRC", label: "Trace", description: "A short telemetry trace runs the frame" },
    { id: "flicker", code: "FLK", label: "Flicker", description: "A restrained terminal refresh flicker" },
  ];
  const FRAME_MOTIONS: { id: MessageFrameMotion; label: string; description: string; glyph: string }[] = [
    { id: "none", label: "Still", description: "No entrance movement", glyph: "—" },
    { id: "glide", label: "Glide", description: "Lift gently into place", glyph: "↑" },
    { id: "fly", label: "Fly in", description: "Sweep in from the side", glyph: "→" },
    { id: "pop", label: "Pop", description: "A quick soft-scale arrival", glyph: "◇" },
    { id: "drift", label: "Drift", description: "Float diagonally into place", glyph: "↗" },
  ];
  const FRAME_EASINGS: { id: MessageFrameEasing; label: string; description: string }[] = [
    { id: "soft", label: "Soft", description: "Gentle terminal easing" },
    { id: "snappy", label: "Snappy", description: "Fast response with a firm stop" },
    { id: "spring", label: "Spring", description: "A restrained overshoot" },
  ];
  function updateFrame(patch: Partial<typeof pFrame>) {
    pBubble = encodeMessageFrame({ ...pFrame, ...patch });
  }
  function selectFrameEffect(id: MessageFrameEffectId) {
    if (pFrame.effects.some((layer) => layer.id === id)) {
      collapsedFrameEffects[id] = false;
      return;
    }
    updateFrame({ effects: [...pFrame.effects, defaultMessageFrameLayer(id)] });
    collapsedFrameEffects[id] = false;
  }
  function setFrameEffectEnabled(id: MessageFrameEffectId, enabled: boolean) {
    updateFrame({ effects: pFrame.effects.map((layer) => layer.id === id ? { ...layer, enabled } : layer) });
  }
  function updateFrameEffect(id: MessageFrameEffectId, key: keyof MessageFrameEffectOptions, value: number) {
    updateFrame({ effects: pFrame.effects.map((layer) => layer.id === id
      ? { ...layer, options: { ...layer.options, [key]: value } }
      : layer) });
  }
  function resetFrameEffect(id: MessageFrameEffectId) {
    updateFrame({ effects: pFrame.effects.map((layer) => layer.id === id
      ? { ...defaultMessageFrameLayer(id), enabled: layer.enabled }
      : layer) });
  }
  function removeFrameEffect(id: MessageFrameEffectId) {
    updateFrame({ effects: pFrame.effects.filter((layer) => layer.id !== id) });
    delete collapsedFrameEffects[id];
  }
  function disableAllFrameEffects() {
    updateFrame({ effects: pFrame.effects.map((layer) => ({ ...layer, enabled: false })) });
  }
  function moveFrameEffect(id: MessageFrameEffectId, direction: -1 | 1) {
    const from = pFrame.effects.findIndex((layer) => layer.id === id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= pFrame.effects.length) return;
    const effects = [...pFrame.effects];
    [effects[from], effects[to]] = [effects[to], effects[from]];
    updateFrame({ effects });
  }
  function updateFrameArrival(patch: Partial<MessageFrameArrival>) {
    updateFrame({ arrival: { ...pFrame.arrival, ...patch } });
  }
  function resetMessageStudio() {
    pBubble = "";
    collapsedFrameEffects = {};
  }
  const FILL_EFFECT_IDS = new Set<NameEffectId>(["gradient", "rainbow", "shimmer", "candy"]);
  const TRANSFORM_EFFECT_IDS = new Set<NameEffectId>(["wave", "wobble"]);

  type NameStyleSnapshot = { font: string; color: string; effect: string };
  let styleUndo = $state<NameStyleSnapshot[]>([]);
  let styleRedo = $state<NameStyleSnapshot[]>([]);
  let lastStyleHistoryKey = "";
  let lastStyleHistoryAt = 0;

  const styleSnapshot = (): NameStyleSnapshot => ({ font: pFont, color: pColor, effect: pEffect });

  function rememberNameStyle(key: string) {
    const now = Date.now();
    // A slider can emit dozens of input events per drag. One undo step per short, continuous
    // adjustment is useful; one per pixel is not.
    if (key !== lastStyleHistoryKey || now - lastStyleHistoryAt > 650) {
      styleUndo = [...styleUndo.slice(-39), styleSnapshot()];
      styleRedo = [];
    }
    lastStyleHistoryKey = key;
    lastStyleHistoryAt = now;
  }

  function restoreNameStyle(snapshot: NameStyleSnapshot) {
    pFont = snapshot.font;
    pColor = snapshot.color;
    pEffect = snapshot.effect;
    pEffects = decodeNameEffects(snapshot.effect);
    lastStyleHistoryKey = "";
  }

  function undoNameStyle() {
    const previous = styleUndo.at(-1);
    if (!previous) return;
    styleRedo = [...styleRedo, styleSnapshot()];
    styleUndo = styleUndo.slice(0, -1);
    restoreNameStyle(previous);
  }

  function redoNameStyle() {
    const next = styleRedo.at(-1);
    if (!next) return;
    styleUndo = [...styleUndo, styleSnapshot()];
    styleRedo = styleRedo.slice(0, -1);
    restoreNameStyle(next);
  }

  function setNameEffects(next: NameEffect[], historyKey = "effects") {
    rememberNameStyle(historyKey);
    pEffects = next;
    pEffect = encodeNameEffects(next);
  }

  function setNameFont(font: string) {
    if (font === pFont) return;
    rememberNameStyle("font");
    pFont = font;
  }

  function setNameColor(color: string) {
    if (color === pColor) return;
    rememberNameStyle("color");
    pColor = color;
  }

  function selectNameEffect(id: NameEffectId) {
    if (effectConfigured(pEffects, id)) {
      collapsedEffects[id] = false;
      return;
    }
    // Both fill effects stay configured, but only one can be enabled at a time. Switching
    // between them therefore keeps every stop, speed and direction setting intact.
    const exclusive = FILL_EFFECT_IDS.has(id) ? FILL_EFFECT_IDS : TRANSFORM_EFFECT_IDS.has(id) ? TRANSFORM_EFFECT_IDS : null;
    const prepared = exclusive
      ? pEffects.map((effect) => exclusive.has(effect.id)
        ? { ...effect, enabled: false }
        : effect)
      : pEffects;
    setNameEffects([...prepared, defaultNameEffect(id)], `add:${id}`);
    collapsedEffects[id] = false;
  }

  function setNameEffectEnabled(id: NameEffectId, enabled: boolean) {
    const exclusive = FILL_EFFECT_IDS.has(id) ? FILL_EFFECT_IDS : TRANSFORM_EFFECT_IDS.has(id) ? TRANSFORM_EFFECT_IDS : null;
    setNameEffects(pEffects.map((effect) => {
      if (effect.id === id) return { ...effect, enabled };
      if (enabled && exclusive?.has(effect.id)) {
        return { ...effect, enabled: false };
      }
      return effect;
    }), `enable:${id}`);
  }

  function removeNameEffect(id: NameEffectId) {
    setNameEffects(pEffects.filter((effect) => effect.id !== id), `remove:${id}`);
    delete collapsedEffects[id];
  }

  function disableAllNameEffects() {
    setNameEffects(pEffects.map((effect) => PUBLIC_EFFECT_IDS.has(effect.id) ? { ...effect, enabled: false } : effect), "all-off");
  }

  function updateNameEffect(id: NameEffectId, key: keyof NameEffectOptions, value: NameEffectOptions[keyof NameEffectOptions]) {
    setNameEffects(pEffects.map((effect) => effect.id === id
      ? { ...effect, options: { ...effect.options, [key]: value } }
      : effect), `${id}:${String(key)}`);
  }

  function updateStudioOption(id: "typography" | "master", key: keyof NameEffectOptions, value: NameEffectOptions[keyof NameEffectOptions]) {
    const existing = pEffects.find((effect) => effect.id === id);
    const next = existing
      ? pEffects.map((effect) => effect.id === id ? { ...effect, enabled: true, options: { ...effect.options, [key]: value } } : effect)
      : [...pEffects, { ...defaultNameEffect(id), options: { ...defaultNameEffect(id).options, [key]: value } }];
    setNameEffects(next, `${id}:${String(key)}`);
  }

  function resetNameEffect(id: NameEffectId) {
    if (id === "typography" || id === "master") {
      setNameEffects(pEffects.filter((effect) => effect.id !== id), `reset:${id}`);
      return;
    }
    setNameEffects(pEffects.map((effect) => effect.id === id
      ? { ...defaultNameEffect(id), enabled: effect.enabled }
      : effect), `reset:${id}`);
  }

  function moveNameEffect(id: NameEffectId, direction: -1 | 1) {
    const publicOrder = appliedEffects.map((effect) => effect.id);
    const from = publicOrder.indexOf(id);
    const to = from + direction;
    if (from < 0 || to < 0 || to >= publicOrder.length) return;
    [publicOrder[from], publicOrder[to]] = [publicOrder[to], publicOrder[from]];
    const ordered = publicOrder.map((effectId) => pEffects.find((effect) => effect.id === effectId)!);
    const specials = pEffects.filter((effect) => !PUBLIC_EFFECT_IDS.has(effect.id));
    setNameEffects([...ordered, ...specials], `move:${id}`);
  }

  let draggedNameEffect = $state<NameEffectId | null>(null);
  function dropNameEffect(target: NameEffectId) {
    if (!draggedNameEffect || draggedNameEffect === target) return;
    const publicOrder = appliedEffects.map((effect) => effect.id);
    const from = publicOrder.indexOf(draggedNameEffect);
    const to = publicOrder.indexOf(target);
    if (from < 0 || to < 0) return;
    const [moved] = publicOrder.splice(from, 1);
    publicOrder.splice(to, 0, moved);
    const specials = pEffects.filter((effect) => !PUBLIC_EFFECT_IDS.has(effect.id));
    setNameEffects([...publicOrder.map((id) => pEffects.find((effect) => effect.id === id)!), ...specials], `drag:${draggedNameEffect}`);
    draggedNameEffect = null;
  }

  function updateGradientStop(index: number, value: string) {
    const stops = [...(effectOptions(pEffects, "gradient").stops ?? [])];
    stops[index] = value;
    updateNameEffect("gradient", "stops", stops);
  }

  function addGradientStop() {
    const stops = [...(effectOptions(pEffects, "gradient").stops ?? [])];
    if (stops.length < GRAD_MAX_STOPS) stops.push(stops[stops.length - 1] ?? "#977df2");
    updateNameEffect("gradient", "stops", stops);
  }

  function removeGradientStop(index: number) {
    const stops = [...(effectOptions(pEffects, "gradient").stops ?? [])];
    if (stops.length > 2) stops.splice(index, 1);
    updateNameEffect("gradient", "stops", stops);
  }

  type NameRecipe = { id: string; name: string; font: string; color: string; effect: string; builtin?: boolean };
  const NAME_RECIPE_KEY = "catcoms.name-style-recipes.v1";

  function recipeFx(id: NameEffectId, options: NameEffectOptions = {}): NameEffect {
    const effect = defaultNameEffect(id);
    return { ...effect, options: { ...effect.options, ...options } };
  }

  const BUILTIN_NAME_RECIPES: NameRecipe[] = [
    {
      id: "cute", name: "Cute", font: "cute", color: "#fff7fc", builtin: true,
      effect: encodeNameEffects([
        recipeFx("outline", { width: 1.5, color: "#e879c0" }),
        recipeFx("shadow", { x: 1, y: 2, blur: 1, opacity: 85, color: "#3b1830" }),
        recipeFx("typography", { weight: 800, tracking: 0.2, bubble: 0.5 }),
      ]),
    },
    {
      id: "arcade", name: "Neon Arcade", font: "mono", color: "#6ee7d8", builtin: true,
      effect: encodeNameEffects([
        recipeFx("neon", { glow: 12, intensity: 90 }), recipeFx("glitch", { spread: 2, opacity: 60 }),
        recipeFx("pulse", { speed: 4, depth: 28 }), recipeFx("typography", { weight: 800, tracking: 1 }),
      ]),
    },
    {
      id: "holo", name: "Holographic", font: "rounded", color: "#c9fbff", builtin: true,
      effect: encodeNameEffects([
        recipeFx("gradient", { stops: ["#6ee7d8", "#977df2", "#e879c0", "#6ee7d8"], angle: 105, speed: 5 }),
        recipeFx("sparkle", { speed: 4, intensity: 80 }), recipeFx("outline", { width: 0.5, color: "#e9fdff" }),
      ]),
    },
    {
      id: "ghost", name: "Ghost", font: "serif", color: "#d9f5ff", builtin: true,
      effect: encodeNameEffects([
        recipeFx("ghost", { opacity: 64, blur: 0.5, glow: 10 }), recipeFx("wobble", { speed: 2, amount: 1 }),
        recipeFx("typography", { italic: true, weight: 600, tracking: 1 }),
      ]),
    },
    {
      id: "candy", name: "Candy", font: "cute", color: "#fff4fb", builtin: true,
      effect: encodeNameEffects([
        recipeFx("candy", { color: "#ff82bd", secondary: "#fff4fb", angle: 45, speed: 2 }),
        recipeFx("outline", { width: 1, color: "#b83d78" }), recipeFx("shadow", { x: 2, y: 2, blur: 2, opacity: 70 }),
      ]),
    },
    {
      id: "retro", name: "Retro 3D", font: "caps", color: "#ffd66e", builtin: true,
      effect: encodeNameEffects([
        recipeFx("extrude", { depth: 5, color: "#7c315c", opacity: 95 }), recipeFx("outline", { width: 1, color: "#3b1830" }),
        recipeFx("typography", { weight: 900, uppercase: true, tracking: 1.5 }),
      ]),
    },
    {
      id: "ember", name: "Ember", font: "rounded", color: "#ffd36a", builtin: true,
      effect: encodeNameEffects([
        recipeFx("fire", { height: 6, intensity: 88, speed: 7 }), recipeFx("extrude", { depth: 2, color: "#7a2518" }),
        recipeFx("typography", { weight: 800 }),
      ]),
    },
  ];

  function loadNameRecipes(): NameRecipe[] {
    try {
      const raw = JSON.parse(localStorage.getItem(NAME_RECIPE_KEY) ?? "[]");
      if (!Array.isArray(raw)) return [];
      return raw.slice(0, 24).flatMap((value): NameRecipe[] => {
        if (!value || typeof value !== "object") return [];
        const r = value as Partial<NameRecipe>;
        if (typeof r.name !== "string" || typeof r.font !== "string" || typeof r.color !== "string" || typeof r.effect !== "string") return [];
        if (!/^#[0-9a-fA-F]{6}$/.test(r.color) || r.effect.length > 4096) return [];
        const effects = decodeNameEffects(r.effect);
        if (r.effect !== "none" && !effects.length) return [];
        return [{
          id: typeof r.id === "string" ? r.id : crypto.randomUUID(),
          name: r.name.slice(0, 32),
          font: NAME_FONT_IDS.has(r.font) ? r.font : "system",
          color: r.color.toLowerCase(),
          effect: encodeNameEffects(effects),
        }];
      });
    } catch {
      return [];
    }
  }

  let savedNameRecipes = $state<NameRecipe[]>(loadNameRecipes());
  let recipeNameDraft = $state("");

  function persistNameRecipes(next: NameRecipe[]) {
    savedNameRecipes = next.slice(-24);
    try {
      localStorage.setItem(NAME_RECIPE_KEY, JSON.stringify(savedNameRecipes));
    } catch {
      // The library still works for this session if private storage is unavailable or full.
    }
  }

  function applyNameRecipe(recipe: NameRecipe) {
    rememberNameStyle(`recipe:${recipe.id}`);
    pFont = recipe.font;
    pColor = recipe.color;
    pEffect = recipe.effect;
    pEffects = decodeNameEffects(recipe.effect);
  }

  function saveNameRecipe() {
    const name = recipeNameDraft.trim().slice(0, 32);
    if (!name) return;
    persistNameRecipes([...savedNameRecipes, {
      id: crypto.randomUUID(), name, font: pFont, color: pColor, effect: pEffect,
    }]);
    recipeNameDraft = "";
  }

  function deleteNameRecipe(id: string) {
    persistNameRecipes(savedNameRecipes.filter((recipe) => recipe.id !== id));
  }

  const pick = <T,>(items: readonly T[]): T => items[Math.floor(Math.random() * items.length)];
  const chance = (probability: number) => Math.random() < probability;

  function randomizeNameStyle() {
    const effects: NameEffect[] = [];
    const fills: NameEffectId[] = fxMotionOff ? ["gradient", "candy"] : ["gradient", "rainbow", "shimmer", "candy"];
    if (chance(0.75)) effects.push(defaultNameEffect(pick(fills)));
    if (!fxMotionOff && chance(0.55)) effects.push(defaultNameEffect(pick(["wave", "mexican", "wobble", "pulse"] as const)));
    const finishes = ["neon", "outline", "shadow", "retro", "glitch", "ghost", "fire", "extrude", "sparkle"] as const;
    for (const id of [...finishes].sort(() => Math.random() - 0.5).slice(0, chance(0.55) ? 2 : 1)) effects.push(defaultNameEffect(id));
    effects.push(recipeFx("typography", {
      weight: pick([600, 700, 800, 900]), tracking: pick([0, 0.3, 0.7, 1.2]),
      italic: chance(0.18), uppercase: chance(0.18), bubble: chance(0.25) ? 0.5 : 0,
    }));
    rememberNameStyle("randomize");
    pFont = pick(["system", "rounded", "cute", "mono", "script"]);
    pColor = pick(NAME_COLORS);
    pEffects = effects;
    pEffect = encodeNameEffects(effects);
  }

  let namePreviewMode = $state<"all" | "profile" | "chat" | "member" | "mention">("all");
  let namePreviewPaused = $state(false);

  function movingNameEffect(id: NameEffectId): boolean {
    if (!animatedEffect(id)) return false;
    return id !== "gradient" && id !== "candy" || (effectOptions(pEffects, id).speed ?? 0) > 0;
  }

  function relativeLuminance(color: string): number {
    const match = /^#([0-9a-f]{6})$/i.exec(color);
    if (!match) return 1;
    const value = Number.parseInt(match[1], 16);
    const channel = (byte: number) => {
      const c = byte / 255;
      return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * channel(value >> 16) + 0.7152 * channel((value >> 8) & 255) + 0.0722 * channel(value & 255);
  }

  function contrastRatio(a: string, b: string): number {
    const [bright, dark] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
    return (bright + 0.05) / (dark + 0.05);
  }

  function nameStyleWarnings(): string[] {
    const warnings: string[] = [];
    const enabled = appliedEffects.filter((effect) => effect.enabled);
    const type = effectOptions(pEffects, "typography");
    const master = effectOptions(pEffects, "master");
    if (contrastRatio(pColor, "#131218") < 3 && !enabled.some((effect) => FILL_EFFECT_IDS.has(effect.id))) {
      warnings.push("Low contrast on the darkest chat background.");
    }
    if (enabled.filter((effect) => movingNameEffect(effect.id)).length > 2) warnings.push("Several animations at once may feel busy or cost more battery.");
    if (enabled.length > 5) warnings.push("This stack may become hard to read at member-list size.");
    if ((type.tracking ?? 0) > 3) warnings.push("Wide letter spacing can clip long names in the member list.");
    if ((type.bubble ?? 0) > 2 || ((effectOptions(pEffects, "outline").width ?? 0) > 2.5 && effectEnabled(pEffects, "outline"))) {
      warnings.push("Very thick lettering can close up small characters.");
    }
    if (effectEnabled(pEffects, "ghost") && (effectOptions(pEffects, "ghost").opacity ?? 100) < 45) warnings.push("Ghost opacity is faint at compact sizes.");
    if ((master.intensity ?? 100) > 140) warnings.push("High master intensity can make glows and shadows muddy.");
    return warnings;
  }
  let styleWarnings = $derived(nameStyleWarnings());

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
  // The call surfaces' counterpart to nameOf/profileFor. Resolves against the room's server
  // first and only falls back to the viewed server's map, so a peer keeps one name for the
  // whole call no matter where the user wanders in the rail.
  function callProfileFor(fp: string): Prof | undefined {
    return (
      callProfiles[fp] ??
      (deviceMap[fp] ? callProfiles[deviceMap[fp].origin] : undefined) ??
      profiles[fp]
    );
  }
  function callNameOf(fp: string): string {
    return callProfileFor(fp)?.name?.trim() || fp;
  }
  // Two-letter mono monogram for a rail circle (one letter for a one-character name).
  function monogram(name: string): string {
    return (name ?? "").trim().slice(0, 2).toUpperCase() || "?";
  }
  // A profile's custom frame is untrusted; the helper validates it before it reaches CSS. A
  // companion device inherits its origin's frame just as it already inherits the origin's name.
  function profileFor(fp: string): Prof | undefined {
    return profiles[fp] ?? (deviceMap[fp] ? profiles[deviceMap[fp].origin] : undefined);
  }
  function bubbleStyle(fp: string): string {
    // Avoid profile/device identity work for every row while live-chat frames are rolled back.
    if (!CHAT_MESSAGE_FRAMES_ENABLED) return "";
    const isOwn = fp === myFp || (!!myFp && identityOf(fp).fp === identityOf(myFp).fp);
    return visibleMessageFrameStyle(
      profileFor(fp)?.bubble,
      appearance.flat,
      isOwn,
      CHAT_MESSAGE_FRAMES_ENABLED,
    );
  }
  function arrivalMotion(fp: string, id: string): MessageFrameMotion {
    return visibleMessageFrameMotion(
      profileFor(fp)?.bubble,
      arrivalMessageIds.has(id),
      appearance.messageMotion === "off" || fxMotionOff,
    );
  }
  function arrivalStyle(fp: string): string {
    return messageFrameArrivalStyle(profileFor(fp)?.bubble);
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
    const gen = viewGeneration;
    const srv = activeServerId;
    if (srv === null) {
      events = [];
      return;
    }
    try {
      const knownEvents = new Set(events.map((e) => e.id));
      const hadEvents = events.length > 0;
      const next = await invoke<UiEvent[]>("get_events", { server: srv });
      if (!viewCurrent(gen, srv)) return;
      events = next;
      if (hadEvents) {
        for (const ev of events) {
          if (knownEvents.has(ev.id)) continue;
          pushTicker("event", srv, `event:${srv}:${ev.id}`, msgSnippet(ev.title, 60), () => void goSurface(srv, "events"));
        }
      }
    } catch {
      if (viewCurrent(gen, srv)) events = [];
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
    if (!file.type.startsWith("image/")) {
      toast("Drop an image to use it as the event poster", "err", 3500);
      return;
    }
    evImageBusy = true;
    const tid = toast(`Uploading ${file.name}…`, "info", 0);
    try {
      evImage = await addSharedFile(file, myEmbedFolder, file.name, file.type || "image/png");
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
    const t = s.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", timeZoneName: "short" });
    const end = e.end_ts ? `–${new Date(e.end_ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}` : "";
    return `${day} · ${t}${end}`;
  }

  // News feed (inbox): recent status posts + upcoming events across every server.
  // Client-side aggregation over existing per-server invokes: nothing new on the wire.
  type NewsItem = { server: number; serverName: string; kind: "status" | "event"; ts: number; text: string; author: string };
  let inboxMode = $state<"mentions" | "news">("mentions");
  let newsItems = $state<NewsItem[]>([]);
  let newsLoading = $state(false);
  let newsUnseen = $state(false);
  let newsGeneration = 0;
  async function loadNews() {
    // The News tab button, "status-updated" and "events-changed" all call this, so concurrent
    // loads are routine: without a generation the first to finish clears the spinner for the rest.
    const generation = ++newsGeneration;
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
    if (generation !== newsGeneration) return; // a later load owns the list and the spinner
    newsLoading = false;
    if (locked) return; // as loadInbox: the lock cleared this, and it is cross-server text
    newsItems = items;
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
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) {
      deviceMap = {};
      return;
    }
    try {
      const next = await invoke<Record<string, UiDevice>>("get_devices", { server });
      if (!viewCurrent(gen, server)) return; // companion attribution is per-server
      deviceMap = next;
    } catch {
      if (viewCurrent(gen, server)) deviceMap = {};
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
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) {
      badges = {};
      return;
    }
    try {
      const raw = await invoke<Record<string, MemberBadge>>("get_badges", { server });
      if (!viewCurrent(gen, server)) return; // a badge is granted by one server, not carried between
      const map: Record<string, MemberBadge> = {};
      for (const [fp, b] of Object.entries(raw)) {
        const ok = sanitizeBadge(b);
        if (ok) map[fp] = ok;
      }
      badges = map;
    } catch {
      if (viewCurrent(gen, server)) badges = {};
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
              : font === "cute"
                ? "font-cute"
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
  function fxClass(effect: string): string {
    // The original accent-derived gradient predates the stack codec. Keep rendering it for
    // profiles that have not opened and re-saved the new editor yet.
    if (effect === "gradient") return "fx-gradient";
    return nameEffectClasses(decodeNameEffects(effect));
  }

  function fxStyle(effect: string): string {
    if (effect === "gradient") return "";
    return nameEffectStyle(decodeNameEffects(effect));
  }
  const nameSegmenter = typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;
  function nameLetters(name: string): string[] {
    return nameSegmenter
      ? Array.from(nameSegmenter.segment(name), (part) => part.segment)
      : Array.from(name);
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

  async function revealOlderMessages() {
    const node = messagesEl;
    if (!node || messageWindow.start <= 0 || expandingMessageWindow) return;
    expandingMessageWindow = true;
    const previousHeight = node.scrollHeight;
    const previousTop = node.scrollTop;
    messageWindow = revealOlder(messageWindow, messages.length, CHAT_WINDOW_STEP);
    await tick();
    // Retain the reader's visual anchor after inserting rows above the viewport.
    if (messagesEl === node) node.scrollTop = previousTop + node.scrollHeight - previousHeight;
    expandingMessageWindow = false;
  }
  function revealNewerMessages() {
    messageWindow = revealNewer(messageWindow, messages.length, CHAT_WINDOW_STEP);
  }
  function onChatScroll() {
    const node = messagesEl;
    if (!node) return;
    // Do not auto-expand a short, non-scrollable list: it would defeat the DOM bound on unusual
    // compact themes. The explicit edge control remains available in that case.
    if (
      node.scrollTop < 80 &&
      node.scrollHeight > node.clientHeight + 40 &&
      messageWindow.start > 0
    ) {
      void revealOlderMessages();
    }
    if (nearScrollBottom(node.scrollTop, node.clientHeight, node.scrollHeight) && messageWindow.end < messages.length) {
      revealNewerMessages();
      chatStickToBottom = false;
      return;
    }
    chatStickToBottom =
      messageWindow.end >= messages.length &&
      nearScrollBottom(node.scrollTop, node.clientHeight, node.scrollHeight);
  }

  $effect(() => {
    void messages;
    void messageWindow;
    if (!chatStickToBottom) return;
    tick().then(() => {
      if (messagesEl && chatStickToBottom && messageWindow.end >= messages.length) {
        messagesEl.scrollTop = messagesEl.scrollHeight;
      }
    });
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
    // onlineMembers is NOT cleared here: clearServerView already drops it synchronously on every
    // switch, and this effect flushes after the switch's reads may have resolved, so clearing it
    // again here could discard a roster that had already landed.
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
    void statuses;
    void files;
    void emojiUrls;
    void emojiSize;
    void events; // a card for an event that has only just synced
    void wikiPages;
    void wikiMap;
    void ensureWikiMap(); // fills the page summaries the wiki cards below want
    void profiles; // cards name the author, so a renamed member re-reads
    void view; // switching tabs destroys + recreates this DOM (fresh, unresolved placeholders)
    void inboxView; // returning from the inbox recreates the chat DOM too
    tick().then(() => {
      resolveMedia(messagesEl);
      resolveRemoteMedia(messagesEl);
      resolveEmoji(messagesEl);
      resolveRefCards(messagesEl);
      resolveMedia(statusEl);
      resolveRemoteMedia(statusEl);
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
        resolveRemoteMedia(wikiEl);
        resolveEmoji(wikiEl);
        resolveWikiLinks(wikiEl);
        resolveRefCards(wikiEl);
        decorateWikiHeadings(wikiEl);
      } else if (wikiPreview) {
        resolveMedia(wikiPreviewEl);
        resolveRemoteMedia(wikiPreviewEl);
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
      await addSharedFile(file, "emoji", sz ? `${code}~${sz}` : code, file.type || "image/png");
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
  function restoreReloaded(reloaded: Reloaded[]) {
    servers = reloaded.map((r) => ({
      id: r.server,
      name: r.name,
      channels: r.channels?.length ? r.channels : [{ id: r.channel, name: "general" }],
      active: r.channel,
      unread: [],
      invite: r.invite,
      dot: false,
      isDm: r.is_dm,
    }));
    locked = false;
    try { sessionStorage.removeItem("catcoms.explicit-lock"); } catch { /* best effort */ }
    const firstServer = servers.find((s) => !s.isDm) ?? servers[0];
    // Drafts/read boundaries must land before switchServer restores the active composer and
    // snapshots its divider. The native actors are already running, so this is only local UI state.
    const continuityGeneration = ++uiStateLoadGeneration;
    void loadUiContinuity(continuityGeneration).finally(() => {
      if (continuityGeneration === uiStateLoadGeneration && !locked && firstServer) void switchServer(firstServer.id);
    });
    refreshAllDmRequests();
    loadInbox();
    refreshAllServerIcons();
  }

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
      restoreReloaded(reloaded);
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
    // Lock wins over an in-progress secret change and drops every transient secret first.
    vaultChangeCurrent = "";
    vaultChangeFirst = "";
    vaultChangeMismatch = false;
    vaultChangeError = "";
    vaultChangeStep = "";
    clearUnlockEntry();
    // Capture the very latest composer value before clearing the screen. Send the immutable JSON
    // argument immediately (without awaiting it) so locking stays visually instant while the
    // native vault writer can finish after the sensitive JS state has been dropped.
    const key = chanKey();
    if (key) {
      if (draft.trim()) drafts[key] = draft;
      else delete drafts[key];
    }
    clearTimeout(uiStateSaveTimer);
    clearTimeout(inboxTimer);
    if (inboxIdle !== undefined && "cancelIdleCallback" in window) window.cancelIdleCallback(inboxIdle);
    inboxIdle = undefined;
    const continuityJson = uiStateReady ? JSON.stringify({ version: 1, drafts, readMarks }) : null;
    try { sessionStorage.setItem("catcoms.explicit-lock", "1"); } catch { /* best effort */ }
    if (inCall) leaveVoice(); // never leave a hot mic behind a lock screen
    void invoke("lock_session", { uiStateJson: continuityJson }).catch((e) => console.warn("Session locked; final UI continuity save failed", e));
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
    tickerReceipts = new Set(); // receipts can include wiki/page ids; do not retain them behind lock
    servers = [];
    beginViewSwitch(); // a read still in flight must not land behind the lock
    activeServerId = null;
    dmHome = false;
    inboxView = false;
    // One definition of "every window onto a group's contents", not two that drift. The hand-rolled
    // list here used to miss the sanitized message-render cache and wikiMap, both of which hold
    // plaintext, along with roles, livery, badges and the rest.
    clearServerView();
    inboxItems = [];
    newsItems = [];
    serverIcons = {};
    delivery = {};
    draft = "";
    drafts = {};
    readMarks = {};
    uiStateReady = false;
    uiStateSaveFailed = false;
    uiStateLoadGeneration += 1;
    clearTimeout(uiStateSaveTimer);
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
    joinReplyReady = null;
    try {
      const { hex, turn } = unwrapInvite(joinInvite);
      const previewMatchesCode = joinPreviewCode === hex;
      if (!previewMatchesCode) {
        joinPreview = await invoke<InvitePreview>("preview_invite", { inviteHex: hex });
        joinPreviewCode = hex;
        joinSwitchboardConsent = false;
      }
      const assistedAction = assistedJoinAction(
        previewMatchesCode,
        joinPreview?.switchboards ?? 0,
        joinSwitchboardConsent,
      );
      // Do not let the click that first reveals the extra-member privacy boundary also cross it.
      if (assistedAction === "preview") return;
      const r = await invoke<Found>("join_server", {
        inviteHex: hex,
        displayName,
        isDm: false,
        allowSwitchboards: assistedAction === "switchboard",
      });
      if (turn) storeServerTurn(r.server, turn); // inherit the operator's shared TURN
      addServer(r, displayName);
      joinInvite = "";
      joinPreview = null;
      joinPreviewCode = "";
      joinSwitchboardConsent = false;
      joinReplyReady = null;
    } catch (e) {
      joinReplyReady = null;
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function applyJoinReply(replace = false) {
    const server = activeServerId;
    if (server === null || !joinReplyInput.trim()) return;
    joinReplyApplying = true;
    joinReplyNeedsReplace = false;
    error = "";
    try {
      const applied = await invoke<{ helper: boolean }>("apply_join_reply", { server, code: joinReplyInput.trim(), replace });
      notice = applied.helper
        ? "Connection reply accepted. Dialling as a member helper; only the admission handshake will be forwarded."
        : "Connection reply accepted. Dialling the joiner now; keep both apps open.";
      joinReplyInput = "";
    } catch (e) {
      const message = String(e);
      if (joinReplyNeedsReplacement(message)) joinReplyNeedsReplace = true;
      error = message;
    } finally {
      joinReplyApplying = false;
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
      // Derive my profile name from the current profile or fall back to "me"
      const myProfileName = (pName.trim() || name).trim() || "me";
      const r = await invoke<Found>("found_server", { displayName: myProfileName, advertise, relay, rendezvous, isDm: true, serverName: name });
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
      // Derive my profile name from the current profile or fall back to "me"
      const myProfileName = (pName.trim() || name).trim() || "me";
      const r = await invoke<Found>("join_server", { inviteHex: dmInvite.trim(), displayName: myProfileName, isDm: true, allowSwitchboards: false, serverName: name });
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
    const channels = r.channels?.length ? r.channels : [{ id: r.channel, name: "general" }];
    servers = [
      ...servers,
      { id: r.server, name, channels, active: r.channel, unread: [], invite: "", dot: false, isDm: r.is_dm },
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
      // Derive my profile name from the current profile or fall back to "me"
      const myProfileName = (pName.trim() || name).trim() || "me";
      const r = await invoke<Found>("found_server", { displayName: myProfileName, advertise, relay, rendezvous, isDm: true, serverName: name });
      // Add the DM to the list without switching away from the current server.
      servers = [
        ...servers,
        { id: r.server, name, channels: r.channels?.length ? r.channels : [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: "", dot: false, isDm: true },
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

  // Refresh pending DM requests across all non-DM servers (for inbox aggregation).
  async function refreshAllDmRequests() {
    try {
      const allReqs: DmRequest[] = [];
      for (const s of servers.filter((s) => !s.isDm)) {
        try {
          const reqs = await invoke<{ from_fp: string; from_name: string; invite: string }[]>("get_dm_requests", { server: s.id });
          allReqs.push(...reqs.map((r) => ({ server: s.id, ...r })));
        } catch {
          /* a server that's gone / mid-shutdown: ignore */
        }
      }
      dmRequests = allReqs;
    } catch {
      /* ignore */
    }
  }

  // Accept a friend request: join the DM group, then clear the request on the carrying server.
  async function acceptDmRequest(req: DmRequest) {
    busy = true;
    error = "";
    try {
      // Derive my profile name from the current profile or fall back to "me"
      const myProfileName = (pName.trim() || req.from_name).trim() || "me";
      const r = await invoke<Found>("join_server", { inviteHex: req.invite, displayName: myProfileName, isDm: true, allowSwitchboards: false, serverName: req.from_name });
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
    if (dmList.length) {
      // Sequenced, not raced: switchServer merges the target DM's own requests in, so running the
      // cross-server aggregate first would have it replaced or not depending on IPC timing.
      void switchServer(dmList[0].id).then(refreshAllDmRequests);
    } else {
      refreshAllDmRequests();
      beginViewSwitch();
      activeServerId = null;
      clearServerView(); // no active group: drop the previous server's stale messages/roster/etc.
    }
  }
  // Reset every collection the panes render for the active group, synchronously. Called when there
  // is no active group (the empty DM-home placeholder) and at the top of every switch: without it
  // the previous group's messages, roster, files and branding stay on screen for the whole
  // round-trip, which reads as the switch not having taken.
  function clearServerView() {
    view = "chat";
    moderationSelected = new Set();
    moderationDeleteArmed = false;
    moderationAnchor = "";
    caseEvidence = new Set();
    moderationUserFilter = "";
    storageHealth = null;
    storageRepairNote = "";
    messages = [];
    messageRenderCache.clear();
    messageWindow = { start: 0, end: 0 };
    messageWindowScope = "";
    chatStickToBottom = true;
    channelTopic = "";
    delivery = {};
    roster = [];
    members = 0;
    onlineMembers = new Set();
    profiles = {};
    // Roles gate the privileged surfaces, so the empty map is the safe transient: unprivileged
    // until this group's own roles resolve.
    roles = {};
    badges = {};
    deviceMap = {};
    files = [];
    hasPeers = false;
    wikiPinned = new Set();
    wikiPages = [];
    wikiMap = {}; // name -> body: the previous server's page CONTENT, not just its names
    wikiMapFor = null;
    wikiPending = [];
    wikiReviewDays = WIKI_REVIEW_UNKNOWN; // NOT 0: see the constant, zero is a real policy
    wikiHistory = [];
    showWikiHistory = false;
    wikiHistorySel = "";
    statuses = [];
    events = [];
    moderation = { events: [], votes: [] };
    moderationMessages = [];
    moderationLoading = false;
    groupLoading = false; // a clear with no load behind it (no active group) is not "loading"
    // Livery is server branding: leaving it up paints the group you left over the one you opened.
    // followLiveryNow already drops to the default theme for DM-home and the inbox, so the brief
    // default between servers is that same transition rather than a new kind of flicker.
    livery = emptyLivery();
    liveryCursorUrl = "";
    liveryLoaded = false;
    liveryDraftFor = null;
    // Surfaces that render one server's data but were never closed on a switch: the settings
    // takeover would keep rendering server A's pages against server B, and the wiki review queue
    // would open on B still asserting A's (now empty) backlog.
    showServerSettings = false;
    wikiReviewOpen = false;
    // Custom emoji are per-server but emojiUrls is keyed by CODE, so two servers defining the same
    // :code: would show the first one's image on the second.
    emojiUrls = {};
    joinAttempts = []; // who tried to join THIS server: never carried to the next one
    // The wiki editor and the chat/fileshare affordances below used to be reset only by
    // switchServer, so the paths that end with no active group (leaving your last server, empty
    // DM-home, locking) kept a page body and its drafts in memory as plaintext.
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
    const gen = beginViewSwitch(); // everything still in flight for the old group is now stale
    activeServerId = id;
    inboxView = false;
    spaceOpen = false; // navigating anywhere leaves the orbit view behind
    // Drop the previous group's plaintext render artifacts, window anchors and every collection
    // the panes render, synchronously, before the scope of trust changes.
    clearServerView();
    groupLoading = true;
    const s = servers.find((x) => x.id === id);
    if (s) s.dot = false;
    // A brand already read this session repaints once, straight to the right one, instead of
    // default-then-brand a round-trip later. refreshLivery still confirms it below.
    const cachedLivery = s && !s.isDm ? liveryCache.get(id) : undefined;
    if (cachedLivery) {
      livery = cachedLivery;
      liveryLoaded = true;
    }
    dmHome = s?.isDm ?? false; // a DM keeps us in DM-home; a server leaves it
    showNewDm = false;
    showAddFriend = false;
    if (showSearch) closeSearch();
    notice = "";
    refreshDmRequests(id); // pick up any friend request that arrived over this server
    // Each server has its own wiki + fileshare; clearServerView above dropped the previous one's.
    storageHealth = storageHealthCache.get(id) ?? null; // a cached report shows without re-probing
    acceptCallsHere = loadAccept(id); // this server's call-notification preference
    loadServerSoundPreferences(id); // local message/mention/news overrides for this server
    loadSrvTurn(id); // this server's operator-set TURN (for the Server-settings editor)
    loadLiveryOptOut(id); // whether the user opted out of this server's livery
    loadVerified(id); // this server's locally-verified members
    loadDraftFor(chanKey()); // restore this server's active-channel draft
    captureDivider(); // snapshot the read boundary for this server's active channel
    // One barrier, not two. refreshModeration used to be awaited AFTER this batch because it read
    // the privileged message corpus and so had to see this server's roles first; it now fetches
    // only the case/vote state everyone needs in chat, and the corpus loads with the surface that
    // uses it, so it joins the batch and the switch loses a whole serial round-trip.
    // The roles-first guarantee still holds, but it now rests on clearServerView having set
    // `view = "chat"` and `roles = {}` above: withCorpus is false here, so the corpus is
    // structurally unreachable during a switch. Anything that later keeps the moderation surface
    // open ACROSS a switch must restore the explicit ordering rather than rely on that.
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
      refreshModeration(),
      refreshWikiPages(),
    ]);
    if (!viewCurrent(gen, id)) return; // moved on while this group was loading
    groupLoading = false;
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
      pEffects = decodeNameEffects(pEffect);
      styleUndo = [];
      styleRedo = [];
      lastStyleHistoryKey = "";
      pDescription = me.description ?? "";
      pBubble = me.bubble ?? "";
      pAvatar = me.avatar || "";
      pBanner = me.banner || "";
      const frame = parseMessageFrame(pBubble);
      const bg = BUB_GRAD_RE.exec(frame.surface);
      if (bg && !BUBBLE_PRESETS.some((b) => b.value === frame.surface)) {
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
      else {
        beginViewSwitch();
        activeServerId = null;
        clearServerView(); // leaving the last one must not leave its messages on screen
      }
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

  async function refreshChannels(server: number) {
    try {
      const channels = await invoke<Channel[]>("get_channels", { server });
      const s = servers.find((x) => x.id === server);
      if (!s || !channels.length) return;
      s.channels = channels;
      const next = reconcileActiveChannel(channels, s.active);
      if (!next.changed) return;
      if (server === activeServerId) {
        // switchTo owns the move for the group on screen: reassigning `active` on its own would
        // leave the previous channel's messages, topic, delivery ticks and draft sitting under the
        // new channel's name. Not awaited here, and `s.active` is deliberately NOT set first:
        // switchTo stashes the outgoing draft under the channel being left.
        void switchTo(next.active);
      } else {
        s.active = next.active; // not on screen: there is no loaded state to reconcile
      }
    } catch {
      // Older backend: retain the locally-known list.
    }
  }

  // `keepSearch` is set when the search itself is driving the move (jumping to a hit in another
  // channel): everything else closes the search bar, as before.
  async function switchTo(id: string, keepSearch = false) {
    if (!cur) return;
    saveDraftFor(chanKey()); // stash the current channel's draft before leaving it
    // Channel-scoped content, dropped before the move rather than when the read returns: the
    // group's own state (roster, files, branding) is unchanged by a channel hop and stays put.
    messages = [];
    messageRenderCache.clear();
    messageWindow = { start: 0, end: 0 };
    messageWindowScope = "";
    chatStickToBottom = true;
    channelTopic = "";
    delivery = {};
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
    const gen = viewGeneration;
    const server = activeServerId;
    const channel = cur?.active;
    if (server === null || !channel) {
      channelTopic = "";
      return;
    }
    try {
      const topic = await invoke<string>("get_channel_topic", { server, channel });
      if (!viewCurrent(gen, server) || cur?.active !== channel) return;
      channelTopic = topic;
    } catch {
      if (!viewCurrent(gen, server) || cur?.active !== channel) return;
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

  let refreshRevision = 0;
  async function refresh(animateArrivals = false) {
    if (!cur || !cur.active || activeServerId === null) return;
    const server = activeServerId;
    const channel = cur.active;
    const revision = ++refreshRevision;
    try {
      const previousMessages = messages;
      const previous = new Set(previousMessages.map((message) => message.id));
      const next = await invoke<Msg[]>("get_messages", { server, channel });
      // A slow response from the conversation we just left must never populate the new one.
      if (revision !== refreshRevision || activeServerId !== server || cur?.active !== channel) return;
      const nextScope = chatScopeKey(server, channel);
      const nextWindow = reconcileChatWindow(
        previousMessages,
        next,
        messageWindow,
        chatStickToBottom,
        messageWindowScope !== nextScope,
      );
      messages = next;
      messageWindow = nextWindow;
      messageWindowScope = nextScope;
      if (animateArrivals) {
        // Own posts already animate at optimistic insertion; excluding them prevents the
        // acknowledged, server-assigned id from replaying the entrance a second time.
        markMessageArrivals(next.filter((message) => message.author !== myFp && !previous.has(message.id)).map((message) => message.id));
      }
      advanceReadMark();
    } catch (e) {
      error = String(e);
    }
  }

  // A single network merge can emit several channel notifications. Serialize and coalesce their
  // full snapshots so the bridge never has multiple large `get_messages` payloads racing for the
  // same view. Direct navigation/send acknowledgements still call `refresh` at their own explicit
  // completion points.
  const channelEventRefresh = new CoalescedAsyncRefresh(refresh);

  // Delivery states for OWN messages (docs/design-delivery-states.md). Evidence-based lower
  // bounds: a member is counted only once it has provably built on the message, so counts
  // only rise and 0 means "no proof yet", never "failed". Red is reserved for the one true
  // negative signal we have: no peers reachable at all.
  type DeliveryState = { id: string; delivered: number; reachable: number };
  let delivery = $state<Record<string, DeliveryState>>({});
  async function refreshDelivery() {
    const gen = viewGeneration;
    const server = activeServerId;
    const channel = cur?.active;
    if (server === null || !channel) {
      delivery = {};
      return;
    }
    try {
      const list = await invoke<DeliveryState[]>("get_delivery", { server, channel });
      if (!viewCurrent(gen, server) || cur?.active !== channel) return;
      const map: Record<string, DeliveryState> = {};
      for (const s of list) map[s.id] = s;
      delivery = map;
    } catch {
      if (!viewCurrent(gen, server) || cur?.active !== channel) return;
      delivery = {}; // older backend or closed actor: ticks simply don't render
    }
  }
  // The gutter tick for one of your messages: ✕ no peers · ◌ no proof yet · ~ partial ·
  // ✓ all reachable confirmed · ✓✓ the whole roster confirmed.
  function deliveryTick(m: Msg): { g: string; cls: string; tip: string } | null {
    if (m.author !== myFp || !m.id) return null;
    if (m.id.startsWith("pending:"))
      return { g: "◌", cls: "d-pending", tip: "Saving this message locally…" };
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
    if (mi !== lastOwnIdx) return null;
    const t = deliveryTick(m);
    if (!t || !m.id) return null;
    if (t.cls === "d-pending") return { g: t.g, label: "sending…", cls: t.cls, tip: t.tip };
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
    const gen = viewGeneration;
    const id = activeServerId;
    if (id === null) return;
    try {
      // Two independent reads, so they go together; the roster is the source of myFp, which half
      // of canModerate derives from, so this is the one that most needs the generation and not
      // just the id: an A -> B -> A hop passes an id-only check with an older snapshot.
      const [r, online] = await Promise.all([
        invoke<Member[]>("get_members", { server: id }),
        invoke<string[]>("get_online_members", { server: id }),
      ]);
      if (!viewCurrent(gen, id)) return;
      roster = r;
      members = r.length;
      onlineMembers = new Set(online);
    } catch (e) {
      if (viewCurrent(gen, id)) error = String(e);
    }
  }
  async function refreshProfiles() {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) return;
    try {
      const list = await invoke<Prof[]>("get_profiles", { server });
      if (!viewCurrent(gen, server)) return; // names and avatars from the group you left
      const map: Record<string, Prof> = {};
      for (const p of list) map[p.fingerprint] = p;
      profiles = map;
    } catch (e) {
      if (viewCurrent(gen, server)) error = String(e);
    }
  }
  // The call surfaces' own copy of the room server's profiles. Deliberately a separate fetch
  // from refreshProfiles: that one is keyed to whatever server is being viewed and is wiped on
  // every switch, which is exactly the behaviour the call chrome must not inherit.
  async function refreshCallProfiles() {
    const server = callServer;
    if (server === null) return;
    try {
      const list = await invoke<Prof[]>("get_profiles", { server });
      if (callServer !== server) return; // left, or moved rooms, mid-fetch
      const map: Record<string, Prof> = {};
      for (const p of list) map[p.fingerprint] = p;
      callProfiles = map;
    } catch {
      // A missing profile renders as a fingerprint, which is honest and still identifies the
      // peer. Surfacing an error banner over a cosmetic lookup would be worse than the gap.
    }
  }
  async function refreshFiles() {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) return;
    try {
      // Wiki-pinned content addresses are derived fresh from the wiki on the backend each call: a
      // file embedded in a live page never drops out of circulation, whatever its expiry says. It
      // is a second round-trip, so it rides alongside the listing rather than after it.
      const [listing, pinned] = await Promise.allSettled([
        invoke<{ files: UiFile[]; has_peers: boolean }>("get_files", { server }),
        invoke<string[]>("get_wiki_pinned_cids", { server }),
      ]);
      if (!viewCurrent(gen, server)) return; // another group's shared files
      // Applied independently: a failing pin lookup must not blank the listing it only decorates.
      if (listing.status === "fulfilled") {
        files = listing.value.files;
        hasPeers = listing.value.has_peers;
      } else {
        error = String(listing.reason);
      }
      if (pinned.status === "fulfilled") wikiPinned = new Set(pinned.value);
    } catch (e) {
      if (viewCurrent(gen, server)) error = String(e);
    }
  }
  async function refreshStorageHealth() {
    if (activeServerId === null) return;
    const server = activeServerId;
    const cached = storageHealthCache.get(server);
    if (cached) {
      storageHealth = cached;
      return;
    }
    storageChecking = true;
    try {
      const report = await invoke<StorageHealth>("get_storage_health", { server });
      storageHealthCache.set(server, report);
      if (activeServerId === server) storageHealth = report;
    } catch (e) {
      if (activeServerId === server) error = String(e);
    } finally {
      // Keyed: a late probe from the server you left must not clear the spinner for the one you
      // opened, which is still reading.
      if (activeServerId === server) storageChecking = false;
    }
  }
  async function repairStorage() {
    if (activeServerId === null || storageRepairing) return;
    storageRepairing = true;
    storageRepairNote = "";
    try {
      const server = activeServerId;
      const result = await invoke<{ attempted_chunks: number; recovered_chunks: number; health: StorageHealth }>(
        "repair_storage", { server },
      );
      storageHealthCache.set(server, result.health);
      if (activeServerId === server) storageHealth = result.health;
      storageRepairNote = result.attempted_chunks
        ? `Checked ${result.attempted_chunks} damaged or missing chunks; recovered ${result.recovered_chunks}.`
        : "Everything referenced by this server already verifies.";
      await refreshFiles();
    } catch (e) {
      error = String(e);
    } finally {
      storageRepairing = false;
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
    if (dl && (dl.status === "downloading" || dl.status === "verifying"))
      return { cls: "downloading", icon: "↓", label: `Downloading ${Math.round(dl.progress * 100)}%` };
    if (dl && (dl.status === "queued" || dl.status === "waiting"))
      return hasPeers
        ? { cls: "downloading", icon: "↓", label: "Waiting for source" }
        : { cls: "offline", icon: "○", label: "No peers online" };
    if (f.total > 0 && f.held >= f.total)
      return { cls: "local", icon: "●", label: "On this device" };
    if (f.held > 0)
      return { cls: "partial", icon: "◐", label: `Partial ${f.held}/${f.total}` };
    if (hasPeers) return { cls: "remote", icon: "○", label: "Downloadable" };
    return { cls: "offline", icon: "○", label: "No peers online" };
  }
  async function refreshStatuses() {
    const gen = viewGeneration;
    const srv = activeServerId;
    if (srv === null) return;
    try {
      // Read the "what we already knew" set before the await, and note that a switch empties it:
      // arriving at a server must not announce its whole status wall as if it had just landed.
      const knownStatuses = new Set(statuses.map((s) => s.id));
      const hadStatuses = statuses.length > 0;
      const next = await invoke<Msg[]>("get_statuses", { server: srv });
      if (!viewCurrent(gen, srv)) return;
      statuses = next;
      if (hadStatuses) {
        for (const st of statuses) {
          if (knownStatuses.has(st.id)) continue;
          pushTicker("status", srv, `status:${srv}:${st.id}`, `${nameOf(st.author)}: ${msgSnippet(st.text, 60)}`, () =>
            void goSurface(srv, "status"),
          );
        }
      }
    } catch (e) {
      if (viewCurrent(gen, srv)) error = String(e);
    }
  }
  // --- Diagnostics: the join log, the connectivity report, and the debug log ---------------
  //
  // Three surfaces for one problem: a join that fails tells nobody anything. The join log is the
  // OPERATOR's view of why this node refused an inbound join (the wire answer to the joiner
  // stays opaque on purpose); the connectivity report is the JOINER's view of what their own app
  // tried; the debug log is the fallback for everything neither explains.
  let joinAttempts = $state<JoinAttempt[]>([]);
  let joinLogCopied = $state(false);
  let connectivity = $state<Connectivity | null>(null);
  type SwitchboardStatus = {
    offered: boolean;
    eligible: boolean;
    online: { fingerprint: string; addresses: number }[];
    reason: string;
  };
  let switchboardStatus = $state<SwitchboardStatus | null>(null);
  let switchboardBusy = $state(false);
  const switchboardRefreshGeneration = new Map<number, number>();
  let connectivityRefreshGeneration = 0;
  let connCopied = $state(false);
  let debugLog = $state<{ enabled: boolean; active: boolean; dir: string; file: string } | null>(null);
  let debugLogBusy = $state(false);

  async function refreshJoinAttempts() {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) return;
    try {
      const next = await invoke<JoinAttempt[]>("get_join_attempts", { server });
      if (!viewCurrent(gen, server)) return; // fingerprints of people who tried to join elsewhere
      joinAttempts = next;
    } catch (e) {
      if (viewCurrent(gen, server)) error = String(e);
    }
  }
  async function refreshConnectivity() {
    const generation = ++connectivityRefreshGeneration;
    try {
      const refreshed = await invoke<Connectivity>("get_connectivity");
      connectivity = withOrderedConnectivity(
        connectivity,
        refreshed,
        generation,
        connectivityRefreshGeneration,
      );
    } catch {
      // A build without the command (or a locked app) simply has nothing to show; the panel
      // says so rather than raising an error over a diagnostic. An older failure must not erase
      // a newer successful snapshot.
      connectivity = withOrderedConnectivity(
        connectivity,
        null,
        generation,
        connectivityRefreshGeneration,
      );
    }
  }
  async function refreshSwitchboards() {
    const server = activeServerId;
    if (server === null) {
      switchboardStatus = null;
      return;
    }
    const generation = (switchboardRefreshGeneration.get(server) ?? 0) + 1;
    switchboardRefreshGeneration.set(server, generation);
    try {
      const status = await invoke<SwitchboardStatus>("get_switchboard_status", { server });
      switchboardStatus = withOrderedSwitchboardStatus(
        switchboardStatus,
        activeServerId,
        server,
        status,
        generation,
        switchboardRefreshGeneration.get(server) ?? 0,
      );
    } catch (e) {
      error = String(e);
    }
  }
  async function toggleSwitchboard() {
    const server = activeServerId;
    if (server === null || switchboardBusy) return;
    switchboardBusy = true;
    const generation = (switchboardRefreshGeneration.get(server) ?? 0) + 1;
    switchboardRefreshGeneration.set(server, generation);
    try {
      const status = await invoke<SwitchboardStatus>("set_switchboard_offered", {
        server,
        offered: !switchboardStatus?.offered,
      });
      switchboardStatus = withOrderedSwitchboardStatus(
        switchboardStatus,
        activeServerId,
        server,
        status,
        generation,
        switchboardRefreshGeneration.get(server) ?? 0,
      );
    } catch (e) {
      error = String(e);
    } finally {
      switchboardBusy = false;
    }
  }
  async function refreshDebugLog() {
    try {
      debugLog = await invoke("get_debug_logging");
    } catch {
      debugLog = null;
    }
  }
  async function toggleDebugLog(on: boolean) {
    debugLogBusy = true;
    try {
      debugLog = await invoke("set_debug_logging", { enabled: on });
    } catch (e) {
      error = String(e);
    } finally {
      debugLogBusy = false;
    }
  }
  async function copyJoinLog() {
    await copyText(formatJoinLog(joinAttempts));
    joinLogCopied = true;
    setTimeout(() => (joinLogCopied = false), 1500);
  }
  async function copyConnectivity() {
    await copyText(formatConnectivity(connectivity));
    connCopied = true;
    setTimeout(() => (connCopied = false), 1500);
  }
  // Local time on screen (the operator is looking at their own clock); the copied text uses UTC,
  // because whoever they paste it to usually is not in this timezone.
  function fmtLocal(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "";
    return new Date(ms).toLocaleString();
  }

  // Load each diagnostic when its page is actually opened: none of them is cheap enough to poll
  // and none is interesting until someone is looking at it.
  $effect(() => {
    if (showServerSettings && serverSettingsPage === "joinlog") {
      void activeServerId;
      refreshJoinAttempts();
    }
  });
  $effect(() => {
    if (showSettings && settingsPage === "diagnostics") {
      refreshDebugLog();
      refreshConnectivity();
    }
  });
  // The onboarding panel: refresh whenever the create/join screen is showing and an attempt has
  // just finished (`busy` falling back to false is exactly that moment).
  $effect(() => {
    if (servers.length === 0 || showAdd) {
      void busy;
      refreshConnectivity();
      refreshDebugLog();
    }
  });

  async function refreshRoles() {
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) return;
    try {
      const next = await invoke<Record<string, string>>("get_roles", { server });
      // The sharpest of the stale writes: canModerate derives from this map, so a late answer
      // from the server you left would hand you moderator chrome on the one you opened.
      if (!viewCurrent(gen, server)) return;
      roles = next;
      // A demotion closes the privileged surface immediately; hiding only the sidebar entry
      // would leave a stale moderation page reachable through history/navigation.
      if (!canModerate && view === "moderation") switchView("chat");
    } catch (e) {
      if (viewCurrent(gen, server)) error = String(e);
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
    if (v === "moderation" && !canModerate) {
      toast("Moderation is available to this server's owner and admins", "info", 3500);
      v = "chat";
    }
    view = v;
    if (v === "wiki") refreshWiki();
    if (v === "files") refreshFiles(); // re-evaluate availability each time the tab opens
    if (v === "status") refreshStatuses(); // a read that failed during the switch gets a retry here
    if (v === "events") refreshEvents();
    if (v === "moderation") refreshModeration();
    if (v === "storage" || v === "downloads") refreshStorageHealth();
    if (v === "connectivity") void Promise.all([refreshConnectivity(), refreshSwitchboards()]);
  }

  // Delegated click handler for rendered rich text: [[wiki links]] navigate to the wiki tab.
  async function handleRichClick(e: MouseEvent) {
    const target = e.target as HTMLElement | null;
    // Spoilers and censored effects use the same local, reader-controlled reveal. First click
    // reveals them and never follows a link that happens to sit inside.
    const sp = target?.closest("[data-spoiler], [data-text-fx='censor']") as HTMLElement | null;
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
    // The inbox and news lists render text from every server, but a [[link]] or a file/event/status
    // chip inside rendered text carries no server with it: resolving one against whatever group
    // happens to sit behind the overlay opens the WRONG server's wiki or fileshare, and a wiki page
    // that server lacks opens its editor. Jump to the item's own server first.
    if (inboxView && target?.closest("[data-wikilink],[data-file-cid],[data-event-id],[data-status-id]")) {
      e.preventDefault();
      toast("Open the item's server first to follow its links", "info", 4000);
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
      return;
    }
    // Never let an external anchor navigate this webview: doing so replaces the entire Mewtual
    // UI and leaves the window effectively softlocked. The native side accepts http(s) only and
    // hands the URL to the user's normal browser without invoking a shell.
    const link = target?.closest("a[href]") as HTMLAnchorElement | null;
    if (link) {
      e.preventDefault();
      const url = link.href || link.getAttribute("href") || "";
      try {
        await invoke("open_external_url", { url });
      } catch (err) {
        error = String(err);
      }
    }
  }

  // Svelte action: delegate clicks inside a rendered-rich-text container (attaches the
  // listener imperatively, so no a11y warning for a click on a non-interactive container).
  function richClicks(node: HTMLElement) {
    const h = (e: Event) => handleRichClick(e as MouseEvent);
    const c = (e: Event) => handleRichContext(e as MouseEvent);
    // Link cards, spoilers, and censored effects are focusable buttons, so they must open from
    // the keyboard too; the synthesized click uses the same delegated handler as a real one.
    const k = (e: Event) => {
      const ev = e as KeyboardEvent;
      if (ev.key !== "Enter" && ev.key !== " ") return;
      const card = (ev.target as HTMLElement | null)?.closest(".ref-card, [data-spoiler], [data-text-fx='censor']") as HTMLElement | null;
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

  // Text effects are rendered in several independent surfaces, so their viewport/pointer
  // behaviour is delegated once at the document. One-shot entrances are armed the first time each
  // rendered instance becomes visible; a WeakSet prevents scroll-jiggling from replaying them.
  // Effect audio uses the shared app preference and is short, deterministic, and Full-mode only.
  function mountTextEffectRuntime() {
    const played = new WeakSet<HTMLElement>();
    const observed = new WeakSet<HTMLElement>();
    const pendingSpeakese = new Set<HTMLElement>();
    const speakeseRevealTimers = new Map<HTMLElement, ReturnType<typeof setTimeout>>();
    const pendingRedTruth = new Set<HTMLElement>();
    const redTruthRevealTimers = new Map<HTMLElement, ReturnType<typeof setTimeout>>();
    let pointerFx: HTMLElement | null = null;
    let speakeseAudioUntil = 0;
    let redTruthAudioUntil = 0;

    function effectVisible(el: HTMLElement) {
      const rect = el.getBoundingClientRect();
      return el.isConnected && rect.bottom > 0 && rect.top < innerHeight && rect.right > 0 && rect.left < innerWidth;
    }

    function scheduleSpeakese(el: HTMLElement) {
      if (document.documentElement.dataset.textEffects !== "full" || !soundOn) return;
      const units = [...el.querySelectorAll<HTMLElement>(".fx-speakese-unit")]
        .filter((unit) => (unit.textContent ?? "").trim())
        .slice(0, MAX_SPEAKESE_BLIPS);
      if (!units.length) return;
      try {
        const ctx = audioCtx;
        if (!ctx || ctx.state !== "running") { pendingSpeakese.add(el); return; }
        if (ctx.currentTime < speakeseAudioUntil) return;
        const start = ctx.currentTime + 0.025;
        const plan = speakeseSoundPlan(units.map((unit) => Number(unit.dataset.fxTone ?? 0)), start);
        speakeseAudioUntil = start + units.length * SPEAKESE_STEP_SECONDS;
        plan.forEach((blip) => {
          const osc = ctx.createOscillator();
          const gain = ctx.createGain();
          osc.type = blip.waveform;
          osc.frequency.setValueAtTime(blip.frequency, blip.at);
          osc.frequency.exponentialRampToValueAtTime(blip.endFrequency, blip.at + 0.055);
          gain.gain.setValueAtTime(0.0001, blip.at);
          gain.gain.exponentialRampToValueAtTime(blip.peak, blip.at + 0.007);
          gain.gain.exponentialRampToValueAtTime(0.0001, blip.at + 0.061);
          osc.connect(gain).connect(ctx.destination);
          osc.start(blip.at);
          osc.stop(blip.stop);
        });
      } catch {
        // Visual reveal remains useful where Web Audio is unavailable or gesture-gated.
      }
    }

    function scheduleRedTruth(el: HTMLElement) {
      if (document.documentElement.dataset.textEffects !== "full" || !soundOn) return;
      try {
        const ctx = audioCtx;
        if (!ctx || ctx.state !== "running") { pendingRedTruth.add(el); return; }
        if (ctx.currentTime < redTruthAudioUntil) return;
        const plan = redTruthSoundPlan(ctx.currentTime + 0.025);
        redTruthAudioUntil = plan.sweep.stop;
        plan.strike.forEach((note) => {
          const osc = ctx.createOscillator();
          const gain = ctx.createGain();
          osc.type = note.waveform;
          osc.frequency.setValueAtTime(note.frequency, note.at);
          osc.frequency.exponentialRampToValueAtTime(note.endFrequency, note.stop - 0.01);
          gain.gain.setValueAtTime(0.0001, note.at);
          gain.gain.exponentialRampToValueAtTime(note.peak, note.at + 0.003);
          gain.gain.exponentialRampToValueAtTime(0.0001, note.stop - 0.006);
          osc.connect(gain).connect(ctx.destination);
          osc.start(note.at);
          osc.stop(note.stop);
        });

        const sweep = plan.sweep;
        const duration = sweep.stop - sweep.at;
        const buffer = ctx.createBuffer(1, Math.ceil(duration * ctx.sampleRate), ctx.sampleRate);
        const samples = buffer.getChannelData(0);
        for (let index = 0; index < samples.length; index += 1) samples[index] = redTruthNoiseSample(index);
        const noise = ctx.createBufferSource();
        const highpass = ctx.createBiquadFilter();
        const bandpass = ctx.createBiquadFilter();
        const washGain = ctx.createGain();
        noise.buffer = buffer;
        highpass.type = "highpass";
        highpass.frequency.setValueAtTime(sweep.highpassFrequency, sweep.at);
        bandpass.type = "bandpass";
        bandpass.Q.setValueAtTime(0.58, sweep.at);
        bandpass.frequency.setValueAtTime(sweep.startFrequency, sweep.at);
        bandpass.frequency.exponentialRampToValueAtTime(sweep.crestFrequency, sweep.crest);
        bandpass.frequency.exponentialRampToValueAtTime(sweep.endFrequency, sweep.stop);
        washGain.gain.setValueAtTime(0.0001, sweep.at);
        washGain.gain.exponentialRampToValueAtTime(sweep.peak * 0.3, sweep.at + 0.13);
        washGain.gain.exponentialRampToValueAtTime(sweep.peak, sweep.crest);
        washGain.gain.exponentialRampToValueAtTime(0.0001, sweep.stop);
        noise.connect(highpass).connect(bandpass).connect(washGain).connect(ctx.destination);
        noise.start(sweep.at);
        noise.stop(sweep.stop);
      } catch {
        // The visual seal and reveal do not depend on Web Audio support.
      }
    }

    function revealPending(
      pending: Set<HTMLElement>,
      timers: Map<HTMLElement, ReturnType<typeof setTimeout>>,
      schedule: ((effect: HTMLElement) => void) | undefined = undefined,
    ) {
      const waiting = [...pending];
      const visible = waiting.filter(effectVisible);
      pending.clear(); // never pile several authored voices of the same kind on top of one another
      for (const effect of waiting) {
        const timer = timers.get(effect);
        if (timer) clearTimeout(timer);
        timers.delete(effect);
        effect.classList.add("fx-play");
      }
      if (schedule && visible[0]) schedule(visible[0]);
    }

    function flushTextEffectAudio() {
      if (document.documentElement.dataset.textEffects !== "full" || !soundOn) {
        revealPending(pendingSpeakese, speakeseRevealTimers);
        revealPending(pendingRedTruth, redTruthRevealTimers);
        return;
      }
      try {
        audioCtx ??= new AudioContext();
        const playPending = () => {
          if (!audioCtx || audioCtx.state !== "running") return;
          revealPending(pendingSpeakese, speakeseRevealTimers, scheduleSpeakese);
          revealPending(pendingRedTruth, redTruthRevealTimers, scheduleRedTruth);
        };
        if (audioCtx.state === "running") playPending();
        else void audioCtx.resume().then(playPending).catch(() => { /* retry on the next gesture */ });
      } catch {
        /* Web Audio is optional; letter playback still runs. */
      }
    }

    function startOneShot(
      el: HTMLElement,
      pending: Set<HTMLElement>,
      timers: Map<HTMLElement, ReturnType<typeof setTimeout>>,
      schedule: (effect: HTMLElement) => void,
    ) {
      const wantsSound = document.documentElement.dataset.textEffects === "full" && soundOn;
      if (!wantsSound || audioCtx?.state === "running") {
        el.classList.add("fx-play");
        if (wantsSound) schedule(el);
        return;
      }
      // Keep sound and letters together when the webview is waiting for a trusted gesture. If no
      // gesture comes promptly, reveal silently so authored text can never remain inaccessible.
      pending.add(el);
      const timer = setTimeout(() => {
        timers.delete(el);
        if (pending.delete(el)) el.classList.add("fx-play");
      }, 420);
      timers.set(el, timer);
    }

    const intersection = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target as HTMLElement;
        intersection.unobserve(el);
        if (played.has(el)) continue;
        played.add(el);
        if (el.dataset.textFx === "speakese") {
          startOneShot(el, pendingSpeakese, speakeseRevealTimers, scheduleSpeakese);
        } else if (el.dataset.textFx === "red-truth") {
          startOneShot(el, pendingRedTruth, redTruthRevealTimers, scheduleRedTruth);
        }
      }
    }, { threshold: 0.18 });

    function discover(root: ParentNode) {
      const effects = root instanceof HTMLElement && root.matches("[data-text-fx]")
        ? [root]
        : [...root.querySelectorAll<HTMLElement>("[data-text-fx]")];
      for (const effect of effects) {
        effect.querySelectorAll<HTMLElement>(".text-fx-unit").forEach((unit, index) =>
          unit.style.setProperty("--fx-i", String(index)));
        if (["speakese", "red-truth"].includes(effect.dataset.textFx ?? "") && !observed.has(effect)) {
          observed.add(effect);
          // Picker/settings previews demonstrate the entrance, but selecting or browsing effects
          // must never make sound. Authored content is the only observer-driven audio source.
          if (effect.closest(".text-fx-selection-bar, .text-fx-catalog, .text-fx-key-preview")) {
            effect.classList.add("fx-play");
          } else {
            intersection.observe(effect);
          }
        }
      }
    }

    const mutations = new MutationObserver((records) => {
      for (const record of records) for (const node of record.addedNodes) {
        if (node instanceof HTMLElement) discover(node);
      }
    });
    discover(document.body);
    mutations.observe(document.body, { childList: true, subtree: true });

    const onPointerMove = (event: PointerEvent) => {
      const next = document.documentElement.dataset.textEffects === "full"
        ? (event.target as HTMLElement | null)?.closest<HTMLElement>("[data-text-fx]:not([data-text-fx='censor'])") ?? null
        : null;
      const entered = next !== pointerFx;
      if (pointerFx && pointerFx !== next) {
        pointerFx.classList.remove("fx-pointer");
        pointerFx.classList.remove("fx-petal-burst");
        pointerFx.style.removeProperty("--fx-px");
        pointerFx.style.removeProperty("--fx-py");
      }
      pointerFx = next;
      if (!next) return;
      const rect = next.getBoundingClientRect();
      const x = rect.width ? (event.clientX - rect.left) / rect.width - 0.5 : 0;
      const y = rect.height ? (event.clientY - rect.top) / rect.height - 0.5 : 0;
      next.style.setProperty("--fx-px", x.toFixed(3));
      next.style.setProperty("--fx-py", y.toFixed(3));
      next.classList.add("fx-pointer");
      if (cherryBlossomShouldBurst(entered, next.dataset.textFx)) {
        // One bloom per visit: movement within the same words may shift them, but only leaving and
        // entering again re-arms their petal burst.
        next.classList.add("fx-petal-burst");
      }
    };
    // Web Audio must be resumed from a trusted interaction in Chromium/WebView. Warm the shared
    // context on normal app input, then flush any visible one-shot effect waiting for it.
    const onSoundGesture = () => flushTextEffectAudio();
    // A textarea keeps its old selection after losing focus. Close the Aa palette when the user
    // goes elsewhere, while allowing its own buttons and the source editor to keep it alive.
    const onPalettePointerDown = (event: PointerEvent) => {
      if (!textEffectTarget) return;
      const target = event.target as HTMLElement | null;
      const region: TextEffectPointerRegion = target?.closest(".text-fx-selection-bar")
        ? "palette"
        : target?.closest(".text-fx-trigger")
          ? "trigger"
          : target === textEffectElement(textEffectTarget)
            ? "editor"
            : "outside";
      if (dismissTextEffectPalette(showTextEffectCatalog, region)) textEffectTarget = null;
    };
    // `select` is not emitted consistently when typing replaces a selection. The document-level
    // event follows caret changes too, so a collapsed selection cannot leave the Aa bar behind.
    const onDocumentSelectionChange = () => {
      const target = activeTextEffectTarget();
      if (target) onTextEffectSelection(target);
    };
    document.addEventListener("pointermove", onPointerMove, { passive: true });
    document.addEventListener("pointerdown", onSoundGesture, true);
    document.addEventListener("keydown", onSoundGesture, true);
    document.addEventListener("pointerdown", onPalettePointerDown, true);
    document.addEventListener("selectionchange", onDocumentSelectionChange);
    return () => {
      intersection.disconnect();
      mutations.disconnect();
      for (const timer of speakeseRevealTimers.values()) clearTimeout(timer);
      for (const timer of redTruthRevealTimers.values()) clearTimeout(timer);
      document.removeEventListener("pointermove", onPointerMove);
      document.removeEventListener("pointerdown", onSoundGesture, true);
      document.removeEventListener("keydown", onSoundGesture, true);
      document.removeEventListener("pointerdown", onPalettePointerDown, true);
      document.removeEventListener("selectionchange", onDocumentSelectionChange);
    };
  }

  // Resolve placeholders only in the row Svelte has just mounted or updated. Resource-index
  // changes still use the coarse effect above because an older placeholder can become resolvable,
  // but ordinary chat arrivals no longer rescan every historical row four times.
  function resolveChatRow(node: HTMLLIElement, _message: Msg) {
    let live = true;
    let generation = 0;
    const resolve = () => {
      const current = ++generation;
      queueMicrotask(() => {
        if (!live || current !== generation || !node.isConnected) return;
        void resolveMedia(node);
        void resolveRemoteMedia(node);
        void resolveEmoji(node);
        void resolveRefCards(node);
      });
    };
    resolve();
    return {
      update(_next: Msg) {
        resolve();
      },
      destroy() {
        live = false;
        generation += 1;
      },
    };
  }

  // A Scan layer remains clipped inside its author's frame, but every enabled Scan shares the
  // message viewport's coordinates and animation phase. The result is one channel-wide beam,
  // revealed only while it crosses frames whose authors opted into the layer.
  function channelScan(node: HTMLUListElement) {
    // Do not attach a ResizeObserver, MutationObserver, scroll listener, or animation-sync work
    // while the live-chat frame rollout is paused. The editor preview remains available and saved
    // frame values remain untouched.
    if (!CHAT_MESSAGE_FRAMES_ENABLED) return;
    let raf = 0;
    const observedBodies = new Set<HTMLElement>();

    const syncAnimations = (rows: HTMLLIElement[]) => {
      const animations: Animation[] = [];
      for (const row of rows) {
        for (const layer of row.querySelectorAll<HTMLElement>(":scope > .m-body > .message-frame-fx > .frame-fx-scan")) {
          animations.push(...layer.getAnimations());
        }
      }
      const duration = Number(animations[0]?.effect?.getTiming().duration);
      if (!Number.isFinite(duration) || duration <= 0) return;
      const phase = performance.now() % duration;
      for (const animation of animations) animation.currentTime = phase;
    };

    const measure = () => {
      raf = 0;
      const rows = [...node.children].filter((child): child is HTMLLIElement => child instanceof HTMLLIElement);
      const viewport = node.getBoundingClientRect();
      const viewportTop = viewport.top + node.clientTop;
      const viewportBottom = viewportTop + node.clientHeight;
      const scanRows: HTMLLIElement[] = [];
      const measured: { row: HTMLLIElement; body: HTMLElement; rect: DOMRect }[] = [];
      for (const row of rows) {
        row.style.removeProperty("--frame-scan-offset");
        row.style.removeProperty("--frame-scan-height");
        const body = row.querySelector<HTMLElement>(":scope > .m-body");
        if (!body) continue;
        if (!observedBodies.has(body)) {
          resize.observe(body);
          observedBodies.add(body);
        }
        measured.push({ row, body, rect: body.getBoundingClientRect() });
      }
      const visible = measured.filter(({ rect }) => rect.bottom > viewportTop && rect.top < viewportBottom);
      const scanTop = visible.length ? Math.max(viewportTop, visible[0].rect.top) : viewportTop;
      const scanBottom = visible.length
        ? Math.min(viewportBottom, visible.at(-1)?.rect.bottom ?? viewportBottom)
        : viewportBottom;
      const scanHeight = Math.max(1, scanBottom - scanTop);
      for (const { row, body, rect } of measured) {
        if (!body.querySelector(":scope > .message-frame-fx > .frame-fx-scan")) continue;
        const geometry = messageFrameScanGeometry(scanTop, scanHeight, rect.top);
        row.style.setProperty("--frame-scan-offset", `${geometry.offset}px`);
        row.style.setProperty("--frame-scan-height", `${geometry.height}px`);
        scanRows.push(row);
      }
      for (const body of observedBodies) {
        if (node.contains(body)) continue;
        resize.unobserve(body);
        observedBodies.delete(body);
      }
      syncAnimations(scanRows);
    };

    const schedule = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(measure);
    };
    const resize = new ResizeObserver(schedule);
    const mutations = new MutationObserver(schedule);
    resize.observe(node);
    mutations.observe(node, { childList: true, subtree: true, attributes: true, attributeFilter: ["class"] });
    node.addEventListener("scroll", schedule, { passive: true });
    schedule();
    return {
      destroy() {
        cancelAnimationFrame(raf);
        resize.disconnect();
        mutations.disconnect();
        node.removeEventListener("scroll", schedule);
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
      toast("Copied to clipboard", "ok", 1800);
    } catch {
      toast("Clipboard unavailable: select and copy the text manually", "err", 3500);
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
    if (s.invite) items.push({ label: "Copy invite", icon: "⧉", onSelect: () => void copyFreshInvite(s.id) });
    items.push({ label: "Server settings", icon: "⚙", onSelect: () => void openServerSettings(s.id) });
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
        { label: "Open announcement", icon: "⊞", onSelect: () => openStatusRef(id) },
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
  // Just the page names, and they belong to every switch rather than only to opening the wiki tab.
  // Two things outside that tab read them: the ref cards in chat, which say "not created yet" for a
  // name the list lacks, and openWikiPage, which opens a page in EDIT mode when the list lacks it.
  // An empty list therefore does not merely look wrong, it drops a [[link]] click into the editor.
  async function refreshWikiPages() {
    const gen = viewGeneration;
    const srv = activeServerId;
    if (srv === null) return;
    try {
      const knownPages = wikiPages;
      // The review policy rides along with the page list. It gates the rename/delete controls and
      // the eager-create path, both of which are reachable from a [[link]] in chat without the
      // wiki tab ever being opened, so it cannot wait for refreshWiki.
      const [listed, reviewDays] = await Promise.allSettled([
        invoke<string[]>("get_wiki_pages", { server: srv }),
        invoke<number>("get_wiki_review_days", { server: srv }),
      ]);
      if (!viewCurrent(gen, srv)) return;
      if (reviewDays.status === "fulfilled") wikiReviewDays = reviewDays.value;
      if (listed.status !== "fulfilled") {
        error = String(listed.reason);
        return;
      }
      const next = listed.value;
      wikiPages = next;
      // A page list arriving for the first time is not news, it is just the list; only pages that
      // appear against a list we already had get announced. A switch empties the list, so arriving
      // at a server never announces its whole wiki.
      if (knownPages.length) {
        for (const pg of next) {
          if (knownPages.includes(pg)) continue;
          pushTicker("wiki", srv, `wiki:${srv}:${pg}`, pg, () => void goWikiPage(srv, pg));
        }
      }
    } catch (e) {
      if (viewCurrent(gen, srv)) error = String(e);
    }
  }
  async function refreshWiki() {
    const gen = viewGeneration;
    const srv = activeServerId;
    if (srv === null) return;
    await refreshWikiPages();
    if (!viewCurrent(gen, srv)) return;
    // Four independent reads, applied independently: one failing (an older backend without
    // get_wiki_review_days, say) must not discard the three that answered.
    const [map, meta, reviewDays, pending] = await Promise.allSettled([
      invoke<Record<string, string>>("get_wiki_map", { server: srv }),
      invoke<Record<string, string>>("get_wiki_meta", { server: srv }),
      invoke<number>("get_wiki_review_days", { server: srv }),
      invoke<UiWikiPending[]>("get_wiki_pending", { server: srv }),
    ]);
    if (!viewCurrent(gen, srv)) return;
    if (map.status === "fulfilled") {
      wikiMap = map.value;
      wikiMapFor = srv;
    }
    if (meta.status === "fulfilled") wikiMeta = meta.value;
    if (reviewDays.status === "fulfilled") wikiReviewDays = reviewDays.value;
    if (pending.status === "fulfilled") wikiPending = pending.value;
    try {
      // Reload the open page only if it still exists and the user isn't mid-edit. Re-checked after
      // the read as well as before it: an edit begun while the body was in flight owns the buffer.
      const page = activeWikiPage;
      if (page && !wikiDirty && wikiPages.includes(page)) {
        const body = await invoke<string>("get_wiki_page", { server: srv, name: page });
        if (viewCurrent(gen, srv) && activeWikiPage === page && !wikiDirty) wikiBody = body;
      }
      // Keep an open history browser current (an approval elsewhere adds a revision). Reached even
      // when the body reload above was skipped or its answer discarded.
      if (showWikiHistory && page) {
        const history = await invoke<UiWikiRev[]>("get_wiki_history", { server: srv, page });
        if (viewCurrent(gen, srv) && activeWikiPage === page) wikiHistory = history;
      }
    } catch (e) {
      if (viewCurrent(gen, srv)) error = String(e);
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
    const gen = viewGeneration;
    const server = activeServerId;
    if (server === null) return;
    if (wikiDirty && activeWikiPage && activeWikiPage !== name) wikiDrafts.set(activeWikiPage, wikiBody);
    if (showInsert && insertTarget === "wiki") closeInsert();
    try {
      let body = await invoke<string>("get_wiki_page", { server, name });
      // Follow #REDIRECT [[Target]] pages Wikipedia-style (bounded; only to pages that exist),
      // remembering where we came from so the notice can link back to the redirect itself.
      let from = "";
      if (!opts.noRedirect) {
        for (let hops = 0; hops < 3; hops++) {
          const target = parseRedirect(body);
          if (!target || target === name || !wikiPages.includes(target)) break;
          from = from || name;
          name = target;
          if (!viewCurrent(gen, server)) return; // do not follow one server's redirect into another
          body = await invoke<string>("get_wiki_page", { server, name });
        }
      }
      // The whole point of the guard: without it this page's text lands in the editor of whatever
      // server you moved to, and because that server's wikiPages lacks the name it opens in EDIT
      // mode, one Ctrl+S from publishing one server's wiki content into another's.
      if (!viewCurrent(gen, server)) return;
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
      if (!wikiPages.includes(name) && mayEditWikiStructure(wikiReviewDays, canModerate)) {
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
          const cid = await addSharedFile(file, `wiki/${activeWikiPage}`);
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
    e.stopPropagation();
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
  const inviteRefreshGeneration = new Map<number, number>();

  async function updateInviteFor(
    server: number,
    mintFresh = false,
  ): Promise<string | undefined> {
    const generation = (inviteRefreshGeneration.get(server) ?? 0) + 1;
    inviteRefreshGeneration.set(server, generation);
    try {
      // Keep both command names as static literals. Besides making the security boundary easy to
      // audit, this lets the command-ledger test prove every frontend IPC call is registered.
      const invite = (
        mintFresh
          ? await invoke<string | null>("mint_invite_fresh", { server })
          : await invoke<string | null>("get_invite", { server })
      ) ?? "";
      const latest = inviteRefreshGeneration.get(server) ?? 0;
      if (generation !== latest) return undefined;
      servers = withOrderedRefreshedInvite(servers, server, invite, generation, latest);
      return invite;
    } catch (e) {
      if (generation === (inviteRefreshGeneration.get(server) ?? 0)) error = String(e);
      return undefined;
    }
  }

  async function refreshInviteFor(server: number): Promise<string | undefined> {
    return updateInviteFor(server);
  }

  async function refreshInvite() {
    const server = activeServerId;
    if (server === null) return;
    await refreshInviteFor(server);
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

  // Read a File as raw base64 (strips the data: prefix), reporting the browser-side read before
  // the backend starts sealing/storing chunks. The Transfers UI reserves its first 10% for this.
  function readBase64(
    file: File,
    onProgress: ((done: number, total: number) => void) | undefined = undefined,
  ): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(new Error("could not read file"));
      reader.onprogress = (e) => onProgress?.(e.loaded, e.lengthComputable ? e.total : file.size);
      reader.onload = () => {
        const r = reader.result;
        resolve(typeof r === "string" ? (r.split(",")[1] ?? "") : "");
      };
      reader.readAsDataURL(file);
    });
  }

  // Decode only metadata before retaining a custom notification tone. This keeps an accidentally
  // selected song from becoming a minutes-long alert and gives unsupported codecs a clear error
  // at import time rather than a silent failure when the next message arrives.
  function customToneDuration(file: File): Promise<number> {
    return new Promise((resolve, reject) => {
      const url = URL.createObjectURL(file);
      const audio = new Audio();
      let settled = false;
      const finish = (duration: number | null) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        audio.removeAttribute("src");
        URL.revokeObjectURL(url);
        if (duration === null) reject(new Error("could not decode that audio file"));
        else resolve(duration);
      };
      const timer = setTimeout(() => finish(null), 5000);
      audio.preload = "metadata";
      audio.onloadedmetadata = () => finish(audio.duration);
      audio.onerror = () => finish(null);
      audio.src = url;
    });
  }

  async function importCustomTone(
    scope: "global" | "server",
    kind: NotificationSoundKind,
    files: FileList | null,
  ) {
    const file = files?.[0];
    if (!file) return;
    const mime = customToneMime(file.type, file.name);
    // Reject obvious type/size failures before asking the decoder to touch the bytes.
    const earlyError = customToneError(mime, file.size, 1);
    if (earlyError) { toast(earlyError, "err", 4500); return; }
    try {
      const duration = await customToneDuration(file);
      const validationError = customToneError(mime, file.size, duration);
      if (validationError || !mime) {
        toast(validationError ?? "Unsupported audio file", "err", 4500);
        return;
      }
      const stored: StoredTone = {
        name: file.name.trim().slice(0, 96) || "custom tone",
        mime,
        dataUrl: `data:${mime};base64,${await readBase64(file)}`,
      };
      if (scope === "global") {
        globalSoundPrefs[kind].custom = stored;
        globalSoundPrefs[kind].tone = "custom";
        saveGlobalSoundPrefs();
      } else {
        serverSoundPrefs[kind].custom = stored;
        serverSoundPrefs[kind].tone = "custom";
        saveServerSoundPrefs();
      }
      toast(`${SOUND_LABELS[kind].title} tone set to ${stored.name}`, "ok", 2600);
    } catch (e) {
      toast(`Could not import tone: ${String(e)}`, "err", 4500);
    }
  }

  function removeCustomTone(scope: "global" | "server", kind: NotificationSoundKind) {
    if (scope === "global") {
      globalSoundPrefs[kind].custom = null;
      globalSoundPrefs[kind].tone = "default";
      saveGlobalSoundPrefs();
    } else {
      serverSoundPrefs[kind].custom = null;
      serverSoundPrefs[kind].tone = "inherit";
      saveServerSoundPrefs();
    }
  }

  // The one upload path used by files, embeds, wiki attachments, event art and custom emoji.
  // Keeping it central means every group upload gets the same progress and terminal state.
  async function addSharedFile(
    file: File,
    path: string,
    name = file.name,
    mime = file.type || "application/octet-stream",
  ): Promise<string> {
    if (activeServerId === null) throw new Error("no server selected");
    const server = activeServerId;
    const uploadId = crypto.randomUUID();
    const key = uploadKey(server, uploadId);
    const started = Date.now();
    uploads[key] = {
      server,
      id: uploadId,
      name,
      path,
      size: file.size,
      done: 0,
      total: Math.max(1, Math.ceil(file.size / TRANSFER_CHUNK_BYTES)),
      status: "reading",
      progress: 0,
      updatedAt: started,
      ts: started,
    };
    try {
      const data = await readBase64(file, (done, total) => {
        const u = uploads[key];
        if (u && u.status === "reading") {
          u.progress = total > 0 ? 0.1 * done / total : 0;
          u.updatedAt = Date.now();
        }
      });
      const u = uploads[key];
      if (u) {
        u.status = "uploading";
        u.progress = Math.max(u.progress, 0.1);
        u.updatedAt = Date.now();
      }
      const cid = await invoke<string>("add_file", { server, name, mime, path, data, uploadId });
      if (uploads[key]) {
        uploads[key].status = "done";
        uploads[key].progress = 1;
        uploads[key].done = uploads[key].total;
        uploads[key].updatedAt = Date.now();
      }
      return cid;
    } catch (e) {
      if (uploads[key]) {
        uploads[key].status = "failed";
        uploads[key].error = String(e);
        uploads[key].updatedAt = Date.now();
      }
      throw e;
    }
  }

  // Share a file into the Files-tab's current folder.
  async function uploadFile(fileList: FileList | null) {
    if (!fileList?.length || activeServerId === null) return;
    uploading = true;
    try {
      for (const file of Array.from(fileList)) {
        const tid = toast(`Sharing ${file.name}…`, "info", 0);
        try {
          await addSharedFile(file, folder);
          updateToast(tid, `Shared ${file.name}`, "ok");
        } catch (e) {
          updateToast(tid, `Sharing ${file.name} failed: ${e}`, "err", 9000);
        }
      }
      await refreshFiles();
    } finally {
      uploading = false;
    }
  }

  function onFilesDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    void uploadFile(e.dataTransfer?.files ?? null);
  }

  function onEventDrop(e: DragEvent) {
    e.preventDefault();
    dragOver = false;
    void pickEventImage(e.dataTransfer?.files ?? null);
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
          const cid = await addSharedFile(file, myEmbedFolder);
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

  function remoteImage(url: string, alt: string): HTMLImageElement {
    const img = document.createElement("img");
    img.src = url;
    img.alt = alt;
    img.loading = "lazy";
    img.referrerPolicy = "no-referrer";
    img.className = "embed-media embed-image remote-image";
    img.title = "Remote image · click to view full size";
    img.dataset.remoteImage = "1";
    return img;
  }

  // Resolve explicit remote-image markdown and bare direct image/Giphy links. The renderer emits
  // only inert placeholders; raw member HTML still cannot inject an image element.
  function resolveRemoteMedia(container: HTMLElement | undefined) {
    if (!container) return;
    for (const span of Array.from(container.querySelectorAll<HTMLElement>("[data-remote-url]:not([data-resolved])"))) {
      span.dataset.resolved = "1";
      const url = safeRemoteUrl(span.dataset.remoteUrl ?? "");
      if (url) span.replaceWith(remoteImage(url, span.dataset.alt ?? "Remote image"));
    }
    for (const a of Array.from(container.querySelectorAll<HTMLAnchorElement>("a[href]:not([data-remote-checked])"))) {
      a.dataset.remoteChecked = "1";
      const url = pastedImageUrl(a.href);
      if (!url) continue;
      a.replaceWith(remoteImage(url, a.textContent?.trim() || "Remote image"));
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
      kicker: `Announcement · ${relDay(post.ts, Date.now())}`,
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
    // Whether `body` is an answer at all. Without this an unread map reads as "every page is
    // empty", which is a claim about the server rather than about what we have loaded.
    const mapped = wikiMapFor === activeServerId;
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
      body: target ? undefined : plainSummary(body, 220) || (mapped ? "(empty page)" : undefined),
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
    const server = activeServerId;
    const key = dlKey(server, f.cid);
    const started = Date.now();
    const held = Math.min(f.held, f.total);
    downloads[key] = {
      server,
      cid: f.cid,
      name: f.name,
      author: f.author,
      status: "queued",
      progress: f.total > 0 ? held / f.total : 0,
      done: held,
      total: f.total,
      heldBefore: held,
      bytesDone: f.total > 0 ? Math.round(f.size * held / f.total) : 0,
      bytesTotal: f.size,
      networkBytesDone: 0,
      speed: 0,
      lastRateAt: started,
      updatedAt: started,
      ts: started,
    };
    try {
      const base64 = await invoke<string>("download_file", { server, cid: f.cid });
      const saved = await saveGroupDownload(invoke, f.name, base64);
      if (downloads[key]) {
        Object.assign(downloads[key], completedDownload(downloads[key], saved, Date.now()));
      }
      const notice = downloadSavedNotice(f.name, saved);
      toast(notice.text, notice.kind, notice.ms);
      refreshFiles(); // the file's chunks are now held locally: update its availability
    } catch (e) {
      error = String(e);
      if (downloads[key]) {
        downloads[key].status = "failed";
        downloads[key].error = String(e);
        downloads[key].updatedAt = Date.now();
      }
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
  // Reading a text/markdown share inline. `fileTextState` is the reader's whole story: nothing to
  // read, fetching, readable, over the inline cap (a fetch the reader has to ask for), not
  // actually text, or the fetch failed.
  let fileText = $state("");
  let fileTextLines = $state(0);
  let fileTextState = $state<"none" | "loading" | "ready" | "too-big" | "binary" | "error">("none");
  let fileTextMode = $state<"render" | "source">("render"); // markdown only; other text is source
  let fileTextWrap = $state(true); // off for logs and code, where columns carry meaning

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
  // Tracked downloads keyed by file cid, for the Transfers tab + the file-info progress bar. Driven
  // by 'download-progress' events (per-chunk) from the actor. Only EXPLICIT downloads (the Download
  // button) are tracked here: background embed/preview fetches emit progress but create no entry.
  type DownloadInfo = {
    server: number;
    cid: string;
    name: string;
    author: string; // the uploader (the file's source)
    provider?: string; // the live serving peer's fingerprint, when bytes came over the network
    status: "queued" | "waiting" | "downloading" | "verifying" | "done" | "failed";
    progress: number; // 0..1
    done: number;
    total: number;
    heldBefore: number;
    bytesDone: number;
    bytesTotal: number;
    networkBytesDone: number;
    speed: number;
    lastRateAt: number;
    updatedAt: number;
    savedPath?: string;
    error?: string;
    ts: number;
  };
  // Keyed by `${server}:${cid}` so a download is scoped to its server (the same content cid can
  // exist on two servers, and switching servers must not show the other's transfers).
  let downloads = $state<Record<string, DownloadInfo>>({});
  const dlKey = (server: number, cid: string) => `${server}:${cid}`;
  type UploadInfo = {
    server: number;
    id: string;
    name: string;
    path: string;
    size: number;
    done: number;
    total: number;
    status: "reading" | "uploading" | "publishing" | "done" | "failed";
    progress: number; // 0..1; 0..0.1 is the webview read, the remainder is backend work
    updatedAt: number;
    error?: string;
    ts: number;
  };
  let uploads = $state<Record<string, UploadInfo>>({});
  let transferNow = $state(Date.now());
  const uploadKey = (server: number, id: string) => `${server}:${id}`;

  // The active server's transfers, newest first. Uploads are first-class rows instead of a
  // transient button label, so a completed share stays visibly completed until cleared.
  let downloadList = $derived(
    Object.values(downloads)
      .filter((d) => d.server === activeServerId)
      .sort((a, b) => b.ts - a.ts)
  );
  let uploadList = $derived(
    Object.values(uploads)
      .filter((u) => u.server === activeServerId)
      .sort((a, b) => b.ts - a.ts)
  );
  type TransferRow =
    | (DownloadInfo & { direction: "download"; key: string })
    | (UploadInfo & { direction: "upload"; key: string });
  let transferList = $derived(
    [
      ...downloadList.map((d): TransferRow => ({ ...d, direction: "download", key: `download:${d.cid}` })),
      ...uploadList.map((u): TransferRow => ({ ...u, direction: "upload", key: `upload:${u.id}` })),
    ].sort((a, b) => b.ts - a.ts)
  );
  let activeTransfers = $derived(
    transferList.filter((t) =>
      t.status === "queued" || t.status === "waiting" || t.status === "downloading" ||
      t.status === "verifying" || t.status === "reading" || t.status === "uploading" ||
      t.status === "publishing"
    ).length
  );
  let movingTransfers = $derived(
    transferList.filter((t) => transferIsActive(t) && transferConnected(t)).length
  );
  let waitingTransfers = $derived(Math.max(0, activeTransfers - movingTransfers));
  let finishedTransfers = $derived(transferList.length - activeTransfers);
  let failedTransfers = $derived(transferList.filter((t) => t.status === "failed").length);
  function clearFinishedTransfers() {
    for (const [k, d] of Object.entries(downloads)) {
      if (d.server === activeServerId && (d.status === "done" || d.status === "failed"))
        delete downloads[k];
    }
    for (const [k, u] of Object.entries(uploads)) {
      if (u.server === activeServerId && (u.status === "done" || u.status === "failed"))
        delete uploads[k];
    }
  }

  function transferConnected(t: TransferRow): boolean {
    if (t.direction === "upload" || t.status === "done") return true;
    return onlineCount > 1 || t.done >= t.total ||
      (t.status === "downloading" && transferNow - t.updatedAt < 3_000);
  }

  function transferIsActive(t: TransferRow): boolean {
    if (t.direction === "download" && t.status === "downloading") {
      return transferNow - t.updatedAt < 3_000;
    }
    return t.status === "reading" || t.status === "uploading" || t.status === "publishing" ||
      t.status === "verifying";
  }

  function transferPieceStates(t: TransferRow): TransferPiece[] {
    return transferPieces(
      t.total,
      t.done,
      transferIsActive(t) && t.status !== "publishing" && t.status !== "verifying",
      transferConnected(t),
      t.status === "failed",
      t.status === "done",
    );
  }

  function transferTone(t: TransferRow): string {
    if (t.status === "done") return "complete";
    if (t.status === "failed" || !transferConnected(t)) return "error";
    if (t.status === "downloading" && !transferIsActive(t)) return "waiting";
    if (t.status === "queued" || t.status === "waiting" || t.status === "reading" ||
        t.status === "publishing" || t.status === "verifying") return "waiting";
    return "active";
  }

  function transferStatus(t: TransferRow): string {
    const pct = Math.round(t.progress * 100);
    if (t.status === "done") return t.direction === "upload" ? "✓ Available" : "✓ Saved";
    if (t.status === "failed") {
      return t.direction === "download" && onlineCount <= 1 ? "No connection" : "✕ Failed";
    }
    if (t.direction === "upload") {
      if (t.status === "reading") return `Preparing ${pct}%`;
      if (t.status === "publishing") return "Publishing…";
      return `Processing ${pct}%`;
    }
    if (!transferConnected(t)) return "No connection";
    if (t.status === "queued" || t.status === "waiting") return "Waiting for source…";
    if (t.status === "verifying") return "Verifying…";
    if (!transferIsActive(t)) return "Waiting for next chunk…";
    return `Receiving ${pct}%`;
  }

  function transferHover(t: TransferRow): string {
    const lines = [
      `Status: ${transferStatus(t).replace(/[✓✕…]/g, "").trim()}`,
      `Chunks ready: ${Math.min(t.done, t.total)} / ${t.total}`,
    ];
    if (t.direction === "download") {
      lines.push(`Data ready: ${formatBytes(t.bytesDone)} / ${formatBytes(t.bytesTotal)}`);
      if (t.savedPath) lines.push(`Saved to: ${t.savedPath}`);
      lines.push(`Source: ${t.provider ? `${nameOf(t.provider)}${transferConnected(t) ? "" : " (last source; now offline)"}` : transferConnected(t) ? "finding a reachable member" : "no member connected"}`);
      if (t.speed > 0 && t.status === "downloading") lines.push(`Speed: ${formatRate(t.speed)}`);
      if (t.heldBefore > 0) lines.push(`Already held when started: ${t.heldBefore} chunk${t.heldBefore === 1 ? "" : "s"}`);
    } else {
      lines.push(`File size: ${formatBytes(t.size)}`);
      lines.push("Source: this device");
      lines.push("Availability: members download these chunks on demand");
    }
    if (t.error) lines.push(`Detail: ${t.error}`);
    return lines.join("\n");
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
    fileText = "";
    fileTextLines = 0;
    fileTextMode = "render";
    // `fileTextKind` reads off the listing just assigned, so the reader shows its bar and a
    // "Loading…" line from the first frame rather than an empty frame until the fetch starts.
    fileTextState = fileTextKind ? "loading" : "none";
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
    // Documents, config and source read inline the same way media plays inline.
    if (fileTextKind && fileInfo?.cid === f.cid) await loadFileText(f);
  }

  function closeFileInfo() {
    fileInfo = null;
    fileInfoPreview = "";
    fileInfoPreviewError = false;
    fileInfoAvail = null;
    confirmDeleteCid = "";
    fileInfoUsage = null;
    fileText = "";
    fileTextLines = 0;
    fileTextState = "none";
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

  // Which reader the pane offers below the media block. Gated on `previewKind` so a listing that
  // somehow satisfies both (a .txt stamped image/png, say) shows one preview, not two.
  const fileTextKind = $derived<TextFileKind>(
    fileInfo && !previewKind ? textFileKind(fileInfo.name, fileInfo.mime) : ""
  );
  // A markdown file opens rendered and can be flipped to its source; everything else IS source.
  const fileTextRendered = $derived(fileTextKind === "markdown" && fileTextMode === "render");
  // Memoised so parsing + sanitizing a long document runs when the body changes, not on every
  // repaint of the pane around it (the progress bar under it ticks once per chunk).
  const fileTextHtml = $derived(fileTextRendered ? renderTextDocument(fileText) : "");

  /**
   * Fetch and decode a text share for the reader. Files over the inline cap are not pulled at all
   * until `force` (the "Read it anyway" button): `download_file` returns the whole blob in one
   * base64 string, and a listing may declare up to 256 MiB.
   */
  async function loadFileText(f: UiFile, force = false) {
    const id = activeServerId;
    if (id === null) return;
    if (!force && f.size > TEXT_PREVIEW_MAX_BYTES) {
      fileTextState = "too-big";
      return;
    }
    fileTextState = "loading";
    try {
      const base64 = await invoke<string>("download_file", { server: id, cid: f.cid });
      // Guard against the pane being closed or switched while the fetch was in flight.
      if (fileInfo?.cid !== f.cid) return;
      const decoded = decodeTextFile(Uint8Array.from(atob(base64), (c) => c.charCodeAt(0)));
      if (!decoded.ok) {
        fileTextState = "binary";
        return;
      }
      fileText = decoded.text;
      fileTextLines = decoded.lines;
      fileTextState = "ready";
    } catch {
      // Not held locally and no peer sharing it right now: say so instead of spinning forever.
      if (fileInfo?.cid === f.cid) fileTextState = "error";
    }
  }

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
    void scrollToMatch(idx);
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
    return plainSummary(text, n);
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
      error = "That announcement is no longer in this server's feed.";
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
    saveDraftFor(chanKey());
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
    // No active server means no read mark to measure against. The sole caller already requires
    // server === activeServerId, and its documented fallback is an ordinary-message notification,
    // which is what false gives it. The old key built `null:channel`, which said the same thing
    // by accident rather than on purpose.
    if (!myFp || activeServerId === null) return false;
    const seen = readMarks[chatScopeKey(activeServerId, channel)] ?? 0;
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
    if (locked) return;
    inboxLoading = true;
    try {
      const next = await invoke<InboxEntry[]>("get_inbox");
      if (locked) return; // the lock cleared this list; it carries message text from every server
      inboxItems = next;
    } catch (e) {
      error = String(e);
    } finally {
      inboxLoading = false;
    }
  }
  let inboxTimer: ReturnType<typeof setTimeout> | undefined;
  let inboxIdle: number | undefined;
  function scheduleInboxReload() {
    if (locked) return;
    clearTimeout(inboxTimer);
    if (inboxIdle !== undefined && "cancelIdleCallback" in window) window.cancelIdleCallback(inboxIdle);
    inboxIdle = undefined;
    inboxTimer = setTimeout(() => {
      inboxTimer = undefined;
      if ("requestIdleCallback" in window) {
        inboxIdle = window.requestIdleCallback(() => {
          inboxIdle = undefined;
          void loadInbox();
        }, { timeout: 2_500 });
      } else {
        void loadInbox();
      }
    }, 1_000); // coalesce bursts, then run the cross-server scan off the interaction path
  }
  // An inbox entry is "unseen" until you've read past it in that channel (the same read marks that
  // drive jump-to-unread); resolved against the entry's own server, not the active one.
  function inboxUnseen(it: InboxEntry): boolean {
    return it.ts > (readMarks[chatScopeKey(it.server, it.channel)] ?? 0);
  }
  let inboxUnseenCount = $derived(inboxItems.filter(inboxUnseen).length);
  // The entry's channel name, resolved from the server's known channel list (names are a UI concern).
  function inboxChannelName(it: InboxEntry): string {
    return servers.find((s) => s.id === it.server)?.channels.find((c) => c.id === it.channel)?.name ?? "channel";
  }
  function openInbox() {
    inboxView = true;
    dmHome = false;
    if (newsUnseen) {
      inboxMode = "news";
      newsUnseen = false;
      loadNews();
    }
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
    server: number;
    channel: string;
    pc: RTCPeerConnection;
    dc: RTCDataChannel | null;
    polite: boolean;
    makingOffer: boolean;
    ignoreOffer: boolean;
    lastRetry: number;
  };
  let inCall = $state(false);
  let callMuted = $state(false);
  // Whether this device has a microphone in the room at all, as distinct from having muted one.
  // Being in a room without one is a supported state, not a failed join.
  let micOn = $state(false);
  let callParticipants = $state<string[]>([]); // peer fingerprints, for the call UI
  let callPeerStates = $state<Record<string, string>>({}); // fp -> RTCPeerConnectionState
  // A voice room is per-CHANNEL: the channel id doubles as the call id (for signalling + the media
  // key). You join a channel's room; others see it via presence (below) and join the same room.
  let callChannel = $state(""); // the channel id of my active voice room ("" = not in a call)
  let callChannelName = $state(""); // for the call bar
  // Reactive, unlike the plain binding this replaces: the call chrome has to re-render when the
  // room's server stops being the viewed one, which is the whole point of the fields below.
  let callServer = $state<number | null>(null); // the server the room is on
  let callServerName = $state(""); // the room's server, for chrome that must not say "here"
  let callSelfFp = $state(""); // identity on callServer; the viewed server may change mid-call
  // Names and avatars on the call surfaces MUST resolve against the room's server, never the
  // viewed one. `profiles` is replaced wholesale on every server switch (see refreshProfiles),
  // so reading it from the call bar re-labelled you and every peer the moment you clicked
  // another server: your own name changed under you, and the dock read as though the call had
  // moved with you. This map is fetched once for callServer and is cleared only on leave.
  let callProfiles = $state<Record<string, Prof>>({});
  // True while the user is looking at a different server from the one the call is on. The dock
  // uses it to say where the call actually is instead of silently implying "here".
  let callElsewhere = $derived(inCall && callServer !== null && callServer !== activeServerId);
  // The room's server name. Prefer the live rail entry so a rename mid-call lands; fall back to
  // the name captured at join, which is all we have if the server leaves the rail under us.
  let callSrvLabel = $derived(
    callServer !== null
      ? (servers.find((s) => s.id === callServer)?.name ?? callServerName)
      : "",
  );
  // Which dock slot the voice chrome occupies. Two slots rather than free dragging: the dock is
  // a fixed centred overlay, and a freely draggable surface near the top edge would fight the
  // titlebar's own drag region for no real gain.
  let callDockTop = $state(loadCallSetting("dock", "top") !== "bottom");
  function toggleDockSlot() {
    callDockTop = !callDockTop;
    try {
      localStorage.setItem("catcoms.call.dock", callDockTop ? "top" : "bottom");
    } catch {
      /* storage unavailable */
    }
  }
  // Per-peer media path. Absent means "not known yet": still negotiating, or getStats has not
  // answered. An unknown path renders as nothing at all, never as a guess.
  let peerTransport = $state<Record<string, "direct" | "relayed">>({});
  let secInfoOpen = $state(false); // the "what a relay can see" fold-out in the stage
  // The room folds to the weakest path present: one relayed leg means a relay learns that much
  // of who is talking to whom.
  let roomPath = $derived.by(() => {
    const known = callParticipants
      .map((fp) => peerTransport[fp])
      .filter((t): t is "direct" | "relayed" => !!t);
    if (!known.length) return "";
    return known.includes("relayed") ? "relayed" : "direct";
  });
  let localStream: MediaStream | null = null;
  const callPeers: Record<string, CallPeer> = {};
  // Trickle ICE may beat the offer over independent Tauri invokes. Hold it until that peer has a
  // remote description instead of throwing it away and making the call depend on event timing.
  const waitingIce: Record<string, RTCIceCandidateInit[]> = {};

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
  async function sendSignal(server: number, targetFp: string, msg: Record<string, unknown>): Promise<boolean> {
    try {
      const delivered = await invoke<boolean>("send_call_signal", {
        server,
        targetFp,
        payload: b64enc(JSON.stringify(msg)),
      });
      if (!delivered) console.warn("voice signal had no member route", { server, targetFp, type: msg.type });
      return delivered;
    } catch (e) {
      console.warn("voice signal failed", { server, targetFp, type: msg.type, error: String(e) });
      return false;
    }
  }
  // Send against the CALL server's live roster, never the server currently being viewed. Fetching
  // this small in-memory view also lets a background call pick up newly-reconnected members.
  async function broadcastOn(server: number, selfFp: string, msg: Record<string, unknown>) {
    try {
      const [membersHere, onlineHere] = await Promise.all([
        invoke<Member[]>("get_members", { server }),
        invoke<string[]>("get_online_members", { server }),
      ]);
      const online = new Set(onlineHere);
      for (const m of membersHere) {
        if (m.fingerprint !== selfFp && online.has(m.fingerprint)) {
          void sendSignal(server, m.fingerprint, msg);
        }
      }
    } catch (e) {
      console.warn("voice broadcast could not read its server roster", { server, error: String(e) });
    }
  }
  // Capture the room synchronously: leaveVoice clears global state immediately after sending bye.
  function broadcast(msg: Record<string, unknown>) {
    const server = callServer;
    const selfFp = callSelfFp;
    if (server !== null && selfFp) void broadcastOn(server, selfFp, msg);
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
  async function ensureMic(announce = true): Promise<MediaStream | null> {
    if (localStream) return localStream;
    // Try the remembered input first; a device that has since vanished must not block the call.
    const tries: (MediaTrackConstraints | boolean)[] = micDev
      ? [{ deviceId: { exact: micDev } }, true]
      : [true];
    for (const audio of tries) {
      try {
        localStream = await navigator.mediaDevices.getUserMedia({ audio, video: false });
        void refreshAudioDevices();
        return localStream;
      } catch {
        /* remembered device gone: fall back to the system default */
      }
    }
    // Joining a room does not announce this: no microphone is a perfectly good way to be in a
    // room (the jukebox and the instruments do not need one), so the dock says so in place
    // rather than the app raising it as a failure.
    if (announce) error = "Couldn't access the microphone (permission denied or no device).";
    return null;
  }
  /**
   * Turn the microphone on for a room already joined. Adding a track raises negotiationneeded on
   * every existing peer, and the perfect-negotiation path already handles the renegotiation, so
   * this needs no signalling of its own.
   */
  async function enableMic() {
    if (localStream || !inCall) return;
    const stream = await ensureMic();
    if (!stream) return;
    callMuted = false;
    for (const t of stream.getAudioTracks()) t.enabled = true;
    for (const p of Object.values(callPeers)) {
      for (const t of stream.getTracks()) {
        try { p.pc.addTrack(t, stream); } catch { /* already added on this edge */ }
      }
    }
    addAnalyser("me", stream);
    micOn = true;
    pushInstState();
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
  // One gesture at a time. Background presses can become look-drags or, after a
  // hold, freehand lassos. Server and tray presses become direct drags after the
  // movement threshold; plain clicks still reach their buttons.
  type SpaceDragMode = "background-maybe" | "look" | "lasso" | "server-maybe" | "server" | "tray-maybe" | "tray";
  let spaceDrag: {
    id: number;
    sx: number;
    sy: number;
    cx0: number;
    cy0: number;
    yaw0: number;
    pitch0: number;
    mode: SpaceDragMode;
    serverId?: number;
  } | null = null;
  let spaceHoldTimer = 0;
  let spaceLasso = $state<{ points: ScreenPoint[] } | null>(null);
  // Captured servers ride as angular offsets around the aim point until dropped.
  let spaceCarried = $state<Record<number, Placement> | null>(null);
  let spaceSwallowClick = false; // a drop's trailing click must not open a server
  let spaceEntering = $state<number | null>(null);
  let spaceEntryPhase = $state<"focus" | "zoom" | null>(null);
  let spaceEnterTimer = 0;
  let spaceCameraRaf = 0;
  let spaceTrayPinned = $state(false);
  let spaceTrayHeld = $state(false);
  let spaceTray = $derived(spaceTrayPinned || spaceTrayHeld);
  let spaceSearch = $state("");
  let spaceSearchOpen = $state(false);
  let spaceSearchEl = $state<HTMLInputElement | undefined>();
  let spaceSearchIdx = $state(0);
  let spaceFocusedServer = $state<number | null>(null);
  let spaceOnlineCounts = $state<Record<number, number>>({});
  let spaceActivityAt = $state<Record<number, number>>({});
  let spaceUndo = $state<Record<number, Placement>[]>([]);
  let spaceRedo = $state<Record<number, Placement>[]>([]);
  let spaceNewCluster = $state("");
  let spaceNewClusterColor = $state("#8d7cf5");
  let spaceClusterOpen = $state<string | null>(null);
  let spaceClusterDrop = $state<string | null>(null);
  let spacePanoYaw = $state(0);
  let spaceSeamPreview = $state(false);
  let spaceLayoutBusy = $state(false);
  let spaceLayoutInput = $state<HTMLInputElement | undefined>();
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
  function cloneSpacePlacements(source = spaceState.placements): Record<number, Placement> {
    return Object.fromEntries(Object.entries(source).map(([id, p]) => [Number(id), { ...p }]));
  }
  function rememberSpaceLayout() {
    spaceUndo = [...spaceUndo.slice(-19), cloneSpacePlacements()];
    spaceRedo = [];
  }
  function undoSpaceLayout() {
    const previous = spaceUndo[spaceUndo.length - 1];
    if (!previous) return;
    spaceRedo = [...spaceRedo.slice(-19), cloneSpacePlacements()];
    spaceUndo = spaceUndo.slice(0, -1);
    spaceState.placements = cloneSpacePlacements(previous);
    saveSpace();
  }
  function redoSpaceLayout() {
    const next = spaceRedo[spaceRedo.length - 1];
    if (!next) return;
    spaceUndo = [...spaceUndo.slice(-19), cloneSpacePlacements()];
    spaceRedo = spaceRedo.slice(0, -1);
    spaceState.placements = cloneSpacePlacements(next);
    saveSpace();
  }
  async function refreshSpacePresence() {
    const pairs = await Promise.all(railServers.map(async (s) => {
      try {
        const online = await invoke<string[]>("get_online_members", { server: s.id });
        return [s.id, online.length + 1] as const; // this device is also present
      } catch {
        return [s.id, 1] as const;
      }
    }));
    spaceOnlineCounts = Object.fromEntries(pairs);
  }
  let spaceMentionCounts = $derived.by(() => {
    const counts: Record<number, number> = {};
    for (const item of inboxItems) if (inboxUnseen(item)) counts[item.server] = (counts[item.server] ?? 0) + 1;
    return counts;
  });
  function spaceVoiceCount(server: number): number {
    let count = 0;
    for (const s of servers.find((item) => item.id === server)?.channels ?? []) {
      count += roomMembers(server, s.id).length;
    }
    return count;
  }
  function stopSpaceCameraTween() {
    if (spaceCameraRaf) cancelAnimationFrame(spaceCameraRaf);
    spaceCameraRaf = 0;
  }
  function tweenSpaceCamera(target: Placement, duration: number, done: () => void = () => {}) {
    stopSpaceCameraTween();
    const from = { ...spaceCam };
    const dyaw = yawDelta(from.yaw, target.yaw);
    const dpitch = target.pitch - from.pitch;
    const start = performance.now();
    const frame = (now: number) => {
      const raw = Math.min(1, (now - start) / Math.max(1, duration));
      const t = 1 - (1 - raw) ** 3;
      spaceCam = { yaw: wrapYaw(from.yaw + dyaw * t), pitch: clampPitch(from.pitch + dpitch * t) };
      if (raw < 1) spaceCameraRaf = requestAnimationFrame(frame);
      else {
        spaceCameraRaf = 0;
        done();
      }
    };
    spaceCameraRaf = requestAnimationFrame(frame);
  }
  function focusSpaceServer(id: number, duration = 360) {
    const placement = spaceState.placements[id];
    if (!placement) return;
    spaceFocusedServer = id;
    tweenSpaceCamera(placement, fxMotionOff ? 1 : duration);
  }
  function focusSpaceCluster(id: string, duration = 420) {
    if (!spaceState.clusters.some((cluster) => cluster.id === id)) return;
    spaceClusterOpen = id;
    spaceFocusedServer = null;
    tweenSpaceCamera(spaceClusterAnchor(id), fxMotionOff ? 1 : duration);
  }
  function toggleSpace() {
    clearTimeout(spaceEnterTimer);
    stopSpaceCameraTween();
    spaceEntering = null;
    spaceEntryPhase = null;
    spaceOpen = !spaceOpen;
    spaceLasso = null;
    spaceCarried = null;
    spaceTrayPinned = false;
    spaceTrayHeld = false;
    spaceDrag = null;
    spaceSearch = "";
    spaceSearchOpen = false;
    spaceClusterOpen = null;
    spaceClusterDrop = null;
    if (spaceOpen) {
      // Migrate an older or hand-edited layout too, rather than only preventing
      // new drops from overlapping from this point onward.
      spaceState.placements = separatePlacements(
        spaceState.placements,
        Object.keys(spaceState.placements).map(Number),
        spaceMinSeparation(),
      );
      saveSpace();
      refreshSpaceAccents();
      void refreshSpacePresence();
      void loadInbox();
    }
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
  let spaceSearchMatches = $derived.by(() => {
    const q = spaceSearch.trim().toLowerCase();
    if (!q) return [] as ServerState[];
    return railServers.filter((s) => {
      const cluster = spaceState.clusters.find((c) => c.id === spaceState.serverClusters[s.id]);
      return s.name.toLowerCase().includes(q) || !!cluster?.name.toLowerCase().includes(q);
    });
  });
  function spaceClusterServerIds(cluster: string): number[] {
    return railServers.filter((s) => spaceState.serverClusters[s.id] === cluster).map((s) => s.id);
  }
  function spaceClusterAnchor(cluster: string): Placement {
    const index = Math.max(0, spaceState.clusters.findIndex((c) => c.id === cluster));
    const fallback = {
      yaw: wrapYaw((index * 360) / Math.max(1, spaceState.clusters.length)),
      pitch: 0,
    };
    return placementCentre(spaceState.placements, spaceClusterServerIds(cluster), fallback);
  }
  let spaceZones = $derived.by(() => {
    const zones: { cluster: SpaceCluster; x: number; y: number; rx: number; ry: number; count: number }[] = [];
    for (const cluster of spaceState.clusters) {
      const ids = spaceClusterServerIds(cluster.id);
      const centre = project(spaceCam, spaceClusterAnchor(cluster.id), spaceF);
      if (!centre.visible) continue;
      const members = spacePlaced.filter((it) => spaceState.serverClusters[it.s.id] === cluster.id);
      const spreadX = members.length ? Math.max(...members.map((m) => Math.abs(m.x - centre.x))) : 0;
      const spreadY = members.length ? Math.max(...members.map((m) => Math.abs(m.y - centre.y))) : 0;
      zones.push({
        cluster,
        x: centre.x,
        y: centre.y,
        rx: Math.min(230, Math.max(70, spreadX + spaceState.serverSize)),
        ry: Math.min(160, Math.max(50, spreadY + spaceState.serverSize * 0.8)),
        count: ids.length,
      });
    }
    return zones;
  });
  let spaceMapServers = $derived.by(() => railServers.flatMap((s) => {
    const p = spaceState.placements[s.id];
    if (!p) return [];
    return [{
      s,
      x: ((yawDelta(spaceCam.yaw, p.yaw) + 180) / 360) * 100,
      y: 50 - ((p.pitch - spaceCam.pitch) / 120) * 72,
    }];
  }));
  // A restrained constellation layer gives the floating icons some shared depth.
  // Each visible server links only to its nearest neighbour, with duplicates folded.
  let spaceLinks = $derived.by(() => {
    const links: { key: string; x1: number; y1: number; x2: number; y2: number }[] = [];
    const seen = new Set<string>();
    for (const a of spacePlaced) {
      let nearest: (typeof spacePlaced)[number] | null = null;
      let nearestD = 360;
      for (const b of spacePlaced) {
        if (a.s.id === b.s.id) continue;
        const d = Math.hypot(a.x - b.x, a.y - b.y);
        if (d < nearestD) {
          nearest = b;
          nearestD = d;
        }
      }
      if (!nearest) continue;
      const ids = [a.s.id, nearest.s.id].sort((x, y) => x - y);
      const key = `${ids[0]}:${ids[1]}`;
      if (seen.has(key)) continue;
      seen.add(key);
      links.push({ key, x1: a.x, y1: a.y, x2: nearest.x, y2: nearest.y });
    }
    return links;
  });
  // "custom" without an uploaded panorama falls back to the default room.
  let spaceBackdropEff = $derived(spaceState.backdrop === "custom" && !spaceState.custom ? "den" : spaceState.backdrop);
  function spaceCursorFrom(e: PointerEvent) {
    const r = spaceRoot?.getBoundingClientRect();
    if (!r) return;
    spaceCursor = { x: e.clientX - r.left - r.width / 2, y: e.clientY - r.top - r.height / 2 };
  }
  function spaceClusterAtPoint(e: PointerEvent): string | null {
    const target = document.elementFromPoint(e.clientX, e.clientY) as HTMLElement | null;
    const id = target?.closest<HTMLElement>("[data-space-cluster]")?.dataset.spaceCluster ?? "";
    return spaceState.clusters.some((cluster) => cluster.id === id) ? id : null;
  }
  function putSpaceCarriedInCluster(cluster: string) {
    if (!spaceCarried) return;
    const ids = Object.keys(spaceCarried).map(Number);
    for (const id of ids) spaceState.serverClusters[id] = cluster;
    const anchor = spaceClusterAnchor(cluster);
    const newlyPlaced = Object.fromEntries(
      ids.filter((id) => !spaceState.placements[id]).map((id) => [id, anchor]),
    );
    if (Object.keys(newlyPlaced).length) commitSpacePlacements(newlyPlaced);
    else saveSpace();
    spaceClusterOpen = cluster;
    spaceCarried = null;
    spaceSwallowClick = true;
  }
  function newSpaceDrag(e: PointerEvent, mode: SpaceDragMode, serverId: number | undefined = undefined) {
    spaceCursorFrom(e);
    spaceDrag = {
      id: e.pointerId,
      sx: e.clientX,
      sy: e.clientY,
      cx0: spaceCursor.x,
      cy0: spaceCursor.y,
      yaw0: spaceCam.yaw,
      pitch0: spaceCam.pitch,
      mode,
      serverId,
    };
  }
  function onSpaceDown(e: PointerEvent) {
    if (e.button !== 0 || spaceEntering !== null) return;
    const target = e.target as HTMLElement | null;
    if (!spaceCarried && target?.closest("button, input, label, .sp-tray")) return;
    newSpaceDrag(e, "background-maybe");
    clearTimeout(spaceHoldTimer);
    // A lasso can only begin on empty space. A server press is handled by
    // onSpaceServerDown and never reaches this timer.
    if (!spaceCarried) {
      spaceHoldTimer = window.setTimeout(() => {
        if (!spaceDrag || spaceDrag.mode !== "background-maybe" || !spaceOpen) return;
        spaceDrag.mode = "lasso";
        spaceRoot?.setPointerCapture(spaceDrag.id);
        spaceLasso = { points: [{ x: spaceCursor.x, y: spaceCursor.y }] };
      }, 320);
    }
  }
  function onSpaceServerDown(e: PointerEvent, id: number) {
    if (e.button !== 0 || spaceEntering !== null || spaceCarried) return;
    e.stopPropagation();
    clearTimeout(spaceHoldTimer);
    newSpaceDrag(e, "server-maybe", id);
  }
  function onSpaceTrayServerDown(e: PointerEvent, id: number) {
    if (e.button !== 0 || spaceEntering !== null || spaceCarried) return;
    e.stopPropagation();
    clearTimeout(spaceHoldTimer);
    newSpaceDrag(e, "tray-maybe", id);
  }
  function onSpaceMove(e: PointerEvent) {
    spaceCursorFrom(e);
    if (!spaceDrag || e.pointerId !== spaceDrag.id) return;
    if (spaceLasso && spaceDrag.mode === "lasso") {
      const last = spaceLasso.points[spaceLasso.points.length - 1];
      if (!last || Math.hypot(spaceCursor.x - last.x, spaceCursor.y - last.y) >= 4) {
        spaceLasso = { points: [...spaceLasso.points, { x: spaceCursor.x, y: spaceCursor.y }] };
      }
      return;
    }
    const dx = e.clientX - spaceDrag.sx;
    const dy = e.clientY - spaceDrag.sy;
    if (spaceDrag.mode === "server-maybe" || spaceDrag.mode === "tray-maybe") {
      if (Math.hypot(dx, dy) < 5) return;
      clearTimeout(spaceHoldTimer);
      spaceRoot?.setPointerCapture(spaceDrag.id);
      const id = spaceDrag.serverId;
      if (id === undefined) return;
      if (spaceDrag.mode === "server-maybe") {
        const grab = unproject(spaceCam, spaceDrag.cx0, spaceDrag.cy0, spaceF);
        spaceCarried = angularOffsets([id], spaceState.placements, grab);
        spaceDrag.mode = "server";
      } else {
        spaceCarried = { [id]: { yaw: 0, pitch: 0 } };
        spaceDrag.mode = "tray";
      }
      return;
    }
    if (spaceDrag.mode === "server" || spaceDrag.mode === "tray") {
      spaceClusterDrop = spaceClusterAtPoint(e);
      return;
    }
    if (spaceDrag.mode === "background-maybe") {
      if (Math.hypot(dx, dy) < 6) return; // still a click or a hold
      clearTimeout(spaceHoldTimer);
      spaceDrag.mode = "look";
      spaceRoot?.setPointerCapture(spaceDrag.id);
    }
    if (spaceDrag.mode !== "look") return;
    // Grab semantics: the world follows the hand, small-angle px-to-degrees via f.
    const k = (180 / Math.PI) / spaceF;
    spaceCam = { yaw: wrapYaw(spaceDrag.yaw0 - dx * k), pitch: clampPitch(spaceDrag.pitch0 + dy * k) };
  }
  function onSpaceUp(e: PointerEvent) {
    clearTimeout(spaceHoldTimer);
    if (!spaceDrag || e.pointerId !== spaceDrag.id) return;
    const mode = spaceDrag.mode;
    const clusterDrop = spaceCarried ? spaceClusterAtPoint(e) : null;
    spaceDrag = null;
    spaceClusterDrop = null;
    if (spaceLasso) {
      const path = [...spaceLasso.points, { x: spaceCursor.x, y: spaceCursor.y }];
      const caught = lassoCapturePath(spaceState.placements, spaceCam, path, spaceF);
      if (caught.length) {
        const aim = unproject(spaceCam, spaceCursor.x, spaceCursor.y, spaceF);
        spaceCarried = angularOffsets(caught, spaceState.placements, aim);
      }
      spaceLasso = null;
      spaceSwallowClick = true;
      return;
    }
    if (clusterDrop && spaceCarried) {
      putSpaceCarriedInCluster(clusterDrop);
      return;
    }
    if ((mode === "server" || mode === "tray") && spaceCarried) {
      const aim = unproject(spaceCam, spaceCursor.x, spaceCursor.y, spaceF);
      commitSpacePlacements(applyOffsets(spaceCarried, aim));
      spaceCarried = null;
      spaceSwallowClick = true;
      return;
    }
    if (mode === "background-maybe" && spaceCarried) {
      // A plain click while carrying: drop the constellation where the cursor aims.
      const aim = unproject(spaceCam, spaceCursor.x, spaceCursor.y, spaceF);
      commitSpacePlacements(applyOffsets(spaceCarried, aim));
      spaceCarried = null;
      spaceSwallowClick = true;
    }
  }
  function spaceLassoPath(points: ScreenPoint[]): string {
    if (!points.length) return "";
    return `M ${points.map((p) => `${p.x + spaceVw / 2} ${p.y + spaceVh / 2}`).join(" L ")} Z`;
  }
  // Drops and lasso releases produce a trailing click on whatever sat under the
  // pointer; capture-phase swallow keeps that click from opening a server.
  function onSpaceClickCapture(e: MouseEvent) {
    if (!spaceSwallowClick) return;
    spaceSwallowClick = false;
    e.stopPropagation();
    e.preventDefault();
  }
  function startSpaceSearch(seed = "") {
    spaceSearchOpen = true;
    spaceSearch = seed;
    spaceSearchIdx = 0;
    void tick().then(() => spaceSearchEl?.focus());
  }
  function pickSpaceSearch(open: boolean) {
    const matches = spaceSearchMatches;
    if (!matches.length) return;
    const server = matches[Math.max(0, Math.min(spaceSearchIdx, matches.length - 1))];
    if (!spaceState.placements[server.id]) placeFromTray(server.id);
    if (open) spaceIconClick(server.id);
    else focusSpaceServer(server.id);
  }
  function onSpaceSearchKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowRight") {
      e.preventDefault();
      spaceSearchIdx = Math.min(spaceSearchMatches.length - 1, spaceSearchIdx + 1);
      pickSpaceSearch(false);
    } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
      e.preventDefault();
      spaceSearchIdx = Math.max(0, spaceSearchIdx - 1);
      pickSpaceSearch(false);
    } else if (e.key === "Enter") {
      e.preventDefault();
      pickSpaceSearch(true);
    } else if (e.key === "Escape") {
      e.preventDefault();
      spaceSearch = "";
      spaceSearchOpen = false;
      spaceFocusedServer = null;
      spaceRoot?.focus();
    }
  }
  function cycleSpaceFocus(direction: number) {
    const ids = railServers.filter((s) => !!spaceState.placements[s.id]).map((s) => s.id);
    if (!ids.length) return;
    const at = spaceFocusedServer === null ? -1 : ids.indexOf(spaceFocusedServer);
    const next = ids[(at + direction + ids.length) % ids.length];
    focusSpaceServer(next);
  }
  function handleSpaceKey(e: KeyboardEvent): boolean {
    if (!spaceOpen || typingTarget(e.target) || e.ctrlKey || e.metaKey || e.altKey) return false;
    if (e.key === "/") {
      e.preventDefault();
      startSpaceSearch();
      return true;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      cycleSpaceFocus(e.shiftKey ? -1 : 1);
      return true;
    }
    if (e.key === "Enter" && spaceFocusedServer !== null) {
      e.preventDefault();
      spaceIconClick(spaceFocusedServer);
      return true;
    }
    if (e.key.startsWith("Arrow")) {
      e.preventDefault();
      if (e.key === "ArrowLeft") spaceCam = { ...spaceCam, yaw: wrapYaw(spaceCam.yaw - 6) };
      if (e.key === "ArrowRight") spaceCam = { ...spaceCam, yaw: wrapYaw(spaceCam.yaw + 6) };
      if (e.key === "ArrowUp") spaceCam = { ...spaceCam, pitch: clampPitch(spaceCam.pitch + 5) };
      if (e.key === "ArrowDown") spaceCam = { ...spaceCam, pitch: clampPitch(spaceCam.pitch - 5) };
      spaceFocusedServer = null;
      return true;
    }
    if (e.key.length === 1 && e.key !== " " && e.key.toLowerCase() !== "t") {
      e.preventDefault();
      startSpaceSearch(e.key);
      return true;
    }
    return false;
  }
  function spaceIconClick(id: number) {
    if (spaceCarried || spaceSwallowClick || spaceEntering !== null) return;
    playSpacePortal();
    if (!spaceState.zoomOnOpen || fxMotionOff) {
      void switchServer(id); // switchServer also folds the space away
      return;
    }
    spaceEntering = id;
    spaceEntryPhase = "focus";
    clearTimeout(spaceEnterTimer);
    const target = spaceState.placements[id];
    if (!target) {
      spaceEntering = null;
      spaceEntryPhase = null;
      void switchServer(id);
      return;
    }
    tweenSpaceCamera(target, 360, () => {
      if (spaceEntering !== id) return;
      spaceEntryPhase = "zoom";
      spaceEnterTimer = window.setTimeout(() => {
        if (spaceEntering !== id) return;
        spaceEntering = null;
        spaceEntryPhase = null;
        void switchServer(id);
      }, 440);
    });
  }
  function spaceServerMenu(s: ServerState): MenuItem[] {
    const clusterItems: MenuItem[] = spaceState.clusters.map((cluster) => ({
      label: `${spaceState.serverClusters[s.id] === cluster.id ? "✓ " : ""}Neighbourhood: ${cluster.name}`,
      onSelect: () => assignSpaceCluster(s.id, cluster.id),
    }));
    return [
      { label: "Open", onSelect: () => spaceIconClick(s.id) },
      { label: "Focus", onSelect: () => focusSpaceServer(s.id) },
      ...(clusterItems.length ? [{ divider: true } as MenuItem, ...clusterItems, {
        label: `${spaceState.serverClusters[s.id] ? "✓ " : ""}Neighbourhood: Unsorted`,
        onSelect: () => assignSpaceCluster(s.id, ""),
      } as MenuItem] : []),
      { divider: true },
      {
        label: "Return to tray",
        onSelect: () => {
          rememberSpaceLayout();
          const { [s.id]: _gone, ...rest } = spaceState.placements;
          spaceState.placements = rest;
          saveSpace();
        },
      },
    ];
  }
  // Tray tap: the server flies to wherever the camera is aiming (the reticle).
  function placeFromTray(id: number) {
    commitSpacePlacements({ [id]: { yaw: spaceCam.yaw, pitch: spaceCam.pitch } });
  }
  function spaceMinSeparation(size = spaceState.serverSize): number {
    // Use the smallest possible focal length so a saved layout remains clear in
    // a narrower window too; the small extra halo leaves the pulse room to breathe.
    return (2 * Math.atan((size * 0.58) / 560) * 180) / Math.PI;
  }
  function commitSpacePlacements(moved: Record<number, Placement>) {
    rememberSpaceLayout();
    const merged = { ...spaceState.placements, ...moved };
    spaceState.placements = separatePlacements(merged, Object.keys(moved).map(Number), spaceMinSeparation());
    saveSpace();
  }
  function setSpaceServerSize(size: number) {
    spaceState.serverSize = Math.max(32, Math.min(88, Math.round(size)));
    spaceState.placements = separatePlacements(
      spaceState.placements,
      Object.keys(spaceState.placements).map(Number),
      spaceMinSeparation(spaceState.serverSize),
    );
    saveSpace();
  }
  function setSpaceShape(shape: "circle" | "square") {
    spaceState.shape = shape;
    saveSpace();
  }
  function setSpaceBackdrop(b: string) {
    spaceState.backdrop = b as SpaceState["backdrop"];
    saveSpace();
  }
  function assignSpaceCluster(server: number, cluster: string) {
    if (cluster && !spaceState.clusters.some((item) => item.id === cluster)) return;
    if (cluster) spaceState.serverClusters[server] = cluster;
    else delete spaceState.serverClusters[server];
    saveSpace();
  }
  function toggleSpaceClusterServer(server: number, cluster: string) {
    const joining = spaceState.serverClusters[server] !== cluster;
    assignSpaceCluster(server, joining ? cluster : "");
    if (joining && !spaceState.placements[server]) {
      commitSpacePlacements({ [server]: spaceClusterAnchor(cluster) });
    }
  }
  function addSpaceCluster() {
    const name = spaceNewCluster.trim().slice(0, 32);
    if (!name) return;
    const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "neighbourhood";
    let id = base.slice(0, 24), suffix = 2;
    while (spaceState.clusters.some((c) => c.id === id)) id = `${base.slice(0, 20)}-${suffix++}`;
    spaceState.clusters = [...spaceState.clusters, { id, name, color: spaceNewClusterColor }];
    spaceNewCluster = "";
    saveSpace();
    spaceClusterOpen = id;
    if (spaceOpen) focusSpaceCluster(id);
  }
  function updateSpaceCluster(id: string, update: Partial<Pick<SpaceCluster, "name" | "color">>) {
    spaceState.clusters = spaceState.clusters.map((c) => c.id === id ? { ...c, ...update } : c);
    saveSpace();
  }
  function removeSpaceCluster(id: string) {
    spaceState.clusters = spaceState.clusters.filter((c) => c.id !== id);
    spaceState.serverClusters = Object.fromEntries(
      Object.entries(spaceState.serverClusters).filter(([, cluster]) => cluster !== id).map(([server, cluster]) => [Number(server), cluster]),
    );
    if (spaceClusterOpen === id) spaceClusterOpen = null;
    if (spaceClusterDrop === id) spaceClusterDrop = null;
    saveSpace();
  }
  function tidySpace() {
    const ids = railServers.map((s) => s.id);
    if (!ids.length) return;
    rememberSpaceLayout();
    spaceState.placements = autoArrangePlacements(ids, spaceState.serverClusters, spaceMinSeparation());
    saveSpace();
  }
  function forgetSpacePlacements() {
    if (!Object.keys(spaceState.placements).length) return;
    rememberSpaceLayout();
    spaceState.placements = {};
    saveSpace();
  }
  let spaceImageNote = $state("");
  // A custom panorama is normalized to an exact 2:1 equirectangular canvas before
  // storage. Centre-cropping odd aspect ratios avoids the stretching that made a
  // portrait or phone photo look especially rough on the cube walls.
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
      let sx = 0, sy = 0, sw = img.naturalWidth, sh = img.naturalHeight;
      if (sw / sh > 2) {
        sw = sh * 2;
        sx = (img.naturalWidth - sw) / 2;
      } else if (sw / sh < 2) {
        sh = sw / 2;
        sy = (img.naturalHeight - sh) / 2;
      }
      const w = Math.max(2, Math.min(2048, Math.floor(sw)));
      const h = Math.max(1, Math.floor(w / 2));
      const c = document.createElement("canvas");
      c.width = w;
      c.height = h;
      c.getContext("2d")?.drawImage(img, sx, sy, sw, sh, 0, 0, w, h);
      URL.revokeObjectURL(url);
      spaceState.custom = c.toDataURL("image/jpeg", 0.82);
      spaceState.backdrop = "custom";
      spaceImageNote = `${file.name} prepared as ${w} × ${h}${Math.abs(img.naturalWidth / img.naturalHeight - 2) > 0.01 ? " (centre-cropped to 2:1)" : ""}.`;
      saveSpace();
    } catch (err) {
      error = String(err);
    }
  }
  // A paint-over PNG for image editors. The cardinal centres, cube seams,
  // horizon, visible band, and circular safe areas mirror this renderer.
  let spaceGuideSaving = $state(false);
  async function downloadSpaceTemplate() {
    if (spaceGuideSaving) return;
    const c = document.createElement("canvas");
    c.width = 2048;
    c.height = 1024;
    const x = c.getContext("2d");
    if (!x) return;
    x.fillStyle = "#11141d";
    x.fillRect(0, 0, c.width, c.height);
    x.fillStyle = "rgba(118, 139, 255, 0.08)";
    x.fillRect(0, 256, c.width, 512);
    x.strokeStyle = "rgba(255, 255, 255, 0.28)";
    x.lineWidth = 2;
    x.setLineDash([12, 10]);
    for (const seam of [256, 768, 1280, 1792]) {
      x.beginPath(); x.moveTo(seam, 0); x.lineTo(seam, 1024); x.stroke();
    }
    x.strokeStyle = "rgba(118, 139, 255, 0.72)";
    x.setLineDash([]);
    x.beginPath(); x.moveTo(0, 512); x.lineTo(2048, 512); x.stroke();
    x.strokeStyle = "rgba(118, 139, 255, 0.42)";
    x.setLineDash([8, 8]);
    for (const center of [0, 512, 1024, 1536, 2048]) {
      x.beginPath(); x.arc(center, 512, 230, 0, Math.PI * 2); x.stroke();
    }
    x.font = "600 28px ui-monospace, monospace";
    x.textAlign = "center";
    x.fillStyle = "rgba(255, 255, 255, 0.82)";
    [[0, "BACK / SEAM"], [512, "LEFT"], [1024, "FRONT"], [1536, "RIGHT"], [2048, "BACK / SEAM"]].forEach(([cx, label]) => {
      x.fillText(String(label), Number(cx), 500);
    });
    x.font = "22px ui-monospace, monospace";
    x.fillStyle = "rgba(255, 255, 255, 0.56)";
    x.fillText("HORIZON — KEEP IMPORTANT DETAIL IN THE MIDDLE 50%", 1024, 548);
    x.fillText("TOP / CEILING POLE — LOW DETAIL", 1024, 54);
    x.fillText("BOTTOM / FLOOR POLE — LOW DETAIL", 1024, 988);
    x.font = "18px ui-monospace, monospace";
    x.fillStyle = "rgba(255, 255, 255, 0.4)";
    x.fillText("2048 × 1024 EQUIRECTANGULAR · HIDE THIS GUIDE LAYER BEFORE EXPORT", 1024, 92);
    const dataUrl = c.toDataURL("image/png");
    const comma = dataUrl.indexOf(",");
    if (comma < 0) {
      spaceImageNote = "The guide could not be generated.";
      toast(spaceImageNote, "err", 6000);
      return;
    }
    spaceGuideSaving = true;
    spaceImageNote = "Saving the guide to Downloads…";
    try {
      const saved = await saveSpaceGuide(invoke, dataUrl.slice(comma + 1));
      const notice = guideSavedNotice(saved);
      spaceImageNote = notice.note;
      toast(notice.text, notice.kind, 5000);
    } catch (err) {
      spaceImageNote = `Guide failed to open: ${String(err)}`;
      toast(spaceImageNote, "err", 8000);
    } finally {
      spaceGuideSaving = false;
    }
  }
  async function exportSpaceLayout() {
    spaceLayoutBusy = true;
    try {
      const json = JSON.stringify({ kind: "mewtual-server-space-layout", version: 1, space: spaceState }, null, 2);
      const saved = await invoke<{ path: string; displayed: boolean; warning?: string }>("save_space_layout", { json });
      toast(saved.displayed ? "Space layout saved to Downloads" : `Layout saved to ${saved.path}`, saved.displayed ? "ok" : "info", 6000);
    } catch (err) {
      toast(`Could not export the Space layout: ${String(err)}`, "err", 8000);
    } finally {
      spaceLayoutBusy = false;
    }
  }
  async function importSpaceLayout(files: FileList | null) {
    const file = files?.[0];
    if (!file) return;
    try {
      if (file.size > 10 * 1024 * 1024) throw new Error("layout file is too large");
      const raw = JSON.parse(await file.text());
      if (raw?.kind !== "mewtual-server-space-layout" || raw?.version !== 1 || !raw.space) {
        throw new Error("not a Mewtual Server Space layout");
      }
      rememberSpaceLayout();
      spaceState = parseSpace(JSON.stringify(raw.space));
      saveSpace();
      toast("Space layout imported", "ok", 5000);
    } catch (err) {
      toast(`Could not import the Space layout: ${String(err)}`, "err", 8000);
    } finally {
      if (spaceLayoutInput) spaceLayoutInput.value = "";
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
    { id: "garden", name: "Lumen Garden" },
  ];
  // Controller support stays entirely local: left stick looks, bumpers cycle,
  // A opens the focused server, X undoes a move, and B leaves Space.
  $effect(() => {
    if (!spaceOpen || typeof navigator === "undefined" || !("getGamepads" in navigator)) return;
    let raf = 0;
    let last = performance.now();
    const previous: boolean[] = [];
    const frame = (now: number) => {
      const pad = navigator.getGamepads().find((g) => !!g);
      if (pad) {
        const dt = Math.min(40, now - last) / 16.67;
        const ax = Math.abs(pad.axes[0] ?? 0) > 0.18 ? pad.axes[0] : 0;
        const ay = Math.abs(pad.axes[1] ?? 0) > 0.18 ? pad.axes[1] : 0;
        if (ax || ay) {
          spaceCam = {
            yaw: wrapYaw(spaceCam.yaw + ax * 1.2 * dt),
            pitch: clampPitch(spaceCam.pitch - ay * 1.05 * dt),
          };
          spaceFocusedServer = null;
        }
        const pressed = (index: number) => !!pad.buttons[index]?.pressed && !previous[index];
        if (pressed(4)) cycleSpaceFocus(-1);
        if (pressed(5)) cycleSpaceFocus(1);
        if (pressed(0) && spaceFocusedServer !== null) spaceIconClick(spaceFocusedServer);
        if (pressed(2)) undoSpaceLayout();
        if (pressed(1)) toggleSpace();
        for (let i = 0; i < pad.buttons.length; i += 1) previous[i] = pad.buttons[i].pressed;
      }
      last = now;
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  });
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
  let jukeVol = $state(loadJukeVol());
  // Dock fold, shared by both call surfaces because the deck is one deck. What stays is the
  // room's shared state (track, time, transport, DJ, sync chip); what folds is the queue, which
  // is planning rather than state. Planning is the part you put away.
  let jukeOpen = $state(loadCallSetting("jukeopen", "on") !== "off");
  function toggleJukeOpen() {
    jukeOpen = !jukeOpen;
    try {
      localStorage.setItem("catcoms.call.jukeopen", jukeOpen ? "on" : "off");
    } catch {
      /* storage unavailable */
    }
  }
  const jukeFailed = new Set<string>(); // cids nobody would serve: the DJ's auto-advance skips them
  // The transport we currently follow. `seq`/`fromFp` decide who wins a race, `off`/`at` anchor the
  // position to the local clock. Plain `let`: identity, not reactivity, is what the races need.
  let jukeSeq = 0; // my own monotonic press counter
  let jukeAdopted: { seq: number; fromFp: string; off: number; at: number } | null = null;
  let jukeHeard = 0; // performance.now() of the last frame from the DJ we follow
  let jukeAudio: HTMLVideoElement | null = null;
  // How the current track is coming in: null once it is playing, otherwise local read or peer
  // pull with a percentage. Fed by the same download-progress events the Downloads surface uses.
  let jukeFetch = $state<FetchPhase | null>(null);
  let jukeBuffering = $state(false); // the element ran out of data mid-track
  let jukeNudging = $state(false); // easing back onto the DJ's clock rather than snapping
  // Audio or video, from the current track's name (a queue entry carries no mime) and the share's
  // declared type when the share is the one in view.
  let jukeKind = $derived<MediaKind>(
    jukeNow ? mediaKind(jukeNow.name, files.find((f) => f.cid === jukeNow?.cid)?.mime ?? "") : "other",
  );
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
  // One <video> element for both kinds. A video element plays audio perfectly well, and building
  // one element means the transport, drift and race logic below never has to care which kind is
  // loaded; only the surfaces care, and they adopt this element by re-parenting it rather than
  // creating their own, so folding the dock or moving to the focus view never restarts playback.
  function jukeEl(): HTMLVideoElement {
    if (jukeAudio) return jukeAudio;
    const el = document.createElement("video");
    el.id = "jukebox-media";
    el.volume = jukeVol;
    el.muted = callDeafened;
    el.playsInline = true;
    el.preload = "auto";
    el.addEventListener("loadedmetadata", () => {
      jukeDur = Number.isFinite(el.duration) ? el.duration : 0;
      jukeSettle(); // the seek the src swap could not take yet
    });
    el.addEventListener("ended", () => jukeEnded());
    // Streaming moves the failure from a thrown fetch to the element: a track nobody can serve,
    // or one the webview cannot decode, both land here.
    el.addEventListener("error", () => {
      if (jukeNow?.cid) jukeFail(jukeNow.cid);
    });
    // Ran dry mid-track. Distinct from the initial pull, and worth showing, because on a slow
    // peer it is the difference between "this is loading" and "this has stalled".
    el.addEventListener("waiting", () => (jukeBuffering = true));
    el.addEventListener("playing", () => {
      jukeBuffering = false;
      jukeFetch = null;
    });
    el.addEventListener("canplay", () => (jukeBuffering = false));
    document.body.appendChild(el);
    jukeAudio = el;
    return el;
  }
  // I am the DJ while the transport we follow is my own press.
  function jukeIsDj(): boolean {
    return !!jukeAdopted && !!callSelfFp && jukeAdopted.fromFp === callSelfFp;
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
    jukeAdopt(jukeSeq, callSelfFp, entry, cid, name, off, paused);
    broadcast({ callId: callChannel, type: "juke", seq: jukeSeq, entry, cid, name, off, paused });
  }
  function jukeAdopt(seq: number, fromFp: string, entry: string, cid: string, name: string, off: number, paused: boolean) {
    const same = jukeNow?.entry === entry && jukeNow?.cid === cid;
    jukeAdopted = { seq, fromFp, off, at: performance.now() };
    jukeHeard = jukeAdopted.at;
    jukeStale = false;
    jukeNow = entry || cid ? { entry, cid, name, paused, dj: fromFp === callSelfFp ? "" : fromFp } : null;
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
    const server = callServer;
    if (server === null) return;
    // The element streams straight out of the vault, so there is no fetch-then-play step any
    // more: playback starts on the first chunk instead of the last, and a seek costs one chunk.
    // A track nobody can serve now surfaces as an element error rather than a thrown fetch,
    // which is what jukeFail below is for.
    const url = mediaUrl(server, cid);
    const el = jukeEl();
    if (!sameTrack || el.src !== url) {
      if (el.src !== url) {
        el.src = url;
        jukeDur = 0;
        jukeFetch = null;
        jukeNudging = false;
        el.playbackRate = 1;
      }
      jukeSettle();
      return;
    }
    // Same track, so this is a ping or a play/pause. Video cannot hide a snap the way audio can,
    // so a small gap is eased out by playing slightly fast or slow and only a large one is
    // snapped; audio keeps snap-or-nothing, because a rate change is audible where a seek is not.
    const target = jukePos();
    if (el.readyState > 0) {
      const drift = target - el.currentTime;
      const action = driftAction(drift, jukeKind);
      if (action === "seek") {
        try { el.currentTime = target; } catch { /* not seekable yet */ }
        jukeNudging = false;
        el.playbackRate = 1;
      } else if (action === "nudge") {
        el.playbackRate = nudgeRate(drift);
        jukeNudging = true;
      } else if (jukeNudging) {
        el.playbackRate = 1;
        jukeNudging = false;
      }
    }
    const live = jukeNow;
    if (!live || live.paused) el.pause();
    else void el.play().catch(() => { /* still loading, or the webview wants a gesture first */ });
  }
  /**
   * Hand the one deck element to whichever surface currently wants to show it. Re-parenting keeps
   * playback running (a media element survives being moved in the DOM), which is the whole reason
   * there is one element rather than one per surface: folding the dock or opening focus must never
   * restart the room's film. On teardown it goes back to the body, still playing, still audible.
   */
  function jukeHost(node: HTMLElement) {
    const el = jukeEl();
    node.appendChild(el);
    return {
      destroy() {
        if (jukeAudio === el) document.body.appendChild(el);
      },
    };
  }
  /** The deck could not play what the DJ named: drop it, and move the room on if the deck is mine. */
  function jukeFail(cid: string) {
    jukeFailed.add(cid);
    jukeFetch = null;
    if (jukeNow?.cid === cid && jukeIsDj()) jukeSkip();
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
  // Leaving the room takes the deck with it. There are no blobs to release any more: the element
  // streamed from the vault rather than holding a decrypted copy of the track.
  function jukeReset() {
    jukeStop();
    jukeAudio?.remove();
    jukeAudio = null;
    jukeFailed.clear();
    jukeQueue = [];
    jukeNow = null;
    jukeAdopted = null;
    jukeStale = false;
    jukeFetch = null;
    jukeBuffering = false;
    jukeNudging = false;
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
  // Anything the deck can play: audio and video both, since one element handles both.
  let jukeAudioFiles = $derived(files.filter((f) => mediaKind(f.name, f.mime) !== "other"));
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
    // Never while the file index is still being read: an unread index is not proof of withdrawal.
    return jukeShareInView && !groupLoading && !files.some((f) => f.cid === cid);
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
  function toggleVideo(kind: "cam" | "screen") {
    if (myVideo === kind) stopVideo();
    else void startVideo(kind);
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
  let focusTiles = $derived([callSelfFp, ...callParticipants]);
  let focusCols = $derived(focusTiles.length <= 1 ? 1 : focusTiles.length <= 4 ? 2 : 3);

  async function negotiatePeer(peer: CallPeer) {
    if (callPeers[peer.fp] !== peer || peer.pc.signalingState === "closed") return;
    try {
      peer.makingOffer = true;
      await peer.pc.setLocalDescription();
      if (peer.pc.localDescription?.type === "offer") {
        void sendSignal(peer.server, peer.fp, {
          callId: peer.channel,
          type: "offer",
          sdp: peer.pc.localDescription,
        });
      }
    } catch (e) {
      console.warn("voice negotiation failed", { peer: peer.fp, error: String(e) });
    } finally {
      peer.makingOffer = false;
    }
  }
  async function flushWaitingIce(peer: CallPeer) {
    if (!peer.pc.remoteDescription) return;
    const queued = waitingIce[peer.fp] ?? [];
    delete waitingIce[peer.fp];
    for (const candidate of queued) {
      try {
        await peer.pc.addIceCandidate(new RTCIceCandidate(candidate));
      } catch (e) {
        if (!peer.ignoreOffer) console.warn("buffered ICE candidate was rejected", { peer: peer.fp, error: String(e) });
      }
    }
  }
  function recoverPeer(peer: CallPeer) {
    const now = Date.now();
    if (now - peer.lastRetry < 4000) return;
    const action = heartbeatRecovery({
      currentRoom: isCurrentVoiceRoom(callServer, callChannel, peer.server, peer.channel),
      hasPeer: true,
      connectionState: peer.pc.connectionState,
      signalingState: peer.pc.signalingState,
      localDescriptionType: peer.pc.localDescription?.type,
    });
    if (!action || action === "create") return;
    peer.lastRetry = now;
    if (action === "resend-offer" || action === "resend-answer") {
      const sdp = peer.pc.localDescription;
      if (sdp) void sendSignal(peer.server, peer.fp, { callId: peer.channel, type: sdp.type, sdp });
      return;
    }
    try {
      peer.pc.restartIce(); // raises negotiationneeded and sends a fresh ICE offer
    } catch (e) {
      console.warn("voice ICE restart failed", { peer: peer.fp, error: String(e) });
    }
  }

  // Ask the ICE agent which candidate pair actually won, and classify the media path from it.
  // A "relay" candidate on either end means a TURN carries the media: still ciphertext, but a
  // third party now sees who is talking to whom, when, and from which address. That is a real
  // difference in what leaks and the room deserves to be told, so it is surfaced rather than
  // averaged away. Best-effort throughout: on any miss the badge stays unknown, which is an
  // honest answer, where guessing "direct" would not be.
  async function sniffTransport(fp: string, pc: RTCPeerConnection) {
    try {
      const stats = await pc.getStats();
      let pairId = "";
      stats.forEach((s) => {
        const t = s as RTCStats & { selectedCandidatePairId?: string };
        if (t.type === "transport" && t.selectedCandidatePairId) pairId = t.selectedCandidatePairId;
      });
      if (!pairId) {
        // Firefox and older Chromium never fill selectedCandidatePairId; the nominated,
        // succeeded pair is the same fact spelled the other way.
        stats.forEach((s) => {
          const p = s as RTCStats & { state?: string; nominated?: boolean };
          if (p.type === "candidate-pair" && p.state === "succeeded" && p.nominated) pairId = p.id;
        });
      }
      const pair = pairId
        ? (stats.get(pairId) as (RTCStats & { localCandidateId?: string; remoteCandidateId?: string }) | undefined)
        : undefined;
      if (!pair) return;
      const local = pair.localCandidateId
        ? (stats.get(pair.localCandidateId) as (RTCStats & { candidateType?: string }) | undefined)
        : undefined;
      const remote = pair.remoteCandidateId
        ? (stats.get(pair.remoteCandidateId) as (RTCStats & { candidateType?: string }) | undefined)
        : undefined;
      if (!local && !remote) return;
      const relayed = local?.candidateType === "relay" || remote?.candidateType === "relay";
      peerTransport = { ...peerTransport, [fp]: relayed ? "relayed" : "direct" };
    } catch {
      /* stats are best-effort: an unknown path renders as no badge at all */
    }
  }

  function createPeer(fp: string): CallPeer | null {
    if (callPeers[fp]) return callPeers[fp];
    const server = callServer;
    const channel = callChannel;
    if (server === null || !channel || !callSelfFp) return null;
    const pc = new RTCPeerConnection({ iceServers: iceServers() });
    const peer: CallPeer = {
      fp,
      server,
      channel,
      pc,
      dc: null,
      polite: callSelfFp < fp,
      makingOffer: false,
      ignoreOffer: false,
      lastRetry: 0,
    };
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
    pc.onnegotiationneeded = () => void negotiatePeer(peer);
    pc.onicecandidate = (e) => {
      if (e.candidate) {
        void sendSignal(peer.server, fp, { callId: peer.channel, type: "ice", candidate: e.candidate.toJSON() });
      }
    };
    pc.onicecandidateerror = (e) => console.warn("voice ICE server/candidate error", {
      peer: fp,
      code: e.errorCode,
      text: e.errorText,
      url: e.url,
    });
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
      if (pc.connectionState === "connected") void sniffTransport(fp, pc);
      if (pc.connectionState === "failed") recoverPeer(peer);
      else if (pc.connectionState === "closed") removePeer(fp);
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
    delete waitingIce[fp];
    document.getElementById(`call-audio-${fp}`)?.remove();
    callParticipants = Object.keys(callPeers);
    const { [fp]: _drop, ...rest } = callPeerStates;
    callPeerStates = rest;
    const { [fp]: _path, ...paths } = peerTransport;
    peerTransport = paths;
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
    // Lift MIDI notes while the drawer is still their target, otherwise closing it mid-hold
    // strands a tone that nothing is left to release.
    if (instOpen) releaseMidiNotes();
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
    playMention(server);
  }
  // Join (or switch to) a channel's voice room. The channel id IS the call id.
  async function joinVoice(channel: string, server: number, name: string) {
    if (inCall && callChannel === channel && callServer === server) return;
    if (inCall) leaveVoice();
    let selfFp = "";
    try {
      const membersHere = await invoke<Member[]>("get_members", { server });
      selfFp = membersHere.find((m) => m.you)?.fingerprint ?? "";
    } catch (e) {
      error = `Couldn't read the voice room's member list: ${String(e)}`;
      return;
    }
    if (!selfFp) {
      error = "Couldn't identify this device on the voice room's server.";
      return;
    }
    callServer = server;
    callSelfFp = selfFp;
    // A missing or refused microphone is no longer a reason not to join. The room is also where
    // the jukebox and the instruments live, and neither needs one: the data channel carries the
    // instruments and the deck rides the mesh, so a peer with no mic is a full participant in
    // everything except talking. The dock offers the mic in place if one turns up later.
    micOn = (await ensureMic(false)) !== null;
    callChannel = channel;
    callChannelName = name;
    // Snapshot the room's server identity now, while we are certainly on it. Everything the
    // dock renders afterwards has to survive the user walking off to another server.
    callServerName = servers.find((s) => s.id === server)?.name ?? "";
    void refreshCallProfiles();
    inCall = true;
    callMuted = false;
    focusOpen = false;
    focusDismissed = false; // a new call earns a fresh chance to take the window
    voiceAlert = null;
    if (localStream) addAnalyser("me", localStream);
    startMeters();
    navigator.mediaDevices?.addEventListener?.("devicechange", onDeviceChange);
    alertedRooms.delete(roomKey(server, channel));
    recordPresence(server, channel, callSelfFp);
    void refreshJukebox(); // the room's queue, whatever the DJ is currently on
    broadcast({ callId: channel, type: "hello", mic: 0, inst: instRxMuted ? 1 : 0 }); // announce + trigger existing members to offer
    clearInterval(pingTimer);
    pingTimer = setInterval(() => {
      if (callChannel && callServer !== null) {
        broadcast({ callId: callChannel, type: "voice-ping", mic: callMuted ? 1 : 0, inst: instRxMuted ? 1 : 0 });
        recordPresence(callServer, callChannel, callSelfFp); // keep my own presence fresh
        jukeTick(); // the DJ's re-announce (and the listener's DJ-left check) ride this tick
        // Re-read the winning candidate pair: an ICE restart can migrate a live call from
        // direct to relayed (or back) with no connection-state change to notice it by.
        for (const [fp, p] of Object.entries(callPeers)) {
          if (p.pc.connectionState === "connected") void sniffTransport(fp, p.pc);
        }
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
    if (callServer !== null && callChannel && callSelfFp) dropPresence(callServer, callChannel, callSelfFp);
    inCall = false;
    callMuted = false;
    micOn = false;
    callChannel = "";
    callChannelName = "";
    callServer = null;
    callServerName = "";
    callSelfFp = "";
    callProfiles = {};
    peerTransport = {};
    secInfoOpen = false;
    for (const fp of Object.keys(waitingIce)) delete waitingIce[fp];
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
    const currentRoom = inCall && isCurrentVoiceRoom(callServer, callChannel, server, cid);
    // Presence: both "hello" (a newcomer) and "voice-ping" (heartbeat) mean someone's in a room.
    if (type === "hello" || type === "voice-ping") {
      const wasActive = roomMembers(server, cid).length > 0;
      recordPresence(server, cid, fromFp);
      maybeNotifyRoom(server, cid, wasActive);
      // Broadcast mute states ride the presence pings (the data channel also carries them, but
      // pings cover the window before it opens). Only my own room's states matter to the UI.
      if (currentRoom && typeof msg.mic === "number") {
        peerMeta = { ...peerMeta, [fromFp]: { mic: msg.mic === 1, inst: msg.inst === 1, vid: typeof msg.vid === "number" ? msg.vid : 0 } };
      }
      if (type === "voice-ping") {
        const peer = callPeers[fromFp];
        const action = heartbeatRecovery({
          currentRoom,
          hasPeer: !!peer,
          connectionState: peer?.pc.connectionState,
          signalingState: peer?.pc.signalingState,
          localDescriptionType: peer?.pc.localDescription?.type,
        });
        if (action === "create") createPeer(fromFp);
        else if (action && peer) recoverPeer(peer);
        return;
      }
    }
    // A bye is useful presence cleanup even for a room other than the one I am in.
    if (type === "bye") {
      dropPresence(server, cid, fromFp);
      if (currentRoom) removePeer(fromFp);
      return;
    }
    // Everything below is only for MY current room.
    if (!currentRoom) return;
    if (type === "juke") {
      jukeRecv(fromFp, msg); // shared-listening transport: what is playing and where it is
      return;
    }
    if (type === "hello") {
      if (callPeers[fromFp]) { recoverPeer(callPeers[fromFp]); return; }
      playBlip(79); // audible arrival: there is no lobby, so the room itself says someone joined
      createPeer(fromFp); // its tracks + data channel raise onnegotiationneeded, which sends the offer
    } else if (type === "offer") {
      const peer = callPeers[fromFp] ?? createPeer(fromFp);
      if (!peer) return;
      const pc = peer.pc;
      // Perfect negotiation, answer side: on a collision the impolite end ignores the incoming
      // offer (its own is in flight and will win); the polite end lets setRemoteDescription
      // implicitly roll its own offer back.
      const collision = peer.makingOffer || pc.signalingState !== "stable";
      peer.ignoreOffer = !peer.polite && collision;
      if (peer.ignoreOffer) return;
      try {
        await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
        await flushWaitingIce(peer);
        await pc.setLocalDescription(); // no-arg picks "answer" from the have-remote-offer state
        void sendSignal(server, fromFp, { callId: cid, type: "answer", sdp: pc.localDescription });
      } catch (e) {
        console.warn("voice offer handling failed", { peer: fromFp, error: String(e) });
      }
    } else if (type === "answer") {
      const peer = callPeers[fromFp];
      const pc = peer?.pc;
      // Guard against a stale answer landing after a rollback settled the state.
      if (peer && pc && pc.signalingState === "have-local-offer") {
        try {
          await pc.setRemoteDescription(new RTCSessionDescription(msg.sdp as RTCSessionDescriptionInit));
          await flushWaitingIce(peer);
        } catch (e) {
          console.warn("voice answer handling failed", { peer: fromFp, error: String(e) });
        }
      }
    } else if (type === "ice") {
      const peer = callPeers[fromFp];
      const candidate = msg.candidate as RTCIceCandidateInit | undefined;
      if (!candidate) return;
      if (!peer || !peer.pc.remoteDescription) {
        waitingIce[fromFp] = bufferIce(waitingIce[fromFp] ?? [], candidate);
      } else {
        try {
          await peer.pc.addIceCandidate(new RTCIceCandidate(candidate));
        } catch (e) {
          if (!peer.ignoreOffer) console.warn("voice ICE candidate was rejected", { peer: fromFp, error: String(e) });
        }
      }
    }
  }

  // Per-channel composer drafts: switching channels/servers preserves what you typed, and the
  // bounded map is vault-sealed through scheduleUiStateSave for restart durability.
  let drafts = $state<Record<string, string>>({});
  function saveDraftFor(key: string | null) {
    if (!key) return;
    if (draft.trim()) drafts[key] = draft;
    else delete drafts[key];
    scheduleUiStateSave();
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
    if (!text || !cur || !cur.active || activeServerId === null || sending) return;
    const server = activeServerId;
    const channel = cur.active;
    const reply_to = replyingTo;
    const key = chanKey();
    draft = "";
    replyingTo = "";
    mentionQuery = null;
    if (key) delete drafts[key];
    scheduleUiStateSave();
    sending = true;
    const pendingId = `pending:${Date.now()}:${pendingSendNonce++}`;
    const previousMessages = messages;
    const nextMessages = [...previousMessages, {
      id: pendingId,
      author: myFp,
      text,
      ts: Date.now(),
      edited: 0,
      reactions: [],
      reply_to,
      pinned: false,
    }];
    const nextScope = chatScopeKey(server, channel);
    chatStickToBottom = true;
    messages = nextMessages;
    messageWindow = reconcileChatWindow(
      previousMessages,
      nextMessages,
      messageWindow,
      true,
      messageWindowScope !== nextScope,
    );
    messageWindowScope = nextScope;
    markMessageArrivals([pendingId]);
    await tick();
    try {
      await invoke("send_message", { server, channel, text, replyTo: reply_to });
      sending = false;
      // The channel-updated event normally refreshes this too, but the command acknowledgement is
      // the deterministic local completion point. Do not leave the just-sent message dependent on
      // event scheduling, and do not refresh a different conversation if the user switched away.
      if (activeServerId === server && cur?.active === channel) await refresh();
    } catch (e) {
      error = String(e);
      if (activeServerId === server && cur?.active === channel) {
        messages = messages.filter((m) => m.id !== pendingId);
      }
      // Put the message back only if the user has not already started another one while the send
      // was in flight. A failed send should never silently eat their text.
      if (activeServerId === server && cur?.active === channel && !draft.trim()) {
        draft = text;
        if (key) drafts[key] = text;
        scheduleUiStateSave();
        replyingTo = reply_to;
      }
    } finally {
      sending = false;
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

  async function copyFreshInvite(server: number): Promise<boolean> {
    const invite = await refreshInviteFor(server);
    if (invite === undefined) {
      toast("Invite changed but could not be refreshed; nothing was copied", "err", 4500);
      return false;
    }
    if (!invite) {
      toast("No invite is available for this server", "info", 3000);
      return false;
    }
    try {
      await navigator.clipboard.writeText(wrapInvite(invite, server));
      toast("Friend code copied", "ok", 1800);
      return true;
    } catch {
      // Clipboard may be unavailable in the webview: the textarea allows manual copy.
      toast("Clipboard unavailable: select and copy the code manually", "err", 3500);
      return false;
    }
  }

  async function copyInvite() {
    const server = activeServerId;
    if (server === null) return;
    if (await copyFreshInvite(server)) {
      copied = true;
      setTimeout(() => (copied = false), 1500);
    }
  }

  let mintingInvite = $state(false);
  // Mint a fresh single-use invite on demand (owner or admin: the backend gates on can_invite).
  // The new invite carries the live bootstrap address, so it works even after a restart changed
  // the listen port. An admin's invitee is owner-serialized (admitted when the owner is online).
  async function generateInvite() {
    const server = activeServerId;
    if (server === null || !cur) return;
    mintingInvite = true;
    try {
      const invite = await updateInviteFor(server, true);
      if (invite !== undefined && activeServerId === server) copied = false;
    } finally {
      mintingInvite = false;
    }
  }

  // A short two-note chime via the Web Audio API (no asset to bundle), gated by the
  // notification-sound preference. Played for messages you aren't actively looking at.
  let audioCtx: AudioContext | null = null;
  // An original, asset-free "painted portal" cue: a soft surface whoomp, an elastic
  // upward glide, then three glassy droplets. It begins in the click gesture so a
  // suspended AudioContext can resume, and its crest lands with the visual zoom.
  function playSpacePortal() {
    if (!soundOn || !spaceState.entrySound) return;
    try {
      audioCtx = audioCtx ?? new AudioContext();
      const ctx = audioCtx;
      if (ctx.state === "suspended") void ctx.resume();
      const now = ctx.currentTime;
      const master = ctx.createGain();
      master.gain.setValueAtTime(0.0001, now);
      master.gain.exponentialRampToValueAtTime(0.34, now + 0.025);
      master.gain.setValueAtTime(0.34, now + 0.48);
      master.gain.exponentialRampToValueAtTime(0.0001, now + 0.88);
      master.connect(ctx.destination);

      const whoomp = ctx.createOscillator();
      const whoompGain = ctx.createGain();
      whoomp.type = "sine";
      whoomp.frequency.setValueAtTime(118, now);
      whoomp.frequency.exponentialRampToValueAtTime(54, now + 0.24);
      whoompGain.gain.setValueAtTime(0.46, now);
      whoompGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.28);
      whoomp.connect(whoompGain).connect(master);
      whoomp.start(now);
      whoomp.stop(now + 0.3);

      const glide = ctx.createOscillator();
      const glideGain = ctx.createGain();
      const wobble = ctx.createOscillator();
      const wobbleDepth = ctx.createGain();
      glide.type = "triangle";
      glide.frequency.setValueAtTime(174, now + 0.04);
      glide.frequency.exponentialRampToValueAtTime(392, now + 0.3);
      glide.frequency.exponentialRampToValueAtTime(784, now + 0.58);
      wobble.type = "sine";
      wobble.frequency.value = 12;
      wobbleDepth.gain.setValueAtTime(20, now);
      wobbleDepth.gain.exponentialRampToValueAtTime(2, now + 0.58);
      wobble.connect(wobbleDepth).connect(glide.detune);
      glideGain.gain.setValueAtTime(0.0001, now + 0.04);
      glideGain.gain.exponentialRampToValueAtTime(0.32, now + 0.1);
      glideGain.gain.exponentialRampToValueAtTime(0.0001, now + 0.64);
      glide.connect(glideGain).connect(master);
      glide.start(now + 0.04);
      wobble.start(now + 0.04);
      glide.stop(now + 0.66);
      wobble.stop(now + 0.66);

      [659.25, 880, 1174.66].forEach((frequency, index) => {
        const drop = ctx.createOscillator();
        const dropGain = ctx.createGain();
        const start = now + 0.33 + index * 0.105;
        drop.type = "sine";
        drop.frequency.setValueAtTime(frequency * 0.96, start);
        drop.frequency.exponentialRampToValueAtTime(frequency, start + 0.045);
        dropGain.gain.setValueAtTime(0.0001, start);
        dropGain.gain.exponentialRampToValueAtTime(0.22, start + 0.012);
        dropGain.gain.exponentialRampToValueAtTime(0.0001, start + 0.24);
        drop.connect(dropGain).connect(master);
        drop.start(start);
        drop.stop(start + 0.26);
      });
    } catch {
      /* audio unavailable */
    }
  }
  function playSynthChime(freqs: number[]) {
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

  const activeTonePlayers = new Set<HTMLAudioElement>();
  function playStoredTone(tone: StoredTone, fallback: () => void) {
    try {
      const player = new Audio(tone.dataUrl);
      let failed = false;
      const cleanup = () => {
        activeTonePlayers.delete(player);
        player.onended = null;
        player.onerror = null;
      };
      const fail = () => {
        if (failed) return;
        failed = true;
        cleanup();
        fallback(); // corrupt/unsupported local data never turns notifications silently off
      };
      player.volume = 0.72;
      player.onended = cleanup;
      player.onerror = fail;
      activeTonePlayers.add(player); // hold it until playback ends; GC must not cut a cue short
      void player.play().catch(fail);
    } catch {
      fallback();
    }
  }

  // One policy gate for every notification caller. Custom files are local data URLs validated at
  // import/load; built-ins stay asset-free Web Audio. Server overrides are read even for a server
  // that is not currently open, which is precisely when most message notifications arrive.
  function playConfiguredSound(kind: NotificationSoundKind, server: number | null) {
    const policy = soundPolicy(kind, server);
    if (!policy.enabled) return;
    const builtIn = () => {
      if (kind === "message") playSynthChime([880, 1318.5]);
      else if (kind === "mention") playSynthChime([987.8, 1318.5, 1760]);
      else {
        try {
          audioCtx = audioCtx ?? new AudioContext();
          if (audioCtx.state === "suspended") void audioCtx.resume();
          scheduleNewsChime(audioCtx);
        } catch {
          /* audio unavailable */
        }
      }
    };
    if (policy.custom) playStoredTone(policy.custom, builtIn);
    else builtIn();
  }

  // A regular new-message chime vs a brighter mention/reply triad. Passing the source server is
  // important: notifications generally arrive while some *other* server is open.
  function playNotify(server: number | null = activeServerId) {
    playConfiguredSound("message", server);
  }
  function playMention(server: number | null = activeServerId) {
    playConfiguredSound("mention", server);
  }
  function playNewsTicker(server: number | null = activeServerId) {
    playConfiguredSound("news", server);
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
  type TickerKind = "status" | "wiki" | "event" | "message";
  type TickerItem = { id: string; server: number; kind: TickerKind; text: string; at: number; go: () => void };
  const TICKER_TTL = 5 * 60_000; // news for five minutes; after that it is just history
  const TICKER_MAX = 8;
  let tickerItems = $state<TickerItem[]>([]);
  // Receipts live for the unlocked UI session. Unlike the old feed-coupled set they are never
  // pruned just because a five-minute item aged out, so a replayed backend event cannot crawl or
  // ring a second time. lockScreen clears them because wiki/page ids can contain content names.
  let tickerReceipts = $state<Set<string>>(new Set());
  function pushTicker(kind: TickerKind, server: number, id: string, text: string, go: () => void): boolean {
    if (locked) return false; // nothing that names app content may reach a locked screen
    if (!text.trim()) return false;
    const nextReceipts = acceptTickerReceipt(tickerReceipts, id);
    if (!nextReceipts) return false;
    const at = Date.now();
    const kept = tickerItems.filter((t) => at - t.at < TICKER_TTL);
    tickerItems = [...kept, { id, server, kind, text, at, go }].slice(-TICKER_MAX);
    tickerReceipts = nextReceipts;
    return true;
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
  async function goTickerMessage(server: number, channel: string, messageId: string) {
    navStepStart();
    try {
      inboxView = false;
      if (server !== activeServerId) await switchServer(server);
      view = "chat";
      // A channel update can arrive before its channel-list event. Register a readable fallback
      // so the click can still land instead of silently selecting an absent sidebar entry.
      if (cur && !cur.channels.some((c) => c.id === channel)) {
        cur.channels = [...cur.channels, { id: channel, name: channelNameFor(server, channel) }];
      }
      if (cur?.active !== channel) await switchTo(channel);
      else await refresh();
      jumpToMessageId(messageId);
    } finally {
      navStepEnd();
    }
  }

  function messageTickerText(server: number, channel: string, message: Msg): string {
    const group = servers.find((s) => s.id === server);
    const channelName = group?.channels.find((c) => c.id === channel)?.name ?? "channel";
    // Profiles are scoped to the active server. Never label a cross-server fingerprint with the
    // active server's unrelated profile; the group/channel plus message snippet remains useful.
    const sender = server === activeServerId ? `${nameOf(message.author)}: ` : "";
    return `${group?.name ?? "Server"} · #${channelName} · ${sender}${msgSnippet(message.text, 72)}`;
  }

  function notifyMessage(
    server: number,
    channel: string,
    message: Msg | undefined,
    kind: "message" | "mention",
  ) {
    if (!message?.id) return; // current clients assign stable ids; legacy rows cannot be click targets
    if (server === activeServerId && message.author === myFp) return;
    const accepted = pushTicker(
      "message",
      server,
      messageTickerId(server, channel, message.id),
      messageTickerText(server, channel, message),
      () => void goTickerMessage(server, channel, message.id),
    );
    // A repeated channel-updated event (reaction, topic, duplicate bridge delivery) must not ring
    // for the same row again. The ticker receipt is the shared exactly-once gate for both signals.
    if (!accepted) return;
    if (kind === "mention") playMention(server);
    else playNotify(server);
  }

  async function notifyLatestChannelMessage(
    server: number,
    channel: string,
    mode: "message" | "mention" | "detect",
  ) {
    try {
      const channelMessages = await invoke<Msg[]>("get_messages", { server, channel });
      const latest = channelMessages[channelMessages.length - 1];
      let kind: "message" | "mention" = mode === "mention" ? "mention" : "message";
      // Mention detection depends on the active server's identity/profile and read marks. If the
      // user switches servers during this fetch, degrade to an ordinary message notification.
      if (mode === "detect" && server === activeServerId && targetsMe(channel, channelMessages)) {
        if (!mentionChannels.has(channel)) mentionChannels = new Set(mentionChannels).add(channel);
        kind = "mention";
      }
      notifyMessage(server, channel, latest, kind);
    } catch {
      // Without a stable message id there is nothing safe to put in a clickable ticker. Preserve
      // the audible alert, but do not create a headline that cannot land anywhere.
      if (mode === "mention") playMention(server);
      else playNotify(server);
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
          : activeTransfers > 0
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
          : activeTransfers > 0
            ? `busy: ${activeTransfers} transfer${activeTransfers === 1 ? "" : "s"} running`
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
  // Newest six unshown: a burst drops its oldest rather than crawling for minutes.
  let tbQueue = $derived(tickerItems.slice(-6));
  let tbHead = $derived(tbQueue[0] ?? null);
  // The same thresholds the voice stage's meter uses, so the two readings of one mic agree.
  let tbMicBars = $derived([0, 0.25, 0.5, 0.75].filter((t) => micLevel > t).length);
  const tbCrawlDur = (text: string) => Math.min(24, Math.max(10, 8.5 + text.length * 0.11));
  function tbAdvance(id: string) {
    tickerItems = tickerItems.filter((item) => item.id !== id);
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
    // Reconnect a controller that was already granted and already plugged in, silently. Waiting
    // for the instrument drawer to be opened once was half of why MIDI felt like a coin toss.
    void primeMidi();
    // Look for a new release shortly after launch rather than during it: the first seconds
    // belong to unlocking and reconnecting, and nothing here is urgent.
    const updateTimer = setTimeout(() => void checkForUpdate(), 4000);
    // F5/HMR remounts only this webview; the native process and unlocked actors are still alive.
    // Resume that session unless the user explicitly pressed Ctrl+L. A cold launch still draws
    // the ordinary vault gate.
    let explicitlyLocked = false;
    try { explicitlyLocked = sessionStorage.getItem("catcoms.explicit-lock") === "1"; } catch { /* best effort */ }
    const chooseGate = () => invoke<boolean>("vault_exists")
      .then((v) => (vaultExists = v))
      .catch(() => (vaultExists = true));
    if (explicitlyLocked) chooseGate();
    else invoke<Reloaded[] | null>("resume_session")
      .then((running) => {
        if (running) {
          vaultExists = true;
          restoreReloaded(running);
        } else chooseGate();
      })
      .catch(chooseGate);
    const subs: Promise<UnlistenFn>[] = [
      appWindow.onResized(() => syncMaximized()),
      appWindow.onFocusChanged(({ payload }) => (windowFocused = payload)),
      listen<{ server: number }>("channels-changed", (e) => {
        void refreshChannels(e.payload.server);
      }),
      listen<{ server: number; channel: string }>("channel-updated", (e) => {
        const { server, channel } = e.payload;
        spaceActivityAt[server] = Date.now();
        if (server === activeServerId && view === "moderation") void refreshModeration();
        // Any server's channel changed → the cross-server inbox may have a new entry (debounced).
        scheduleInboxReload();
        // A DM got a message → its activity stats changed; keep the friends sorting fresh.
        if (dmHome && servers.find((x) => x.id === server)?.isDm) refreshDmStats();
        // Jukebox edits ride the same event, and the room I'm listening in need not be the one I'm looking at.
        if (inCall && server === callServer && channel === callChannel) void refreshJukebox();
        if (server === activeServerId && channel === cur?.active) {
          refreshTopic(); // topic edits ride the same channel-updated event
          channelEventRefresh.request(true).then(() => {
            // You're looking at this channel: only notify if the window isn't focused. The same
            // stable message id gates its sound and its clickable ticker receipt exactly once.
            if (document.hasFocus()) return;
            // request() hands back ONE promise for the whole drain, so this can resolve after a
            // pass that loaded a different conversation: reading the shared array then would
            // headline THIS channel with ANOTHER one's text, and the ticker click would try to
            // land a foreign message id here and silently do nothing. Fetch our own rows in that
            // case, exactly as the non-active-channel branch below always does.
            if (!scopeHoldsConversation(messageWindowScope, server, channel)) {
              void notifyLatestChannelMessage(server, channel, "detect");
              return;
            }
            const last = messages[messages.length - 1];
            const forMe =
              last &&
              last.author !== myFp &&
              (mentionsMe(last.text) || (!!last.reply_to && msgById.get(last.reply_to)?.author === myFp));
            notifyMessage(server, channel, last, forMe ? "mention" : "message");
          });
          return;
        }
        const s = servers.find((x) => x.id === server);
        if (s && s.channels.some((c) => c.id === channel)) {
          if (!s.unread.includes(channel)) s.unread.push(channel);
          if (server !== activeServerId) s.dot = true;
          if (server !== activeServerId) {
            // Another server: its profile identity is not loaded, so this is an ordinary-message
            // alert, but its own server sound override still applies.
            void notifyLatestChannelMessage(server, channel, "message");
          } else if (mentionChannels.has(channel)) {
            void notifyLatestChannelMessage(server, channel, "mention");
          } else {
            // A non-active channel of the server I'm in: scan for a message aimed at me, then use
            // that same fetched row for the ticker so its click target and sound cannot diverge.
            void notifyLatestChannelMessage(server, channel, "detect");
          }
        }
      }),
      listen<{ server: number; count: number }>("members-changed", (e) => {
        spaceActivityAt[e.payload.server] = Date.now();
        if (e.payload.server === activeServerId) {
          refreshMembers();
          if (view === "files") refreshFiles(); // membership change ⇒ re-check fetch availability
        }
      }),
      listen<{ server: number }>("profiles-updated", (e) => {
        if (e.payload.server === activeServerId) refreshProfiles();
        // A rename on the room's server has to reach the dock even while another server is
        // being viewed, which is precisely when the active-server refresh above does nothing.
        if (e.payload.server === callServer) void refreshCallProfiles();
      }),
      listen<{ server: number }>("files-updated", (e) => {
        spaceActivityAt[e.payload.server] = Date.now();
        if (e.payload.server === activeServerId) {
          refreshFiles();
          if (view === "storage" || view === "downloads") refreshStorageHealth();
        }
      }),
      listen<{
        server: number;
        cid: string;
        done: number;
        total: number;
        bytes_done: number;
        bytes_total: number;
        network_bytes_done: number;
        provider: string | null;
      }>("download-progress", (e) => {
        // The deck reads the same events the Downloads surface does. It has no download of its
        // own to key off: the media element pulls ranges, and the backend emits progress for the
        // chunks it needs, so this is the only view the deck gets of how a track is coming in.
        if (jukeNow?.cid === e.payload.cid) jukeFetch = fetchPhase(e.payload);
        const d = downloads[dlKey(e.payload.server, e.payload.cid)];
        if (!d) return; // only track explicitly-initiated downloads
        const now = Date.now();
        if (e.payload.network_bytes_done > d.networkBytesDone) {
          d.speed = sampleRate(
            d.speed,
            d.networkBytesDone,
            d.lastRateAt,
            e.payload.network_bytes_done,
            now,
          );
          d.lastRateAt = now;
        }
        d.networkBytesDone = e.payload.network_bytes_done;
        d.total = e.payload.total;
        d.done = Math.max(d.heldBefore, e.payload.done);
        d.bytesTotal = e.payload.bytes_total;
        d.bytesDone = Math.max(d.bytesDone, e.payload.bytes_done);
        d.progress = d.total > 0 ? Math.min(1, d.done / d.total) : 0;
        d.updatedAt = now;
        if (e.payload.done === 0) d.provider = undefined; // fresh transfer: drop any prior provider
        if (e.payload.provider) d.provider = e.payload.provider; // keep the latest live provider
        if (e.payload.done >= e.payload.total) d.status = "verifying";
        else if (!d.provider && onlineCount <= 1) d.status = "waiting";
        else d.status = "downloading";
      }),
      listen<{ server: number; upload_id: string; done: number; total: number }>("upload-progress", (e) => {
        const u = uploads[uploadKey(e.payload.server, e.payload.upload_id)];
        if (!u || u.status === "done" || u.status === "failed") return;
        const chunkTotal = Math.max(1, e.payload.total - 1);
        u.total = chunkTotal;
        u.done = Math.min(chunkTotal, e.payload.done);
        u.status = e.payload.done >= e.payload.total - 1 ? "publishing" : "uploading";
        u.updatedAt = Date.now();
        // Reading owns 0..10%. Backend work owns the rest, but only the completed invoke marks
        // the row Done: keep event progress just shy of 100% until persistence has also returned.
        const backend = e.payload.total > 0 ? e.payload.done / e.payload.total : 0;
        u.progress = Math.max(u.progress, Math.min(0.99, 0.1 + backend * 0.89));
      }),
      listen<{ server: number }>("status-updated", (e) => {
        spaceActivityAt[e.payload.server] = Date.now();
        if (e.payload.server === activeServerId) refreshStatuses();
        if (!(e.payload.server === activeServerId && view === "status" && document.hasFocus())) {
          newsUnseen = true;
        }
        if (inboxView && inboxMode === "news") { newsUnseen = false; loadNews(); }
      }),
      listen<{ server: number }>("wiki-updated", (e) => {
        spaceActivityAt[e.payload.server] = Date.now();
        if (e.payload.server === activeServerId) refreshWiki();
      }),
      listen<{ server: number }>("roles-updated", (e) => {
        if (e.payload.server === activeServerId) refreshRoles();
      }),
      listen<{ server: number }>("moderation-updated", (e) => {
        if (e.payload.server === activeServerId) refreshModeration();
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
        spaceActivityAt[e.payload.server] = Date.now();
        if (e.payload.server === activeServerId) refreshEvents();
        if (!(e.payload.server === activeServerId && view === "events" && document.hasFocus())) {
          newsUnseen = true;
        }
        if (inboxView && inboxMode === "news") { newsUnseen = false; loadNews(); }
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
        spaceOnlineCounts[e.payload.server] = e.payload.online.length + 1;
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
      listen<JoinReplyReady>("join-reply-ready", (e) => {
        // The native join command deliberately remains pending while its listener and NAT mapping
        // stay alive. This event gives the human the return signalling channel without moving the
        // punch deadline into a throttled webview timer.
        joinReplyReady = e.payload;
        notice = "Send the connection reply back to the inviter now; keep this app open.";
      }),
      listen<number>("reachability-changed", (e) => {
        // Router mapping and AutoNAT settle after founding/joining returns. Both onboarding and
        // Settings use this same report. Refresh that server's cached invite too: the event can
        // arrive while another server is active, and copied codes must never retain an expired
        // mapping or relay route.
        if (locked) return;
        if (reachabilityEventAffectsReport(connectivity, e.payload)) refreshConnectivity();
        if (e.payload === activeServerId && view === "connectivity") refreshSwitchboards();
        void refreshInviteFor(e.payload);
      }),
      listen<number>("switchboard-changed", (e) => {
        const decision = switchboardEventRefreshDecision(locked, activeServerId, e.payload);
        if (decision.refreshStatus) refreshSwitchboards();
        if (decision.refreshInvite) void refreshInviteFor(e.payload);
      }),
      listen<{ server: number; caution: boolean }>("eclipse-changed", (e) => {
        if (e.payload.server === activeServerId) eclipseCaution = e.payload.caution;
      }),
      listen<{ server: number }>("server-closed", (e) => {
        servers = servers.filter((s) => s.id !== e.payload.server);
        if (activeServerId === e.payload.server) {
          if (servers.length) void switchServer(servers[0].id);
          else {
            beginViewSwitch();
            activeServerId = null;
            clearServerView();
          }
        }
      }),
    ];
    // Global keyboard shortcuts: Escape closes the top-most overlay/menu; Ctrl/Cmd+1–5 switch
    // tabs; Ctrl/Cmd+K opens the quick switcher.
    const onKey = (e: KeyboardEvent) => {
      if (!locked && !e.repeat) {
        const target = activeTextEffectTarget();
        const effect = target ? effectForKeybind(textEffectKeybinds, keybindFromEvent(e)) : "";
        if (target && effect) {
          e.preventDefault();
          if (captureTextEffectSelection(target)) void applyTextEffect(effect, target);
          return;
        }
      }
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
      if (spaceOpen && (e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) redoSpaceLayout();
        else undoSpaceLayout();
        return;
      }
      if (handleSpaceKey(e)) return;
      if (e.key === "Escape") {
        if (textEffectTarget) { textEffectTarget = null; showTextEffectCatalog = false; }
        else if (showQuickSwitch) closeQuickSwitch();
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
        // The space folds last: entry, carrying, and the tray release first, then the view itself.
        else if (spaceOpen && spaceEntering !== null) {
          clearTimeout(spaceEnterTimer);
          stopSpaceCameraTween();
          spaceEntering = null;
          spaceEntryPhase = null;
        }
        else if (spaceOpen && spaceCarried) {
          spaceCarried = null;
          spaceDrag = null;
        }
        else if (spaceOpen && spaceTrayPinned) spaceTrayPinned = false;
        else if (spaceOpen) {
          clearTimeout(spaceEnterTimer);
          stopSpaceCameraTween();
          spaceEntering = null;
          spaceEntryPhase = null;
          spaceOpen = false;
        }
        return;
      }
      // Hold T while the space is up: the draggable server tray slides out.
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
      textEffectTarget = null;
      showTextEffectCatalog = false;
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
    const stopTextEffects = mountTextEffectRuntime();
    // Keep relative presence times current.
    const tick = setInterval(() => {
      nowTick = Date.now();
      pruneTicker(); // stale news stops being news
    }, 60_000);
    // A moving transfer must stop looking active when no new chunk has arrived. This small UI-only
    // clock drives that freshness check; it does not poll the network or alter transfer state.
    const transferTick = setInterval(() => {
      transferNow = Date.now();
      joinReplyNow = transferNow;
    }, 1_000);
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
      stopTextEffects();
      releaseAll();
      stopPlayback();
      clearInterval(tick);
      clearInterval(transferTick);
      clearInterval(blink);
      clearInterval(callCleanup);
      clearTimeout(inboxTimer);
      if (inboxIdle !== undefined && "cancelIdleCallback" in window) window.cancelIdleCallback(inboxIdle);
      clearTimeout(updateTimer);
      clearInterval(pingTimer);
      subs.forEach((p) => p.then((un) => un()));
    };
  });
</script>

{#snippet styledName(name: string, color: string, font: string, effect: string)}
  {@const effects = decodeNameEffects(effect)}
  {@const letters = nameLetters(name)}
  {@const mexican = effectEnabled(effects, "mexican")}
  {@const mexicanOptions = effectOptions(effects, "mexican")}
  <span class="name {fontClass(font)} {fxClass(effect)}" style={colorStyle(color) + fxStyle(effect)} aria-label={name} data-name={name}>
    {#if mexican}
      {#each letters as letter, i (i)}
        <span
          class="fx-letter"
          aria-hidden="true"
          style={`animation-delay:${(-1 * (mexicanOptions.direction === -1 ? letters.length - i - 1 : i) * (0.025 + (mexicanOptions.spread ?? 4) * 0.0095)).toFixed(3)}s`}
        >{letter === " " ? "\u00a0" : letter}</span>
      {/each}
    {:else}{name}{/if}
  </span>
{/snippet}

<!-- The connectivity detail, rendered by BOTH the create/join screen and Settings ->
     Diagnostics: one panel, two doors. It reports what was TRIED, never a verdict the code
     cannot support (see `reachabilitySummary`: AutoNAT v2 proves one address/observer pair at
     one moment, not universal reachability). -->
{#snippet connDetail(c: Connectivity)}
  {@const status = connectivityStatus(c)}
  <div class="conn-detail">
    <div class="reach-status-line" data-tone={status.tone}>
      <span class="reach-status-dot" aria-hidden="true"></span>
      <span class="reach-status-key">{status.key}</span>
      <span class="reach-status-body">{status.sentence}</span>
    </div>
    <pre class="conn-readout">{connectivityReadout(c)}</pre>
    {#if status.tone === "warn"}
      <div class="conn-diagnosis">
        <b>No outside route is proven yet.</b>
        {#if automaticMappingUnavailable(c.upnp)}
          This router did not provide an automatic mapping. A manual port forward, a known public
          address, or a relay can provide the missing route.
        {:else}
          Refresh after the remote check settles, or configure a relay when this network does not
          accept incoming connections.
        {/if}
      </div>
    {/if}
    <h4>Automatic port mapping (UPnP / PCP / NAT-PMP)</h4>
    <p class="muted small">{c.upnp || "not attempted"}</p>
    <h4>Remote dial-back (AutoNAT)</h4>
    <p class="muted small">{c.autonat || "not tested"}</p>
    <h4>What connected peers observed ({c.mesh_observations?.length ?? 0})</h4>
    {#if c.mesh_observations?.length}
      <ul class="conn-addrs">
        {#each c.mesh_observations as observation, i (i)}<li class="fp">{observation}</li>{/each}
      </ul>
      <p class="muted small">Diagnostic only. These are outbound source sockets reported by peers, not verified listener addresses; Mewtual never puts them in an invite or dials them.</p>
    {:else}
      <p class="muted small">No connected peer has reported an outbound source address yet.</p>
    {/if}
    <h4>Addresses this device offers ({c.advertised.length})</h4>
    {#if c.advertised.length}
      <ul class="conn-addrs">
        {#each c.advertised as a, i (i)}<li class="fp">{a}</li>{/each}
      </ul>
    {:else}
      <p class="muted small">None yet. Without one, someone on another network has nothing to dial.</p>
    {/if}
    <h4>What the attempt did ({c.steps.length})</h4>
    {#if c.steps.length}
      <ul class="conn-steps">
        {#each c.steps as st, i (i)}
          <li class={st.status}>
            <span class="cs-kind">{st.kind}</span>
            {#if st.target}<span class="fp cs-target">{st.target}</span>{/if}
            <span class="cs-detail">{st.detail}</span>
          </li>
        {/each}
      </ul>
      <p class="muted small">
        Addresses marked <b>unknown</b> were dialled and nothing more is known about them
        individually: they are attempted at once and only the first to answer is reported.
      </p>
    {:else}
      <p class="muted small">Nothing recorded.</p>
    {/if}
    {#if c.last_error}
      <h4>Last error</h4>
      <textarea class="invite-code" readonly rows="2" value={c.last_error}></textarea>
    {/if}
  </div>
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

<!--
  The call's own address: which server the room is on, and which channel. Rendered by both dock
  shapes. While the viewed server IS the call's server the channel alone is unambiguous, so the
  server name is dropped to keep the bar tight; the moment they diverge the full identity
  appears and becomes the way back. Advisory gold, not danger: nothing is wrong, you are just
  looking somewhere else.
-->
{#snippet callServerTag()}
  {#if callElsewhere}
    <button
      class="call-srv away"
      title={`This call is on ${callSrvLabel}: click to go back to it`}
      onclick={() => { if (callServer !== null) switchServer(callServer); }}
    >
      {#if callServer !== null && serverIcons[callServer]}
        <img class="call-srv-ico" src={imgSrc(serverIcons[callServer])} alt="" />
      {/if}
      <span class="call-srv-nm">{callSrvLabel}</span>
      <span class="call-srv-ch">#{callChannelName}</span>
    </button>
    <span class="stage-chip away-chip">VIEWING ELSEWHERE</span>
  {:else}
    <span class="call-srv">
      {#if callServer !== null && serverIcons[callServer]}
        <img class="call-srv-ico" src={imgSrc(serverIcons[callServer])} alt="" />
      {/if}
      <span class="call-srv-ch">#{callChannelName}</span>
    </span>
  {/if}
{/snippet}

<!--
  Call-surface twins of nameTag/avatarTag. Identical rendering, different lookup: these resolve
  through the room server's profile map so switching servers mid-call cannot rename the room.
-->
{#snippet callNameTag(fp: string)}
  {@const p = callProfileFor(fp)}
  {@render styledName(callNameOf(fp), p?.color ?? "", p?.font ?? "", p?.effect ?? "")}
{/snippet}

{#snippet callAvatarTag(fp: string)}
  {@const p = callProfileFor(fp)}
  {#if p?.avatar}
    <img class="avatar" src={imgSrc(p.avatar)} alt="" />
  {:else}
    <span class="avatar fallback" style={p?.color ? `background:${p.color}` : ""}>
      {callNameOf(fp).slice(0, 1).toUpperCase()}
    </span>
  {/if}
{/snippet}

{#snippet textEffectButton(target: TextEffectTarget, label = "Text effects")}
  <button
    type="button"
    class="text-fx-trigger"
    class:active={textEffectTarget === target}
    title={`${label}: select text for the quick Aa strip, or open the full catalog`}
    aria-label={label}
    onclick={() => openTextEffectCatalog(target)}
  ><span>Aa</span><b>FX</b></button>
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
    <div class="field name-studio">
      <div class="name-studio-head">
        <span class="muted">Name Style Studio</span>
        <div class="name-studio-actions">
          <button type="button" class="ghost small" disabled={!styleUndo.length} onclick={undoNameStyle} title="Undo the last unsaved style change">↶ Undo</button>
          <button type="button" class="ghost small" disabled={!styleRedo.length} onclick={redoNameStyle} title="Redo the last undone style change">↷ Redo</button>
          <button type="button" class="ghost small" onclick={randomizeNameStyle}>⚄ Randomise</button>
        </div>
      </div>
      <span class="muted small">Recipes change the draft only; Save profile publishes it to this server.</span>
      <div class="name-recipes">
        {#each BUILTIN_NAME_RECIPES as recipe (recipe.id)}
          <button type="button" class="name-recipe" title={`Load ${recipe.name}`} onclick={() => applyNameRecipe(recipe)}>
            {@render styledName(recipe.name, recipe.color, recipe.font, recipe.effect)}
          </button>
        {/each}
      </div>
      {#if savedNameRecipes.length}
        <span class="name-studio-label">MY LIBRARY · AVAILABLE ON EVERY SERVER</span>
        <div class="saved-recipes">
          {#each savedNameRecipes as recipe (recipe.id)}
            <span class="saved-recipe">
              <button type="button" class="name-recipe" title={`Load ${recipe.name}`} onclick={() => applyNameRecipe(recipe)}>
                {@render styledName(recipe.name, recipe.color, recipe.font, recipe.effect)}
              </button>
              <button type="button" class="recipe-delete" aria-label={`Delete saved recipe ${recipe.name}`} title="Delete this saved recipe" onclick={() => deleteNameRecipe(recipe.id)}>✕</button>
            </span>
          {/each}
        </div>
      {/if}
      <div class="recipe-save">
        <input value={recipeNameDraft} maxlength="32" placeholder="Recipe name" aria-label="New recipe name" oninput={(e) => (recipeNameDraft = e.currentTarget.value)} onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); saveNameRecipe(); } }} />
        <button type="button" class="ghost small" disabled={!recipeNameDraft.trim()} onclick={saveNameRecipe}>Save current to library</button>
      </div>
    </div>
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
            onclick={() => setNameFont(f.id)}
          ><span class="name {fontClass(f.id)}">Gg</span></button>
        {/each}
      </div>
    </div>
    <div class="field type-studio">
      <span class="muted">Typography</span>
      <div class="fx-option-grid">
        <label class="fx-option"><span>Weight <output>{effectOptions(pEffects, "typography").weight}</output></span><input type="range" min="400" max="900" step="100" value={effectOptions(pEffects, "typography").weight} oninput={(e) => updateStudioOption("typography", "weight", e.currentTarget.valueAsNumber)} /></label>
        <label class="fx-option"><span>Letter spacing <output>{effectOptions(pEffects, "typography").tracking}px</output></span><input type="range" min="-1" max="6" step="0.1" value={effectOptions(pEffects, "typography").tracking} oninput={(e) => updateStudioOption("typography", "tracking", e.currentTarget.valueAsNumber)} /></label>
        <label class="fx-option"><span>Bubble thickness <output>{effectOptions(pEffects, "typography").bubble}px</output></span><input type="range" min="0" max="3" step="0.25" value={effectOptions(pEffects, "typography").bubble} oninput={(e) => updateStudioOption("typography", "bubble", e.currentTarget.valueAsNumber)} /></label>
      </div>
      <div class="type-toggles">
        <label><input type="checkbox" checked={effectOptions(pEffects, "typography").italic} onchange={(e) => updateStudioOption("typography", "italic", e.currentTarget.checked)} /> Italic</label>
        <label><input type="checkbox" checked={effectOptions(pEffects, "typography").uppercase} onchange={(e) => updateStudioOption("typography", "uppercase", e.currentTarget.checked)} /> Uppercase</label>
        <button type="button" class="ghost small" onclick={() => resetNameEffect("typography")}>Reset typography</button>
      </div>
    </div>
    <div class="field">
      <div class="effect-field-head">
        <span class="muted">Effects</span>
        <button
          type="button"
          class="ghost small"
          class:active={!appliedEffects.some((effect) => effect.enabled)}
          title="Temporarily turn every effect off without losing its settings"
          aria-label="All effects off"
          aria-pressed={!appliedEffects.some((effect) => effect.enabled)}
          onclick={disableAllNameEffects}
        >All off</button>
      </div>
      <div class="effect-catalog">
        {#each EFFECT_GROUPS as group}
          <div class="effect-catalog-group">
            <span class="name-studio-label">{group}</span>
            <div class="ns-tiles">
              {#each NAME_EFFECTS.filter((effect) => effect.group === group) as fx}
                {@const dead = fxMotionOff && movingNameEffect(fx.id)}
                {@const configured = effectConfigured(pEffects, fx.id)}
                <button
                  type="button"
                  class="ns-tile"
                  class:active={configured}
                  class:effect-off={configured && !effectEnabled(pEffects, fx.id)}
                  class:motion-dead={dead}
                  title={dead ? `${fx.label}: this one animates, and motion is off (Appearance: Hover motion, or the system's reduced-motion)` : configured ? `${fx.label}: show its saved settings` : `Add ${fx.label}`}
                  aria-label={fx.label}
                  aria-pressed={effectEnabled(pEffects, fx.id)}
                  onclick={() => selectNameEffect(fx.id)}
                >{@render styledName(fx.label, pColor, pFont, encodeNameEffects([defaultNameEffect(fx.id)]))}</button>
              {/each}
            </div>
          </div>
        {/each}
      </div>
      <span class="muted small">Add as many as you like, then tick combinations on and off below. Fill effects, and Bounce/Wobble, preserve every setup while enabling one compatible choice at a time.</span>

      <div class="fx-master">
        <label class="fx-option"><span>Master intensity <output>{effectOptions(pEffects, "master").intensity}%</output></span><input type="range" min="25" max="175" step="5" value={effectOptions(pEffects, "master").intensity} oninput={(e) => updateStudioOption("master", "intensity", e.currentTarget.valueAsNumber)} /></label>
        <label class="fx-option"><span>Animation speed <output>{effectOptions(pEffects, "master").speed}%</output></span><input type="range" min="25" max="200" step="5" value={effectOptions(pEffects, "master").speed} oninput={(e) => updateStudioOption("master", "speed", e.currentTarget.valueAsNumber)} /></label>
        <button type="button" class="ghost small" onclick={() => resetNameEffect("master")}>Reset master</button>
      </div>

      {#if appliedEffects.length}
        <div class="fx-settings-list" aria-label="Applied effect options">
          {#each appliedEffects as active, ai (active.id)}
            {@const definition = NAME_EFFECTS.find((effect) => effect.id === active.id)}
            <section
              class="fx-settings"
              class:effect-off={!active.enabled}
              class:dragging={draggedNameEffect === active.id}
              role="group"
              aria-label={`${definition?.label ?? active.id} effect settings`}
              ondragover={(e) => e.preventDefault()}
              ondrop={(e) => { e.preventDefault(); dropNameEffect(active.id); }}
            >
              <div class="fx-settings-head">
                <div class="fx-settings-label">
                  <button
                    type="button"
                    class="fx-drag"
                    draggable="true"
                    aria-label={`Drag ${definition?.label ?? active.id} to reorder; arrow controls are also available`}
                    title="Drag to reorder"
                    ondragstart={() => (draggedNameEffect = active.id)}
                    ondragend={() => (draggedNameEffect = null)}
                  >⠿</button>
                  <button
                    type="button"
                    class="fx-settings-title"
                    aria-expanded={!collapsedEffects[active.id]}
                    onclick={() => (collapsedEffects[active.id] = !collapsedEffects[active.id])}
                  >
                    <span class="fx-chevron" aria-hidden="true">{collapsedEffects[active.id] ? "▸" : "▾"}</span>
                    <span>
                      <strong>{definition?.label ?? active.id}</strong>
                      <span class="muted small">{definition?.description ?? ""}</span>
                    </span>
                  </button>
                </div>
                <div class="fx-settings-actions">
                  <label class="fx-enabled">
                    <input type="checkbox" checked={active.enabled} onchange={(e) => setNameEffectEnabled(active.id, e.currentTarget.checked)} />
                    <span>{active.enabled ? "On" : "Off"}</span>
                  </label>
                  <button type="button" class="ghost fx-order" disabled={ai === 0} aria-label={`Move ${definition?.label ?? active.id} up`} onclick={() => moveNameEffect(active.id, -1)}>↑</button>
                  <button type="button" class="ghost fx-order" disabled={ai === appliedEffects.length - 1} aria-label={`Move ${definition?.label ?? active.id} down`} onclick={() => moveNameEffect(active.id, 1)}>↓</button>
                  <button type="button" class="ghost small" onclick={() => resetNameEffect(active.id)}>Reset</button>
                  <button type="button" class="ghost small" aria-label={`Remove ${definition?.label ?? active.id} and forget its settings`} onclick={() => removeNameEffect(active.id)}>Remove</button>
                </div>
              </div>

              {#if !collapsedEffects[active.id]}
                <div class="fx-settings-body">
                {#if active.id === "gradient"}
                <div class="grad-maker">
                  {#each active.options.stops ?? [] as stop, si (si)}
                    <span class="grad-stop">
                      <input type="color" value={stop} aria-label={`Gradient stop ${si + 1}`} oninput={(e) => updateGradientStop(si, e.currentTarget.value)} />
                      {#if (active.options.stops?.length ?? 0) > 2}
                        <button type="button" class="grad-del" title="Remove this stop" aria-label={`Remove gradient stop ${si + 1}`} onclick={() => removeGradientStop(si)}>✕</button>
                      {/if}
                    </span>
                  {/each}
                  {#if (active.options.stops?.length ?? 0) < GRAD_MAX_STOPS}
                    <button type="button" class="ghost small" onclick={addGradientStop}>＋ stop</button>
                  {/if}
                </div>
                <label class="fx-option"><span>Angle <output>{active.options.angle}°</output></span><input type="range" min="0" max="360" step="15" value={active.options.angle} oninput={(e) => updateNameEffect(active.id, "angle", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Scroll <output>{active.options.speed ? `speed ${active.options.speed}` : "still"}</output></span><input type="range" min="0" max="10" step="1" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <button type="button" class="ghost small fx-direction" disabled={!active.options.speed} onclick={() => updateNameEffect(active.id, "direction", active.options.direction === -1 ? 1 : -1)}>{active.options.direction === -1 ? "◀ reverse" : "▶ forward"}</button>
              {:else if active.id === "rainbow"}
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <button type="button" class="ghost small fx-direction" onclick={() => updateNameEffect(active.id, "direction", active.options.direction === -1 ? 1 : -1)}>{active.options.direction === -1 ? "◀ reverse" : "▶ forward"}</button>
              {:else if active.id === "neon"}
                <label class="fx-option"><span>Glow size <output>{active.options.glow}px</output></span><input type="range" min="2" max="18" value={active.options.glow} oninput={(e) => updateNameEffect(active.id, "glow", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Brightness <output>{active.options.intensity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.intensity} oninput={(e) => updateNameEffect(active.id, "intensity", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "wave"}
                <label class="fx-option"><span>Height <output>{active.options.height}px</output></span><input type="range" min="1" max="8" value={active.options.height} oninput={(e) => updateNameEffect(active.id, "height", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "mexican"}
                <label class="fx-option"><span>Height <output>{active.options.height}px</output></span><input type="range" min="1" max="10" value={active.options.height} oninput={(e) => updateNameEffect(active.id, "height", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Letter spread <output>{active.options.spread}</output></span><input type="range" min="1" max="10" value={active.options.spread} oninput={(e) => updateNameEffect(active.id, "spread", e.currentTarget.valueAsNumber)} /></label>
                <button type="button" class="ghost small fx-direction" onclick={() => updateNameEffect(active.id, "direction", active.options.direction === -1 ? 1 : -1)}>{active.options.direction === -1 ? "◀ right to left" : "▶ left to right"}</button>
              {:else if active.id === "pulse"}
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Fade depth <output>{active.options.depth}%</output></span><input type="range" min="15" max="85" step="5" value={active.options.depth} oninput={(e) => updateNameEffect(active.id, "depth", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "outline"}
                <label class="fx-option"><span>Thickness <output>{active.options.width}px</output></span><input type="range" min="0.5" max="3" step="0.5" value={active.options.width} oninput={(e) => updateNameEffect(active.id, "width", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-color-option"><span>Outline colour</span><input type="color" value={active.options.color} oninput={(e) => updateNameEffect(active.id, "color", e.currentTarget.value)} /></label>
              {:else if active.id === "shadow"}
                <div class="fx-option-grid">
                  <label class="fx-option"><span>X <output>{active.options.x}px</output></span><input type="range" min="-8" max="8" value={active.options.x} oninput={(e) => updateNameEffect(active.id, "x", e.currentTarget.valueAsNumber)} /></label>
                  <label class="fx-option"><span>Y <output>{active.options.y}px</output></span><input type="range" min="-8" max="8" value={active.options.y} oninput={(e) => updateNameEffect(active.id, "y", e.currentTarget.valueAsNumber)} /></label>
                  <label class="fx-option"><span>Blur <output>{active.options.blur}px</output></span><input type="range" min="0" max="16" value={active.options.blur} oninput={(e) => updateNameEffect(active.id, "blur", e.currentTarget.valueAsNumber)} /></label>
                  <label class="fx-option"><span>Opacity <output>{active.options.opacity}%</output></span><input type="range" min="10" max="100" step="5" value={active.options.opacity} oninput={(e) => updateNameEffect(active.id, "opacity", e.currentTarget.valueAsNumber)} /></label>
                </div>
                <label class="fx-color-option"><span>Shadow colour</span><input type="color" value={active.options.color} oninput={(e) => updateNameEffect(active.id, "color", e.currentTarget.value)} /></label>
              {:else if active.id === "retro"}
                <label class="fx-option"><span>Offset <output>{active.options.offset}px</output></span><input type="range" min="1" max="6" value={active.options.offset} oninput={(e) => updateNameEffect(active.id, "offset", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Strength <output>{active.options.opacity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.opacity} oninput={(e) => updateNameEffect(active.id, "opacity", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "glitch"}
                <label class="fx-option"><span>Spread <output>{active.options.spread}px</output></span><input type="range" min="1" max="5" value={active.options.spread} oninput={(e) => updateNameEffect(active.id, "spread", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Strength <output>{active.options.opacity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.opacity} oninput={(e) => updateNameEffect(active.id, "opacity", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "shimmer"}
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Highlight <output>{active.options.intensity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.intensity} oninput={(e) => updateNameEffect(active.id, "intensity", e.currentTarget.valueAsNumber)} /></label>
                <button type="button" class="ghost small fx-direction" onclick={() => updateNameEffect(active.id, "direction", active.options.direction === -1 ? 1 : -1)}>{active.options.direction === -1 ? "◀ reverse" : "▶ forward"}</button>
              {:else if active.id === "sparkle"}
                <label class="fx-option"><span>Twinkle speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Brightness <output>{active.options.intensity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.intensity} oninput={(e) => updateNameEffect(active.id, "intensity", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "wobble"}
                <label class="fx-option"><span>Speed <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Tilt <output>{active.options.amount}°</output></span><input type="range" min="1" max="8" step="0.5" value={active.options.amount} oninput={(e) => updateNameEffect(active.id, "amount", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "candy"}
                <div class="fx-colour-pair">
                  <label class="fx-color-option"><span>Stripe one</span><input type="color" value={active.options.color} oninput={(e) => updateNameEffect(active.id, "color", e.currentTarget.value)} /></label>
                  <label class="fx-color-option"><span>Stripe two</span><input type="color" value={active.options.secondary} oninput={(e) => updateNameEffect(active.id, "secondary", e.currentTarget.value)} /></label>
                </div>
                <label class="fx-option"><span>Angle <output>{active.options.angle}°</output></span><input type="range" min="0" max="360" step="15" value={active.options.angle} oninput={(e) => updateNameEffect(active.id, "angle", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Scroll <output>{active.options.speed ? `speed ${active.options.speed}` : "still"}</output></span><input type="range" min="0" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "ghost"}
                <label class="fx-option"><span>Opacity <output>{active.options.opacity}%</output></span><input type="range" min="20" max="95" step="5" value={active.options.opacity} oninput={(e) => updateNameEffect(active.id, "opacity", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Frost <output>{active.options.blur}px</output></span><input type="range" min="0" max="3" step="0.25" value={active.options.blur} oninput={(e) => updateNameEffect(active.id, "blur", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Glow <output>{active.options.glow}px</output></span><input type="range" min="2" max="14" value={active.options.glow} oninput={(e) => updateNameEffect(active.id, "glow", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "fire"}
                <label class="fx-option"><span>Flame height <output>{active.options.height}px</output></span><input type="range" min="1" max="10" value={active.options.height} oninput={(e) => updateNameEffect(active.id, "height", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Heat <output>{active.options.intensity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.intensity} oninput={(e) => updateNameEffect(active.id, "intensity", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Flicker <output>{active.options.speed}</output></span><input type="range" min="1" max="10" value={active.options.speed} oninput={(e) => updateNameEffect(active.id, "speed", e.currentTarget.valueAsNumber)} /></label>
              {:else if active.id === "extrude"}
                <label class="fx-option"><span>Depth <output>{active.options.depth}</output></span><input type="range" min="1" max="7" value={active.options.depth} oninput={(e) => updateNameEffect(active.id, "depth", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-option"><span>Strength <output>{active.options.opacity}%</output></span><input type="range" min="20" max="100" step="5" value={active.options.opacity} oninput={(e) => updateNameEffect(active.id, "opacity", e.currentTarget.valueAsNumber)} /></label>
                <label class="fx-color-option"><span>Depth colour</span><input type="color" value={active.options.color} oninput={(e) => updateNameEffect(active.id, "color", e.currentTarget.value)} /></label>
                <button type="button" class="ghost small fx-direction" onclick={() => updateNameEffect(active.id, "direction", active.options.direction === -1 ? 1 : -1)}>{active.options.direction === -1 ? "◀ left" : "▶ right"}</button>
              {/if}
                {#if fxMotionOff && active.enabled && movingNameEffect(active.id)}
                  <span class="muted small">Motion is off, so this effect is shown paused.</span>
                {/if}
                </div>
              {/if}
            </section>
          {/each}
        </div>
      {/if}
      {#if styleWarnings.length}
        <div class="fx-warnings" role="status">
          <strong>READABILITY CHECK</strong>
          {#each styleWarnings as warning}<span>△ {warning}</span>{/each}
        </div>
      {:else}
        <span class="fx-readable">✓ Readable at compact chat sizes</span>
      {/if}
    </div>
    <div class="field">
      <span class="muted">Colour</span>
      <div class="ns-swatches">
        <input type="color" value={pColor} aria-label="Custom name colour" oninput={(e) => setNameColor(e.currentTarget.value)} />
        {#each NAME_COLORS as c}
          <button
            type="button"
            class="ns-swatch"
            class:active={pColor === c}
            title={c}
            aria-label={`Name colour ${c}`}
            aria-pressed={pColor === c}
            style={`background:${c}`}
            onclick={() => setNameColor(c)}
          ></button>
        {/each}
      </div>
    </div>
    <div class="field text-fx-field">
      <div class="text-fx-field-head"><label class="muted" for="profile-bio">About you</label>{@render textEffectButton("bio", "Bio text effects")}</div>
      <textarea id="profile-bio" bind:this={profileBioEl} bind:value={pDescription} rows="3" maxlength="280" placeholder="A short bio shown on your profile card…" onselect={() => onTextEffectSelection("bio")}></textarea>
    </div>
    <div class="field message-frame-field frame-studio">
      <div class="message-frame-head">
        <div>
          <span class="name-studio-label">MESSAGE FRAME STUDIO</span>
          <strong>Frame &amp; arrival</strong>
        </div>
        <button type="button" class="ghost small" disabled={!pBubble} onclick={resetMessageStudio}>Reset all</button>
      </div>
      <span class="muted small">Style your posts without widening the message lane. Consecutive posts join into one continuous, translucent frame.</span>
      <span class="name-studio-label">SURFACE</span>
      <div class="bubble-presets" aria-label="Message frame preset">
        {#each BUBBLE_PRESETS as b}
          <button
            type="button"
            class="bubble-swatch"
            class:active={pFrame.surface === b.value}
            title={b.label}
            aria-pressed={pFrame.surface === b.value}
            onclick={() => updateFrame({ surface: b.value })}
          >
            <span class="bubble-swatch-demo" class:open={!b.value} style={b.value ? `--message-surface:${b.value};--message-opacity:${pFrame.opacity / 100}` : ""}>
              <i></i><i></i><i></i>
            </span>
            <span class="bubble-swatch-label"><b>{b.code}</b>{b.label}</span>
          </button>
        {/each}
        <button
          type="button"
          class="bubble-swatch"
          class:active={pFrame.surface === customBubble()}
          title="Custom gradient"
          aria-pressed={pFrame.surface === customBubble()}
          onclick={() => updateFrame({ surface: customBubble() })}
        >
          <span class="bubble-swatch-demo" style={`--message-surface:${customBubble()};--message-opacity:${pFrame.opacity / 100}`}><i></i><i></i><i></i></span>
          <span class="bubble-swatch-label"><b>USR</b>Custom mix</span>
        </button>
      </div>
      {#if pFrame.surface === customBubble()}
        <div class="grad-maker bubble-customizer">
          <label><span>A / SRC</span><input type="color" value={pBubA} aria-label="Frame gradient start colour" oninput={(e) => { pBubA = e.currentTarget.value; updateFrame({ surface: customBubble() }); }} /></label>
          <span class="bubble-gradient-link" aria-hidden="true"></span>
          <label><span>B / DST</span><input type="color" value={pBubB} aria-label="Frame gradient end colour" oninput={(e) => { pBubB = e.currentTarget.value; updateFrame({ surface: customBubble() }); }} /></label>
        </div>
        <span class="muted small">Keep both nodes dark enough for white terminal text; the signal rail adds structure, not contrast.</span>
      {/if}
      <span class="name-studio-label">CHASSIS</span>
      <div class="frame-preset-grid" aria-label="Message frame chassis">
        {#each FRAME_SHAPES as shape}
          <button
            type="button"
            class="frame-preset-tile"
            class:active={pFrame.shape === shape.id}
            title={shape.description}
            aria-pressed={pFrame.shape === shape.id}
            disabled={!pFrame.surface}
            onclick={() => updateFrame({ shape: shape.id })}
          >
            <span
              class="frame-preset-demo frame-{shape.id}"
              style={`--message-surface:${pFrame.surface || "#3a3f4b"};--message-opacity:${pFrame.opacity / 100};--message-edge:${pFrame.edge}%`}
              aria-hidden="true"
            ><i></i><i></i></span>
            <span class="bubble-swatch-label"><b>{shape.code}</b>{shape.label}</span>
          </button>
        {/each}
      </div>
      <div class="frame-control-grid">
        <label class="frame-control">
          <span><b>Frame opacity</b><output>{pFrame.opacity}%</output></span>
          <input type="range" min="20" max="90" step="1" value={pFrame.opacity} disabled={!pFrame.surface} oninput={(e) => updateFrame({ opacity: e.currentTarget.valueAsNumber })} />
        </label>
        <label class="frame-control">
          <span><b>Signal edge</b><output>{pFrame.edge}%</output></span>
          <input type="range" min="0" max="100" step="1" value={pFrame.edge} disabled={!pFrame.surface} oninput={(e) => updateFrame({ edge: e.currentTarget.valueAsNumber })} />
        </label>
      </div>
      <div class="effect-field-head frame-layer-head">
        <div>
          <span class="name-studio-label">FRAME LAYER STUDIO</span>
          <strong>Ambient effects</strong>
        </div>
        <button
          type="button"
          class="ghost small"
          class:active={!pFrame.effects.some((layer) => layer.enabled)}
          disabled={!pFrame.effects.length}
          title="Turn every frame layer off without losing its settings"
          onclick={disableAllFrameEffects}
        >All off</button>
      </div>
      <div class="frame-preset-grid" aria-label="Ambient message frame effect catalog">
        {#each FRAME_EFFECTS as effect}
          {@const configured = pFrame.effects.some((layer) => layer.id === effect.id)}
          {@const demoLayer = pFrame.effects.find((layer) => layer.id === effect.id) ?? defaultMessageFrameLayer(effect.id)}
          <button
            type="button"
            class="frame-preset-tile"
            class:active={configured}
            class:effect-off={configured && !demoLayer.enabled}
            title={configured ? `${effect.label}: show its saved settings` : `Add ${effect.label}`}
            aria-pressed={configured && demoLayer.enabled}
            disabled={!pFrame.surface}
            onclick={() => selectFrameEffect(effect.id)}
          >
            <span
              class="frame-preset-demo frame-{pFrame.shape}"
              style={`--message-surface:${pFrame.surface || "#3a3f4b"};--message-opacity:${pFrame.opacity / 100};--message-edge:${pFrame.edge}%`}
              aria-hidden="true"
            >
              <span class="message-frame-fx"><i class="frame-fx-layer frame-fx-{effect.id}" class:reverse={effect.id !== "scan" && demoLayer.options.direction < 0} style={messageFrameLayerStyle(demoLayer)}></i></span>
              <i></i><i></i>
            </span>
            <span class="bubble-swatch-label"><b>{effect.code}</b>{effect.label}</span>
          </button>
        {/each}
      </div>
      <span class="muted small">Add and combine layers, then tune, reorder, or temporarily disable them below. Later layers render above earlier ones.</span>
      {#if pFrame.effects.length}
        <div class="fx-settings-list frame-layer-list" aria-label="Applied frame layer options">
          {#each pFrame.effects as layer, li (layer.id)}
            {@const definition = FRAME_EFFECTS.find((effect) => effect.id === layer.id)}
            <section class="fx-settings" class:effect-off={!layer.enabled} aria-label={`${definition?.label ?? layer.id} frame layer settings`}>
              <div class="fx-settings-head">
                <div class="fx-settings-label">
                  <span class="frame-layer-index" aria-hidden="true">L{li + 1}</span>
                  <button type="button" class="fx-settings-title" aria-expanded={!collapsedFrameEffects[layer.id]} onclick={() => (collapsedFrameEffects[layer.id] = !collapsedFrameEffects[layer.id])}>
                    <span class="fx-chevron" aria-hidden="true">{collapsedFrameEffects[layer.id] ? "▸" : "▾"}</span>
                    <span><strong>{definition?.label ?? layer.id}</strong><span class="muted small">{definition?.description ?? ""}</span></span>
                  </button>
                </div>
                <div class="fx-settings-actions">
                  <label class="fx-enabled"><input type="checkbox" checked={layer.enabled} onchange={(e) => setFrameEffectEnabled(layer.id, e.currentTarget.checked)} /><span>{layer.enabled ? "On" : "Off"}</span></label>
                  <button type="button" class="ghost fx-order" disabled={li === 0} aria-label={`Move ${definition?.label ?? layer.id} down a visual layer`} onclick={() => moveFrameEffect(layer.id, -1)}>↑</button>
                  <button type="button" class="ghost fx-order" disabled={li === pFrame.effects.length - 1} aria-label={`Move ${definition?.label ?? layer.id} up a visual layer`} onclick={() => moveFrameEffect(layer.id, 1)}>↓</button>
                  <button type="button" class="ghost small" onclick={() => resetFrameEffect(layer.id)}>Reset</button>
                  <button type="button" class="ghost small" onclick={() => removeFrameEffect(layer.id)}>Remove</button>
                </div>
              </div>
              {#if !collapsedFrameEffects[layer.id]}
                <div class="fx-settings-body">
                  <div class="fx-option-grid">
                    {#if layer.id !== "scan"}<label class="fx-option"><span>Speed <output>{layer.options.speed}</output></span><input type="range" min="1" max="10" value={layer.options.speed} oninput={(e) => updateFrameEffect(layer.id, "speed", e.currentTarget.valueAsNumber)} /></label>{/if}
                    <label class="fx-option"><span>Strength <output>{layer.options.intensity}%</output></span><input type="range" min="20" max="100" step="5" value={layer.options.intensity} oninput={(e) => updateFrameEffect(layer.id, "intensity", e.currentTarget.valueAsNumber)} /></label>
                  </div>
                  {#if layer.id === "scan"}
                    <label class="fx-option"><span>Beam width <output>{layer.options.amount}px</output></span><input type="range" min="1" max="8" value={layer.options.amount} oninput={(e) => updateFrameEffect(layer.id, "amount", e.currentTarget.valueAsNumber)} /></label>
                    <span class="muted small">Shares one top-to-bottom sweep across the visible message stack with every Scan-enabled frame.</span>
                  {:else if layer.id === "pulse"}
                    <label class="fx-option"><span>Breathing depth <output>{layer.options.amount}%</output></span><input type="range" min="10" max="80" step="5" value={layer.options.amount} oninput={(e) => updateFrameEffect(layer.id, "amount", e.currentTarget.valueAsNumber)} /></label>
                  {:else if layer.id === "trace"}
                    <label class="fx-option"><span>Trace length <output>{layer.options.amount}%</output></span><input type="range" min="10" max="70" step="5" value={layer.options.amount} oninput={(e) => updateFrameEffect(layer.id, "amount", e.currentTarget.valueAsNumber)} /></label>
                    <button type="button" class="ghost small fx-direction" onclick={() => updateFrameEffect(layer.id, "direction", layer.options.direction < 0 ? 1 : -1)}>{layer.options.direction < 0 ? "◀ right to left" : "▶ left to right"}</button>
                  {:else}
                    <label class="fx-option"><span>Refresh variance <output>{layer.options.amount}%</output></span><input type="range" min="5" max="80" step="5" value={layer.options.amount} oninput={(e) => updateFrameEffect(layer.id, "amount", e.currentTarget.valueAsNumber)} /></label>
                  {/if}
                </div>
              {/if}
            </section>
          {/each}
        </div>
      {/if}
      <div class="message-frame-head motion-studio-head">
        <div>
          <span class="name-studio-label">MESSAGE ARRIVAL STUDIO</span>
          <strong>New-message arrival</strong>
        </div>
        <span class="message-frame-kicker">PROFILE MOTION</span>
      </div>
      <div class="frame-motion-grid" aria-label="New message arrival animation">
        {#each FRAME_MOTIONS as motion}
          <button
            type="button"
            class="frame-motion-tile motion-demo-{motion.id}"
            class:active={pFrame.motion === motion.id}
            title={motion.description}
            aria-pressed={pFrame.motion === motion.id}
            onclick={() => updateFrame({ motion: motion.id })}
          >
            <span aria-hidden="true">{motion.glyph}</span>
            <b>{motion.label}</b>
          </button>
        {/each}
      </div>
      {#if pFrame.motion !== "none"}
        <div class="arrival-settings">
          <div class="fx-option-grid">
            <label class="fx-option"><span>Duration <output>{pFrame.arrival.duration}ms</output></span><input type="range" min="240" max="1200" step="20" value={pFrame.arrival.duration} oninput={(e) => updateFrameArrival({ duration: e.currentTarget.valueAsNumber })} /></label>
            <label class="fx-option"><span>{pFrame.motion === "pop" ? "Scale depth" : "Travel"} <output>{pFrame.arrival.distance}</output></span><input type="range" min="4" max="80" step="2" value={pFrame.arrival.distance} oninput={(e) => updateFrameArrival({ distance: e.currentTarget.valueAsNumber })} /></label>
            <label class="fx-option"><span>Starting visibility <output>{pFrame.arrival.fade}%</output></span><input type="range" min="0" max="80" step="5" value={pFrame.arrival.fade} oninput={(e) => updateFrameArrival({ fade: e.currentTarget.valueAsNumber })} /></label>
            <div class="arrival-direction">
              <span class="muted small">ENTRY VECTOR</span>
              <button type="button" class="ghost small" disabled={pFrame.motion === "pop"} onclick={() => updateFrameArrival({ direction: pFrame.arrival.direction < 0 ? 1 : -1 })}>
                {#if pFrame.motion === "fly"}{pFrame.arrival.direction < 0 ? "← from left" : "from right →"}
                {:else if pFrame.motion === "glide"}{pFrame.arrival.direction < 0 ? "↑ from above" : "from below ↓"}
                {:else if pFrame.motion === "drift"}{pFrame.arrival.direction < 0 ? "↖ drift left" : "drift right ↗"}
                {:else}centred{/if}
              </button>
            </div>
          </div>
          <div class="arrival-curve-picker" aria-label="Arrival easing">
            <span class="name-studio-label">RESPONSE CURVE</span>
            <div>
              {#each FRAME_EASINGS as easing}
                <button type="button" class:active={pFrame.arrival.easing === easing.id} title={easing.description} aria-pressed={pFrame.arrival.easing === easing.id} onclick={() => updateFrameArrival({ easing: easing.id })}>{easing.label}</button>
              {/each}
            </div>
          </div>
        </div>
      {/if}
      <span class="muted small">Chassis, layer stack, and arrival recipe travel with your profile. Viewers may flatten peer frames or disable arrivals locally in Settings - Appearance.</span>
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
    <button onclick={saveProfile}>Save profile</button>
  </div>
{/snippet}

{#snippet frameLayers(frame: MessageFrame, visible = true)}
  {#if visible && frame.surface && frame.effects.some((layer) => layer.enabled)}
    <span class="message-frame-fx" aria-hidden="true">
      {#each frame.effects as layer (layer.id)}
        {#if layer.enabled}
          <i class="frame-fx-layer frame-fx-{layer.id}" class:reverse={layer.id !== "scan" && layer.options.direction < 0} style={messageFrameLayerStyle(layer)}></i>
        {/if}
      {/each}
    </span>
  {/if}
{/snippet}

<!-- The settings live preview: the REAL message-log markup at miniature scale, fed by the
     profile DRAFT, so it can never drift from the log and every knob (density, text size,
     clock, flatten, message frame, name style) applies the moment you turn it. -->
{#snippet previewLog()}
  {@const pv = messageFrameStyle(pBubble)}
  {@const previewMotion = pFrame.motion}
  <ul
    class="messages stx-plog frame-motion-preview"
    class:preview-arrival={previewMotion !== "none"}
    class:arrival-glide={previewMotion === "glide"}
    class:arrival-fly={previewMotion === "fly"}
    class:arrival-pop={previewMotion === "pop"}
    class:arrival-drift={previewMotion === "drift"}
    style={messageFrameArrivalStyle(pBubble)}
    use:channelScan
  >
    <li class="frame-{pFrame.shape}" class:has-bubble={!!pv} class:frame-start={!!pv} style={pv}>
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
        {@render frameLayers(pFrame)}
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
    <li class="grouped frame-{pFrame.shape}" class:has-bubble={!!pv} class:frame-end={!!pv} style={pv}>
      <span class="t">{fmtTime(Date.now())}</span>
      <div class="m-body">{@render frameLayers(pFrame)}<span class="text">bringing biscuits too</span></div>
    </li>
  </ul>
{/snippet}

<!-- Shared by the standalone Profile surface and Settings → My Profile: both keep this full
     card-and-chat preview pinned to the editor's right instead of duplicating a smaller preview
     below a potentially long effect-options list. -->
{#snippet profilePreview()}
  <div class="name-preview-stack" class:name-preview-paused={namePreviewPaused}>
    <div class="stx-ph"><i></i>LIVE PREVIEW</div>
    <div class="name-preview-tools">
      {#each ["all", "profile", "chat", "member", "mention"] as mode}
        <button type="button" class:active={namePreviewMode === mode} onclick={() => (namePreviewMode = mode as typeof namePreviewMode)}>{mode}</button>
      {/each}
      <button type="button" class:active={namePreviewPaused} onclick={() => (namePreviewPaused = !namePreviewPaused)}>{namePreviewPaused ? "▶" : "Ⅱ"}</button>
    </div>
    {#if namePreviewMode === "all" || namePreviewMode === "profile"}
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
            {#if pDescription}<span class="stx-prof-desc" use:richClicks>{@html renderMessage(pDescription, "")}</span>{/if}
          </div>
        </div>
      </div>
    {/if}
    {#if namePreviewMode === "all" || namePreviewMode === "chat"}
      <div class="stx-pcard">
        <div class="stx-pcap frame-preview-cap">
          <span>IN CHAT</span>
          <button type="button" title="Replay the selected arrival" disabled={pFrame.motion === "none"} onclick={() => (framePreviewReplay += 1)}>REPLAY</button>
        </div>
        {#key `${pBubble}:${framePreviewReplay}`}
          {@render previewLog()}
        {/key}
      </div>
    {/if}
    {#if namePreviewMode === "all" || namePreviewMode === "member"}
      <div class="stx-pcard">
        <div class="stx-pcap">MEMBER LIST · COMPACT</div>
        <div class="stx-member-preview">
          <span class="presence online">●</span>
          {#if pAvatar}<img class="avatar" src={imgSrc(pAvatar)} alt="" />{:else}<span class="avatar fallback" style={`background:${pColor}`}>{(pName || displayName).slice(0, 1).toUpperCase()}</span>{/if}
          <span class="stx-member-name">{@render styledName(pName || displayName, pColor, pFont, pEffect)}</span>
          <span class="you-badge">you</span>
        </div>
      </div>
    {/if}
    {#if namePreviewMode === "all" || namePreviewMode === "mention"}
      <div class="stx-pcard">
        <div class="stx-pcap">MENTION / NOTIFICATION</div>
        <div class="stx-mention-preview">
          <span>{@render styledName(pName || displayName, pColor, pFont, pEffect)}</span>
          <span class="muted small">mentioned you in #lounge</span>
        </div>
      </div>
    {/if}
    <p class="muted small stx-pnote">Draft preview · {namePreviewPaused ? "animations paused" : "animations live"}</p>
  </div>
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

{#snippet icoShield()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M12 2.8 20 6v5.6c0 5-3.2 8.2-8 9.7-4.8-1.5-8-4.7-8-9.7V6z" />
    <path d="m8.5 12 2.2 2.2 4.8-5" />
  </svg>
{/snippet}

{#snippet icoStorage()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <ellipse cx="12" cy="5.5" rx="8" ry="3" /><path d="M4 5.5v6c0 1.65 3.58 3 8 3s8-1.35 8-3v-6" /><path d="M4 11.5v6c0 1.65 3.58 3 8 3s8-1.35 8-3v-6" /><path d="m9 17.2 1.8 1.8 4-4" />
  </svg>
{/snippet}

{#snippet icoConnectivity()}
  <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M4.2 9.4a11 11 0 0 1 15.6 0M7.2 12.5a6.8 6.8 0 0 1 9.6 0M10.2 15.6a2.6 2.6 0 0 1 3.6 0" /><circle cx="12" cy="19" r="1" fill="currentColor" stroke="none" />
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
  {:else if b === "garden"}
    <svg class="sp-art sp-garden-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
      <!-- One continuous bioluminescent conservatory: glass ribs repeat across
           walls while each cardinal face gets its own little garden landmark. -->
      <path d="M8 100V38Q8 10 50 7Q92 10 92 38V100M8 38Q50 21 92 38M50 7V100" style="fill:none; stroke:color-mix(in oklab, var(--text-2) 20%, transparent); stroke-width:0.65" />
      <path d="M0 80Q18 68 36 78T70 76T100 78V100H0Z" style="fill:color-mix(in oklab, var(--ok) 11%, var(--bg-0))" />
      <path d="M0 85Q24 75 48 84T100 82" style="fill:none; stroke:color-mix(in oklab, var(--accent) 28%, transparent); stroke-width:0.5" />
      {#if fy === 0}
        <ellipse cx="50" cy="76" rx="18" ry="6" style="fill:color-mix(in oklab, var(--accent) 18%, var(--bg-0)); stroke:color-mix(in oklab, var(--accent) 52%, transparent); stroke-width:0.45" />
        <path d="M50 75Q43 59 49 45Q55 57 50 75M48 58Q39 51 36 42Q46 43 50 51M51 62Q60 55 65 46Q55 47 50 55" style="fill:color-mix(in oklab, var(--ok) 38%, var(--panel)); stroke:color-mix(in oklab, var(--ok) 58%, transparent); stroke-width:0.35" />
        <circle cx="49" cy="43" r="2.8" style="fill:color-mix(in oklab, var(--accent) 78%, white); opacity:0.72" />
      {:else if fy === 90}
        <path d="M22 80Q20 60 28 47M35 79Q37 58 32 39M72 80Q76 58 67 44M82 81Q79 65 86 53" style="fill:none; stroke:color-mix(in oklab, var(--ok) 62%, var(--panel)); stroke-width:1.1" />
        <path d="M27 58q-10-8-12 4q8 3 12-4M33 51q10-9 13 3q-8 5-13-3M68 55q-9-8-12 3q8 4 12-3M78 66q10-8 13 3q-8 4-13-3" style="fill:color-mix(in oklab, var(--ok) 34%, var(--panel))" />
      {:else if fy === 180}
        <rect x="28" y="49" width="44" height="29" rx="3" style="fill:color-mix(in oklab, var(--panel) 54%, transparent); stroke:color-mix(in oklab, var(--accent) 30%, var(--border)); stroke-width:0.5" />
        <path d="M31 75Q39 56 46 72Q54 48 61 70Q67 58 70 75" style="fill:color-mix(in oklab, var(--ok) 24%, var(--bg-elev)); stroke:color-mix(in oklab, var(--ok) 58%, transparent); stroke-width:0.4" />
        <circle cx="46" cy="65" r="1.6" style="fill:var(--accent); opacity:0.75" /><circle cx="61" cy="64" r="1.3" style="fill:var(--text-2); opacity:0.68" />
      {:else}
        <path d="M18 22Q26 38 18 55M42 12Q48 31 40 48M69 16Q75 34 68 51M88 25Q81 39 86 56" style="fill:none; stroke:color-mix(in oklab, var(--ok) 48%, var(--panel)); stroke-width:0.8" />
        <path d="M17 35q-8-6-9 3q6 3 9-3M41 26q9-7 11 3q-7 3-11-3M69 31q-8-7-10 3q7 3 10-3M86 42q8-6 10 3q-7 3-10-3" style="fill:color-mix(in oklab, var(--ok) 32%, var(--panel))" />
      {/if}
      {#each [[18, 30], [30, 20], [57, 29], [77, 37], [88, 19], [40, 39], [64, 17]] as [cx, cy], i}
        <circle class="sp-firefly" cx={cx} cy={cy} r={i % 2 ? 0.65 : 0.45} style={`--fly-delay:${-i * 0.43}s; fill:${i % 3 ? "var(--accent)" : "var(--ok)"}`} />
      {/each}
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

<!-- Dock slot: a frame with one edge weighted, so the glyph reads as "which end it sits at".
     The CSS flips it vertically when the dock is already at the top. -->
{#snippet icoDock()}
  <svg class="ico ico-dock" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="4" y="4" width="16" height="16" rx="2.5" />
    <path d="M4 15.5h16" fill="none" />
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
  <div class="juke-dock" class:folded={!jukeOpen}>
    <div class="juke-head">
      <span class="juke-head-ico">{@render icoNote()}</span>
      <span class="stage-label">JUKEBOX</span>
      <!-- One chip, in the order that matters: bytes in flight beats a dead DJ beats "we agree".
           Reading a held file off this disk and pulling one off a peer feel completely different
           to wait through, so they are named differently rather than both being "FETCHING". -->
      {#if jukeFetch}
        <span
          class="juke-chip info"
          title={jukeFetch.source === "network"
            ? `Pulling this track off ${jukeFetch.provider ? callNameOf(jukeFetch.provider) : "a peer"}`
            : "Reading this track from your vault"}
        >{jukeFetch.source === "network" ? "PULLING" : "LOADING"} {jukeFetch.percent}%</span>
      {:else if jukeBuffering}
        <span class="juke-chip warn" title="The deck ran out of data mid-track">BUFFERING</span>
      {:else if jukeNudging}
        <span class="juke-chip info" title="Easing playback back onto the DJ's clock">SYNCING</span>
      {:else if jukeStale}
        <span class="juke-chip warn" title="The DJ went quiet: the deck is frozen until someone presses">DECK STALE</span>
      {:else if jukeNow}
        <span class="juke-chip ok" title="You are where the DJ says the room is">SYNCED</span>
      {/if}
      {#if !jukeOpen && jukeUpNext.length}
        <span class="stage-label juke-qn" title={`${jukeUpNext.length} queued`}>Q{jukeUpNext.length}</span>
      {/if}
      <span class="stage-spacer"></span>
      {#if jukeNow}
        <span class="stage-label juke-dj" title="Whoever pressed last owns the deck">dj {jukeIsDj() ? "you" : callNameOf(jukeNow.dj)}</span>
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
      <button
        class="ghost stage-chev juke-chev"
        aria-expanded={jukeOpen}
        title={jukeOpen ? "Fold the jukebox to one line" : "Open the jukebox"}
        aria-label={jukeOpen ? "Fold the jukebox" : "Open the jukebox"}
        onclick={toggleJukeOpen}
      >{#if jukeOpen}{@render icoChevDown()}{:else}{@render icoChevUp()}{/if}</button>
    </div>

    {#if jukeNow && !jukeOpen}
      <!-- Folded: one line of the room's shared state, plus a hairline of progress. -->
      <div class="juke-min">
        <button
          class="juke-play mini"
          title={jukeNow.paused || jukeStale ? "Play for the room" : "Pause the room"}
          aria-label={jukeNow.paused || jukeStale ? "Play" : "Pause"}
          onclick={jukeToggle}
        >{#if jukeNow.paused || jukeStale}{@render icoPlay()}{:else}{@render icoPause()}{/if}</button>
        <span class="juke-min-nm" title={jukeNow.name}>{jukeNow.name}</span>
        <span class="juke-time">{jukeElapsed(jukePaint)} / {jukeDur > 0 ? jukeClock(jukeDur) : "?:??"}</span>
      </div>
      <div class="juke-bar slim"><i class="juke-bar-fill" style={`width:${jukePct(jukePaint)}%`}></i></div>
    {:else if jukeNow}
      {@const cur = jukeQueue.find((e) => e.id === jukeNow?.entry)}
      <div class="juke-now">
        <div class="juke-now-top">
          <span class="juke-now-nm" title={jukeNow.name}>{jukeNow.name}</span>
          <span class="juke-time">{jukeElapsed(jukePaint)} / {jukeDur > 0 ? jukeClock(jukeDur) : "?:??"}</span>
        </div>
        <!-- Two bars, never at once: how much of the track has arrived, then where the room is
             in it. Showing the play head over a track that has not arrived would be a lie. -->
        {#if jukeFetch}
          <div class="juke-bar load {jukeFetch.source}" title={jukeFetch.source === "network" ? "Coming off a peer" : "Coming off your vault"}>
            <i class="juke-bar-fill" style={`width:${jukeFetch.percent}%`}></i>
          </div>
        {:else}
          <div class="juke-bar"><i class="juke-bar-fill" style={`width:${jukePct(jukePaint)}%`}></i></div>
        {/if}
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
            <span class="stage-label juke-by" style={`color:${instColor(cur.author)}`} title={`Queued by ${callNameOf(cur.author)}`}>added by {callNameOf(cur.author)}</span>
          {/if}
        </div>
      </div>
    {:else}
      <div class="juke-idle">
        <span class="stage-label">deck idle</span>
        <span class="juke-idle-hint">queue something and press play</span>
      </div>
    {/if}

    {#if jukeOpen}
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
            <span class="juke-dot" style={`background:${instColor(e.author)}`} title={`Queued by ${callNameOf(e.author)}`}></span>
            <button class="juke-nm" title={gone ? `${e.name} is no longer in the share` : `Play ${e.name} for the room`} onclick={() => jukePlayEntry(e.id)}>{e.name}</button>
            {#if jukeFetch && jukeNow?.cid === e.cid}
              <span class="juke-chip info">{jukeFetch.percent}%</span>
            {/if}
            {#if mediaKind(e.name) === "video"}
              <span class="juke-kind" title="A video: it plays on the focus view">VID</span>
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
        <button type="button" role="tab" aria-selected={insertTab === "status"} class:active={insertTab === "status"} onclick={() => (insertTab = "status")}>Announcements</button>
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
      placeholder={insertTab === "files" ? "Find a file…" : insertTab === "status" ? "Find one of your announcements…" : insertTab === "events" ? "Find an event…" : "Find a wiki page…"}
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
            <button type="button" class="ip-item" title="Insert a link to this announcement" onclick={() => insertStatusRef(s)}>
              <span class="ip-ico">◈</span>
              <span class="ip-name">{msgSnippet(s.text, 70) || "(empty post)"}</span>
              <span class="ip-meta">{fmtTime(s.ts)}</span>
            </button>
            <span class="ip-mode">link</span>
          </div>
        {:else}
          <p class="ip-empty muted">{insertLoading ? "Loading…" : insertQuery.trim() ? "None of your announcements match that." : "You haven't posted an announcement on this server yet."}</p>
        {/each}
      {:else if insertTab === "events"}
        {#each insertEvents as ev (ev.id)}
          <div class="ip-row">
            <button type="button" class="ip-item" title="Insert a link to this event" onclick={() => insertEventRef(ev)}>
              <span class="ip-ico">⧗</span>
              <span class="ip-name">{msgSnippet(ev.title, 70)}</span>
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
          value={Math.max(wikiReviewDays, 0)}
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
  {:else if view === "moderation"}
    <h3><span>Moderation plane</span></h3>
    <p class="muted small">Signed warnings, evidence-backed cases and the server-wide event timeline.</p>
    <p class="muted small">Votes advise the owner; they never remove a member by themselves.</p>
  {:else if view === "storage"}
    <h3><span>Storage health</span></h3>
    <p class="muted small">Verifies seals, content addresses and file references. Repair only fetches from authenticated members.</p>
  {:else if view === "connectivity"}
    <h3><span>Connectivity</span></h3>
    <p class="muted small">Evidence from the last connection attempt, plus the live member count. “Started” is not presented as “reachable”.</p>
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
      <input type="file" multiple disabled={uploading} onchange={(e) => { uploadFile(e.currentTarget.files); e.currentTarget.value = ''; }} />
    </label>
    <form class="new-folder" onsubmit={(e) => { e.preventDefault(); const n = newFolder.trim(); if (n) { enterFolder(n); newFolder = ''; } }}>
      <input bind:value={newFolder} placeholder="＋ new folder…" />
    </form>
  {:else if view === "downloads"}
    <h3><span>Transfers</span></h3>
    <button class="ghost small ctx-action" disabled={finishedTransfers === 0} onclick={clearFinishedTransfers}>Clear finished</button>
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
    <h3><span>Announcements</span></h3>
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
    {#if locked || changingVaultSecret}
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
            onanimationstart={() => { if (head.kind !== "message") playNewsTicker(head.server); }}
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
  {#if eclipseCaution && activeServerId !== null && !locked && !changingVaultSecret}
    <div class="eclipse-banner" role="status">
      ⚠ You may be isolated from this server: few members are reachable. Verify a member out of band.
    </div>
  {/if}
  {#if locked || changingVaultSecret}
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
      {:else if changingVaultSecret}
        {@render brandMark(`vault secret · ${vaultChangeStep === "current" ? "verify" : vaultChangeStep === "new" ? "choose" : "confirm"}`)}
        <p class="muted">
          {vaultChangeStep === "current"
            ? "Enter your current vault secret. It is checked only when the replacement is ready, so a failed change cannot leave the vault half-updated."
            : vaultChangeStep === "new"
              ? "Choose a new passphrase, sigil, or tune. This rewraps the same random vault key; it does not expose or rewrite your server data."
              : "Enter the new secret again. After this succeeds, the old secret opens only backups that were exported before the change."}
        </p>
        {#if vaultChangeMismatch}<p class="error">That did not match the new secret. Cleared: try the confirmation again.</p>{/if}
        {#if vaultChangeError}<p class="error">{vaultChangeError}</p>{/if}
      {:else}
        {@render brandMark("")}
        <p class="muted">
          Unlock your servers: with a passphrase, a sigil, or a tune. All three seal the
          same vault; pick the one you'll actually remember.
        </p>
      {/if}
      <!-- Locked while confirming: the two performances are compared as encoded strings, so
           switching method between them could only ever mismatch. -->
      {@const tabsLocked = (inSetup && setupStep === "confirm") || vaultChangeStep === "confirm"}
      <div class="ul-tabs" role="tablist">
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "pass"} aria-selected={unlockMethod === "pass"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "pass"; }}>
          Passphrase <span class="ul-rec">recommended</span>
        </button>
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "sigil"} aria-selected={unlockMethod === "sigil"} onclick={() => { stopPlayback(); releaseAll(); unlockMethod = "sigil"; }}>Sigil</button>
        <button type="button" role="tab" disabled={tabsLocked} title={tabsLocked ? "Confirming: go back to change how you unlock" : ""} class:active={unlockMethod === "melody"} aria-selected={unlockMethod === "melody"} onclick={() => { unlockMethod = "melody"; void initMidi(); }}>Melody</button>
      </div>
      {#if unlockMethod === "pass"}
        <label class="field">
          <span class="muted">Passphrase</span>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            type="password"
            bind:value={passphrase}
            onkeydown={(e) => e.key === "Enter" && passphrase && (changingVaultSecret ? submitVaultSecretChange() : gateSubmit())}
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
      {:else if changingVaultSecret}
        <div class="setup-actions">
          <button class="ghost" disabled={vaultChangeBusy} onclick={cancelVaultSecretChange}>Cancel</button>
          <button disabled={vaultChangeBusy || !unlockSecret()} onclick={submitVaultSecretChange}>
            {vaultChangeBusy ? "Rewrapping vault…" : vaultChangeStep === "confirm" ? "Change vault secret" : "Continue"}
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
      <textarea
        class="invite-code"
        bind:value={joinInvite}
        oninput={() => {
          joinPreview = null;
          joinPreviewCode = "";
          joinSwitchboardConsent = false;
        }}
        rows="3"
        placeholder="paste invite here"
      ></textarea>
      {#if joinPreview?.switchboards}
        <section class="repair-card switchboard-consent">
          <div>
            <h3>Direct first; member fallback only with your permission</h3>
            <p class="muted small">
              This signed invite offers {joinPreview.switchboards} standing switchboard{joinPreview.switchboards === 1 ? "" : "s"}.
              Mewtual will try the named inviter directly first. If that fails, a switchboard can
              forward the admission handshake and remain your first encrypted group connection.
              That member learns your IP address and connection timing and may carry encrypted
              catch-up traffic. It already has ordinary member access, but helping grants no
              additional content access, and it cannot admit you itself.
            </p>
            <label class="check-row">
              <input type="checkbox" bind:checked={joinSwitchboardConsent} />
              Allow the signed member fallback after the direct attempt fails
            </label>
            <p class="muted small">Leave this off to try direct routes only. You can retry with fallback later.</p>
          </div>
        </section>
      {/if}
      <div class="pc-actions">
        <button onclick={join} disabled={busy || !joinInvite.trim()}>
          {joinPreview?.switchboards
            ? joinSwitchboardConsent
              ? "Join with fallback"
              : "Join directly"
            : "Join"}
        </button>
        <button class="ghost" disabled={scanOpen} onclick={() => scanQr((t) => {
          if (t) {
            joinInvite = t;
            joinPreview = null;
            joinPreviewCode = "";
            joinSwitchboardConsent = false;
          }
        })}>⛶ Scan invite QR</button>
      </div>
      {#if joinReplyReady && !joinReplyExpired}
        <section class="repair-card reply-card">
          <div>
            <h3>The inviter needs to dial you back</h3>
            <p class="muted small">
              The invite's routes did not answer. Send this authenticated reply to the inviter,
              or to a member whose app confirms a current live route to the named inviter, within
              60 seconds and keep both apps open. It offers {joinReplyCandidateLabel(joinReplyReady.candidate_count)};
              it is not a
              relay and cannot cross symmetric NAT/CGNAT on its own.
            </p>
            <textarea class="invite-code" readonly rows="3" value={joinReplyReady.code}></textarea>
          </div>
          <button class="ghost small" onclick={() => copyText(joinReplyReady?.code ?? "")}>Copy reply</button>
        </section>
      {:else if joinReplyReady}
        <section class="repair-card reply-card">
          <div><h3>Connection reply expired</h3><p class="muted small">Start the join again to mint a fresh 60-second route. The expired code is no longer copyable.</p></div>
        </section>
      {/if}
      {#if servers.length && activeServerId !== null}
        <details class="conn-panel reply-apply">
          <summary>I received a connection reply</summary>
          <p class="muted small">
            Paste the reply from the person joining <b>{cur?.name ?? "the active server"}</b>.
            Their app must still be waiting. If this device did not issue the invite, it acts only
            as a handshake helper; the named inviter and normal MLS admission rules still decide.
          </p>
          <textarea class="invite-code" rows="3" bind:value={joinReplyInput} placeholder="paste mewtual-reply-v1 code"></textarea>
          <div class="pc-actions">
            <button class="ghost small" disabled={joinReplyApplying || !joinReplyInput.trim()} onclick={() => applyJoinReply(false)}>
              {joinReplyApplying ? "Dialling…" : "Dial joiner"}
            </button>
            {#if joinReplyNeedsReplace}
              <button class="ghost small danger-btn" disabled={joinReplyApplying} onclick={() => applyJoinReply(true)}>
                Confirm different joiner
              </button>
            {/if}
          </div>
        </details>
      {/if}
      <details open={syncIntent}>
        <summary>Link this device to another device you own</summary>
        <p class="muted small">
          Your other device stays the master: it will show a code and ask permission before
          this device gets anything. Once approved, open the grant here and join its servers;
          admission completes when each server's owner is online to serialize it safely.
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
      <details class="conn-panel">
        <summary>Connection check</summary>
        <p class="muted small">
          What this app knows about reaching, and being reached by, other people. Open it when a
          server you founded cannot be joined, or when an invite you pasted times out.
        </p>
        {#if connectivity && connectivity.action}
          {@const reach = reachabilitySummary(connectivity)}
          <p class="muted small">
            Last attempt: <b>{connectivity.action === "found" ? "founding a server" : "joining"}</b>
            {connectivity.subject ? ` (${connectivity.subject})` : ""} at {fmtLocal(connectivity.at)}.
          </p>
          <p class="muted small">Observed reachability: <b>{reach.verdict}</b>. {reach.detail}</p>
          {@render connDetail(connectivity)}
          <div class="pc-actions">
            <button class="ghost small" onclick={refreshConnectivity}>Refresh</button>
            <button class="ghost small" onclick={copyConnectivity}>{connCopied ? "Copied!" : "Copy report"}</button>
          </div>
        {:else}
          <p class="muted small">
            Nothing has been tried yet this session. Found or join a server and this fills in with
            the addresses used and what happened to each.
          </p>
        {/if}
        <p class="muted small">
          If the person who invited you says their end shows nothing at all, your app never
          reached them. If their end shows a refusal, ask them to open
          <b>Server settings &rarr; Join Log</b>: it records the reason, which is deliberately not
          sent back to you.
        </p>
        {#if debugLog}
          <p class="muted small">
            Deeper detail goes in a debug log ({debugLog.enabled ? "on" : "off"}), switched on in
            Settings &rarr; Diagnostics, where its folder is also shown.
          </p>
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
            title="Inbox: mentions, replies & server news"
            onclick={openInbox}
          >
            {@render icoInbox()}
            {#if inboxUnseenCount || newsUnseen}
              <span class="rail-badge">{inboxUnseenCount + (newsUnseen ? 1 : 0)}</span>
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
              <button class:active={inboxMode === "news"} onclick={() => { inboxMode = "news"; newsUnseen = false; loadNews(); }}>News</button>
            </div>
            <span class="muted small">
              {inboxMode === "mentions" ? "Mentions & replies, across every server & DM" : "Announcements & upcoming events, across your servers"}
            </span>
            <button class="ghost small inbox-refresh" onclick={() => (inboxMode === "mentions" ? loadInbox() : loadNews())} disabled={inboxMode === "mentions" ? inboxLoading : newsLoading}>↻ Refresh</button>
          </div>
          {#if inboxMode === "news"}
            {#if newsLoading && !newsItems.length}
              <p class="muted inbox-empty">Loading…</p>
            {:else}
              {#if newsUpcoming.length}
                <h3 class="ev-h"><span>Upcoming events</span></h3>
                <ul class="inbox-list" use:richClicks>
                  {#each newsUpcoming as n (n.server + ":" + n.kind + ":" + n.ts + n.text)}
                    <li class="inbox-item">
                      <button class="inbox-jump" onclick={(event) => { if (!(event.target as HTMLElement).closest("[data-text-fx='censor']:not(.revealed)")) jumpToNews(n); }}>
                        <div class="inbox-meta">
                          <span class="inbox-tag event-tag">⧗ event</span>
                          <span class="inbox-where">{n.serverName}</span>
                          <span class="inbox-time" title={new Date(n.ts).toLocaleString()}>{dayLabel(n.ts)}</span>
                        </div>
                        <div class="inbox-body"><span class="inbox-text">{@html renderMessage(n.text, "")}</span></div>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
              <h3 class="ev-h"><span>Recent announcements</span></h3>
              {#if !newsFeed.length}
                <p class="muted inbox-empty">No announcements yet: servers' Announcements surfaces feed this.</p>
              {:else}
                <ul class="inbox-list" use:richClicks>
                  {#each newsFeed as n (n.server + ":" + n.ts + ":" + n.author)}
                    <li class="inbox-item">
                      <button class="inbox-jump" onclick={(event) => { if (!(event.target as HTMLElement).closest("[data-text-fx='censor']:not(.revealed)")) jumpToNews(n); }}>
                        <div class="inbox-meta">
                          <span class="inbox-tag reply-tag">◇ announcement</span>
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
                      <span class="inbox-where">
                        {#if it.is_dm}
                          Direct message · @{it.server_name}
                        {:else}
                          {it.server_name} · #{inboxChannelName(it)}
                        {/if}
                      </span>
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
              <button class="ghost small" onclick={copyInvite}>{copied ? "Copied!" : "Copy code"}</button>
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
        <div class="sidebar-utility" aria-label="Server safety and operations">
          {#if canModerate}
            <button type="button" class="sidebar-mod" class:active={view === "moderation"} onclick={() => switchView("moderation")}>
              {@render icoShield()}<span>Moderation</span>
              {#if moderationCases.length}<b>{moderationCases.length}</b>{/if}
            </button>
          {/if}
          <div class="sidebar-ops">
            <button type="button" class:active={view === "storage"} title="Storage health & repair" aria-label="Storage health and repair" onclick={() => switchView("storage")}>{@render icoStorage()}<span>Storage</span></button>
            <button type="button" class:active={view === "connectivity"} title="Connectivity assistant" aria-label="Connectivity assistant" onclick={() => switchView("connectivity")}>{@render icoConnectivity()}<span>Connect</span></button>
          </div>
        </div>
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
              <span class="sb-ico">◇</span>announcements
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
              {#if activeTransfers}<span class="tab-count">{activeTransfers}</span>{/if}
            </button>
          </nav>
        {/if}
        {#if view === "chat"}
          <!-- Header: identity on the left, the channel's description filling the middle, every
               action on the right. The member count lives in the members column, not here. -->
          <h2 class="chan-head">
            {#if cur?.isDm}
              <span class="chan-title"><span class="ch-hash">@</span>{cur.name}</span>
            {:else}
              <span class="chan-title"><span class="ch-hash">#</span>{activeName()}</span>
            {/if}
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
                <button class="ghost small jump-unread" title="Jump to where you left off" onclick={() => void scrollToMatch(firstUnreadIdx)}>↑ {unreadCount} new</button>
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
          {#if !canModerate && !cur?.isDm && moderationCases.length}
            <section class="community-votes" aria-label="Community kick votes">
              {#each moderationCases as kick (kick.id)}
                {@const tally = voteTally(moderation.votes, kick.id)}
                {@const evidence = signedWarnings.filter((warning) => kick.evidence_ids.includes(warning.id))}
                <article>
                  <span class="community-vote-mark">?</span>
                  <div>
                    <strong>Community vote: remove {nameOf(kick.target)}?</strong>
                    <p>{kick.reason}</p>
                    {#if evidence.length}<details><summary>{evidence.length} warned post{evidence.length === 1 ? "" : "s"} attached as evidence</summary>{#each evidence as warning (warning.id)}<blockquote><b>{warning.reason}</b><span>{msgSnippet(warning.message_text, 180)}</span></blockquote>{/each}</details>{/if}
                    <span class="muted small">{tally.yes} yes · {tally.no} no · advisory only; the owner makes the final MLS removal decision</span>
                  </div>
                  <div class="community-vote-actions"><button class="ghost small" onclick={() => voteKick(kick.id, true)}>Vote yes</button><button class="ghost small" onclick={() => voteKick(kick.id, false)}>Vote no</button></div>
                </article>
              {/each}
            </section>
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
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <ul
            class="messages"
            class:drag-over={dragOver}
            bind:this={messagesEl}
            use:richClicks
            use:channelScan
            onscroll={onChatScroll}
            ondragover={(e) => { e.preventDefault(); dragOver = true; }}
            ondragleave={() => (dragOver = false)}
            ondrop={(e) => onComposerDrop("chat", e)}
          >
            {#if messageWindow.start > 0}
              <li class="message-window-edge">
                <button class="ghost small" type="button" onclick={revealOlderMessages}>
                  Load {Math.min(CHAT_WINDOW_STEP, messageWindow.start)} older messages
                </button>
              </li>
            {/if}
            {#each renderedMessages as m, visibleIndex (messageDomKey(m, messageWindow.start + visibleIndex))}
              {@const mi = messageWindow.start + visibleIndex}
              {@const newDay = visibleIndex === 0 || mi === 0 || !sameDay(messages[mi - 1].ts, m.ts)}
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
              {@const framePosition = CHAT_MESSAGE_FRAMES_ENABLED
                ? messageFramePosition(messages, mi, messageFrameBreaks)
                : "single"}
              {@const bubble = bubbleStyle(m.author)}
              {@const messageFrame = CHAT_MESSAGE_FRAMES_ENABLED
                ? parseMessageFrame(profileFor(m.author)?.bubble)
                : DEFAULT_MESSAGE_FRAME}
              {@const arrival = arrivalMotion(m.author, m.id)}
              {@const arrivalVars = arrivalStyle(m.author)}
              {@const tick = mi === lastOwnIdx ? deliveryTick(m) : null}
              {@const ident = identityOf(m.author)}
              {@const warning = warningFor(cur?.active ?? "", m.id)}
              <li
                class="frame-{messageFrame.shape}"
                data-mi={mi}
                class:own={m.author === myFp}
                class:grouped
                class:unread={isUnread(m)}
                class:pings-me={m.author !== myFp && mentionsMe(m.text)}
                class:has-bubble={!!bubble}
                class:frame-start={!!bubble && framePosition === "start"}
                class:frame-middle={!!bubble && framePosition === "middle"}
                class:frame-end={!!bubble && framePosition === "end"}
                class:message-arrival={arrival !== "none"}
                class:arrival-glide={arrival === "glide"}
                class:arrival-fly={arrival === "fly"}
                class:arrival-pop={arrival === "pop"}
                class:arrival-drift={arrival === "drift"}
                class:search-match={showSearch && searchMatchSet.has(mi)}
                class:search-current={showSearch && searchCur?.ch === cur?.active && searchCur?.idx === mi}
                class:flash={!!m.id && m.id === flashId}
                style={[bubble, arrivalVars].filter(Boolean).join(";")}
                use:contextMenu={() => messageMenu(m)}
                use:resolveChatRow={m}
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
                {@render frameLayers(messageFrame, !!bubble)}
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
                      bind:this={editMessageEl}
                      bind:value={editDraft}
                      rows="2"
                      onselect={() => onTextEffectSelection("chat-edit")}
                      onkeydown={(e) => { if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); saveEdit(m); } else if (e.key === "Escape") { e.preventDefault(); cancelEdit(); } }}
                    ></textarea>
                    <div class="msg-edit-actions">
                      {@render textEffectButton("chat-edit", "Edited message text effects")}
                      <button class="ghost small" onclick={() => saveEdit(m)}>Save</button>
                      <button class="ghost small" onclick={cancelEdit}>Cancel</button>
                      <span class="muted small">Enter to save · Esc to cancel</span>
                    </div>
                  </div>
                {:else if warning && !expandedWarnings.has(warning.id)}
                  <button type="button" class="warned-collapse" onclick={() => toggleWarning(warning.id)}>
                    <span class="warned-mark">!</span>
                    <span><b>A moderator warned this post</b><small>{warning.reason} · expand for context</small></span>
                  </button>
                {:else}
                  {#if warning}
                    <button type="button" class="warning-banner" onclick={() => toggleWarning(warning.id)} title="Collapse this warned post"><b>Warning:</b> {warning.reason}<span>collapse</span></button>
                  {/if}
                  <span class="text">{@html renderedMessage(m)}{#if m.edited}<span class="edited-tag muted" title={"edited " + new Date(m.edited).toLocaleString()}> (edited)</span>{/if}</span>
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
              <li class="muted">{groupLoading ? "Loading messages…" : "No messages yet: say hello."}</li>
            {/each}
            {#if messageWindow.end < messages.length}
              <li class="message-window-edge">
                <button class="ghost small" type="button" onclick={revealNewerMessages}>
                  Load {Math.min(CHAT_WINDOW_STEP, messages.length - messageWindow.end)} newer messages
                </button>
              </li>
            {/if}
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
                title="Link or embed a file, one of your announcements, or a wiki page"
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
                onselect={() => onTextEffectSelection("chat")}
                onkeydown={onComposerKeydown}
                onblur={() => queueMicrotask(() => (mentionQuery = null))}
              ></textarea>
              <span class="c-hint">enter to send · shift+enter newline</span>
              {@render textEffectButton("chat", "Message text effects")}
              <button type="button" class="attach" title="Emoji" onclick={() => (showEmoji = !showEmoji)}>{@render icoCat()}</button>
              <button type="submit" disabled={uploading || sending}>Send</button>
            </form>
          </div>
        {:else if view === "moderation" && canModerate}
          <div class="moderation-pane tab-pane">
            <header class="ops-head">
              <div>
                <span class="stx-crumb">SERVER // SAFETY // MODERATION</span>
                <h2>Moderation plane</h2>
                <p class="muted small">A public, signed history. Warnings preserve what was seen; kick votes advise the owner and never bypass MLS authorization.</p>
              </div>
              <button class="ghost small" disabled={moderationLoading} onclick={() => refreshModeration(true)}>{moderationLoading ? "Loading…" : "Refresh"}</button>
            </header>
            {#if canModerate}
              <section class="mod-batch">
                <div><strong>{selectedModerationMessages().length} message{selectedModerationMessages().length === 1 ? "" : "s"} selected</strong><span class="muted small">Click a box; Shift-click extends the range.</span></div>
                <input bind:value={moderationReason} maxlength="2048" placeholder="Public warning reason…" />
                <button disabled={!selectedModerationMessages().length || !moderationReason.trim()} onclick={warnModerationSelection}>Warn &amp; collapse</button>
                <button class="danger-btn" disabled={!selectedModerationMessages().length} onclick={deleteModerationSelection}>{moderationDeleteArmed ? `Confirm delete ${selectedModerationMessages().length}` : "Delete selected"}</button>
              </section>
            {/if}
            <section class="mod-visual">
              <header>
                <div>
                  <h3>ACTIVITY FLOW <span>{filteredModerationTimeline.length}</span></h3>
                  <p class="muted small">Each rail is a member. Lines connect a moderator action to the person it concerns; the detailed evidence remains in the scroll below.</p>
                </div>
                <label class="mod-user-filter">
                  <span>View user</span>
                  <select value={moderationUserFilter} onchange={(event) => setModerationUserFilter(event.currentTarget.value)}>
                    <option value="">All users</option>
                    {#each moderationUsers as identity (identity)}
                      <option value={identity}>{nameOf(identity)}</option>
                    {/each}
                  </select>
                </label>
              </header>
              <div class="mod-graph-scroll">
                <svg
                  class="mod-graph"
                  viewBox={`0 0 ${moderationGraph.width} ${moderationGraph.height}`}
                  style={`min-width:${Math.max(760, moderationGraph.nodes.length * 14)}px`}
                  role="img"
                  aria-label={`Moderation activity flow across ${moderationGraph.lanes.length} user lanes`}
                >
                  {#each moderationGraph.lanes as lane (lane.identity)}
                    <g class="mod-lane">
                      <text x="8" y={lane.y + 4}>{nameOf(lane.identity)}</text>
                      <line x1="150" y1={lane.y} x2={moderationGraph.width - 24} y2={lane.y}></line>
                    </g>
                  {/each}
                  {#each moderationGraph.nodes as node (node.key)}
                    {#if node.fromY !== node.y}
                      <path class="mod-branch" d={`M ${node.x - 10} ${node.fromY} C ${node.x - 3} ${node.fromY}, ${node.x - 3} ${node.y}, ${node.x} ${node.y}`}></path>
                    {/if}
                    <a href={`#mod-row-${node.key}`} aria-label={`${node.kind} at ${new Date(node.ts).toLocaleString()}`}>
                      <circle class="mod-node" class:message={node.kind === "message"} class:warning={node.kind === "warning"} class:case={node.kind === "kick_case"} class:resolution={node.kind === "case_resolution"} cx={node.x} cy={node.y} r={node.kind === "message" ? 4 : 6}></circle>
                      <title>{node.kind.replace("_", " ")} · {new Date(node.ts).toLocaleString()}</title>
                    </a>
                  {/each}
                </svg>
              </div>
              <div class="mod-legend muted small"><span><i class="message"></i>message</span><span><i class="warning"></i>warning</span><span><i class="case"></i>kick case</span><span><i class="resolution"></i>resolution</span></div>
            </section>
            <div class="mod-grid">
              <section class="mod-timeline">
                <h3>EVENT DETAIL <span>{filteredModerationTimeline.length}</span></h3>
                <ol>
                  {#each [...filteredModerationTimeline].reverse() as row (row.key)}
                    <li id={`mod-row-${row.key}`} class:mod-event={row.kind === "event"} class:selected={moderationSelected.has(row.key)}>
                      {#if row.kind === "message"}
                        <label class="mod-check" title="Select this message">
                          <input type="checkbox" checked={moderationSelected.has(row.key)} onclick={(e) => selectModerationRow(row.key, e.shiftKey)} />
                        </label>
                        <div class="mod-row-copy">
                          <div class="mod-row-meta"><b>#{row.message.channelName}</b><span>{nameOf(row.message.author)}</span><time>{new Date(row.ts).toLocaleString()}</time></div>
                          <p>{msgSnippet(row.message.text, 220)}</p>
                        </div>
                      {:else}
                        <span class="mod-glyph">{row.event.kind === "warning" ? "!" : row.event.kind === "kick_case" ? "?" : "✓"}</span>
                        <div class="mod-row-copy">
                          <div class="mod-row-meta"><b>{row.event.kind.replace("_", " ")}</b><span>by {nameOf(row.event.actor)}</span><time>{new Date(row.ts).toLocaleString()}</time></div>
                          <p>{row.event.reason || row.event.outcome}</p>
                          {#if !row.event.signature_valid}<span class="mod-proof bad">invalid signature · ignored</span>{:else if !row.event.authorized}<span class="mod-proof warn">signer lacks current authority · ignored</span>{:else}<span class="mod-proof ok">signed · attributed</span>{/if}
                        </div>
                      {/if}
                    </li>
                  {:else}
                    <li class="muted">No user events yet.</li>
                  {/each}
                </ol>
              </section>
              <aside class="mod-cases">
                <section>
                  <h3>OPEN KICK CASES <span>{moderationCases.length}</span></h3>
                  {#each moderationCases as kick (kick.id)}
                    {@const tally = voteTally(moderation.votes, kick.id)}
                    <article class="kick-case">
                      <div class="kick-case-head"><strong>{nameOf(kick.target)}</strong><span>{tally.yes} yes · {tally.no} no</span></div>
                      <p>{kick.reason}</p>
                      {#if kick.evidence_ids.length}<span class="muted small">{kick.evidence_ids.length} signed warning{kick.evidence_ids.length === 1 ? "" : "s"} attached</span>{/if}
                      <div class="kick-actions">
                        <button class="ghost small" onclick={() => voteKick(kick.id, true)}>Vote yes</button>
                        <button class="ghost small" onclick={() => voteKick(kick.id, false)}>Vote no</button>
                        {#if myRole === "owner"}
                          <button class="ghost small" onclick={() => resolveKick(kick.id, false)}>Dismiss</button>
                          <button class="danger-btn small" onclick={() => resolveKick(kick.id, true)}>Remove member</button>
                        {/if}
                      </div>
                    </article>
                  {:else}<p class="muted small">No case is awaiting an owner decision.</p>{/each}
                </section>
                {#if canModerate}
                  <section class="case-builder">
                    <h3>MAKE A CASE</h3>
                    <label><span>Member</span><select bind:value={caseTarget}><option value="">Choose…</option>{#each roster.filter((member) => !member.you) as member (member.fingerprint)}<option value={member.fingerprint}>{nameOf(member.fingerprint)}</option>{/each}</select></label>
                    <label><span>Public reason</span><textarea bind:value={caseReason} maxlength="2048" rows="3" placeholder="Explain why removal is being proposed…"></textarea></label>
                    <div class="evidence-list">
                      <span class="muted small">Evidence: signed warnings for this member</span>
                      {#each signedWarnings.filter((warning) => !caseTarget || warning.target === caseTarget) as warning (warning.id)}
                        <label><input type="checkbox" checked={caseEvidence.has(warning.id)} onchange={() => toggleCaseEvidence(warning.id)} /><span><b>{warning.reason}</b><small>{msgSnippet(warning.message_text, 100)}</small></span></label>
                      {:else}<p class="muted small">Warn a relevant message first, then attach it here.</p>{/each}
                    </div>
                    <button disabled={!caseTarget || !caseReason.trim()} onclick={openKickCase}>Publish case for a vote</button>
                  </section>
                {/if}
              </aside>
            </div>
          </div>
        {:else if view === "storage"}
          <div class="operations-pane tab-pane">
            <header class="ops-head">
              <div><span class="stx-crumb">SERVER // OPERATIONS // STORAGE</span><h2>Storage health &amp; repair</h2><p class="muted small">The first visit verifies every referenced chunk once and saves a session snapshot. Revisiting this page never starts another scan; Repair explicitly verifies again after recovery.</p></div>
              <span class="storage-snapshot">{storageChecking ? "Checking once…" : storageHealth ? `Saved · ${new Date(storageHealth.checked_at_ms).toLocaleTimeString()}` : "Waiting"}</span>
            </header>
            {#if storageHealth}
              <div class="health-score" class:healthy={!storageHealth.missing_chunks && !storageHealth.unreadable_chunks && !storageHealth.invalid_manifests}>
                <div class="health-ring"><b>{storageHealth.referenced_chunks ? Math.round(storageHealth.verified_chunks / storageHealth.referenced_chunks * 100) : 100}%</b><span>verified</span></div>
                <div><h3>{!storageHealth.missing_chunks && !storageHealth.unreadable_chunks && !storageHealth.invalid_manifests ? "Storage is healthy" : "Storage needs attention"}</h3><p>{storageHealth.verified_chunks} of {storageHealth.referenced_chunks} unique chunks verified · {fmtSize(storageHealth.verified_bytes)} encrypted content</p></div>
              </div>
              <div class="health-cards">
                <article><b>{fmtSize(storageHealth.verified_bytes)}</b><span>verified encrypted content</span></article><article><b>{storageHealth.unique_files}</b><span>unique files · {storageHealth.listed_files} listings</span></article><article><b>{storageHealth.missing_chunks}</b><span>missing chunks</span></article><article><b>{storageHealth.unreadable_chunks + storageHealth.invalid_manifests}</b><span>unreadable / invalid</span></article>
              </div>
              <div class="storage-breakdown">
                <section class="storage-categories">
                  <h3>SPACE BY TYPE <span>local estimate</span></h3>
                  <p class="muted small">The integrity total above is exact ciphertext bytes. These bars estimate locally held plaintext from chunk availability, while “logical” is the full deduplicated share size.</p>
                  {#each storageHealth.categories as category (category.name)}
                    <div class="storage-category-row">
                      <div><b>{category.name}</b><span>{category.files} file{category.files === 1 ? "" : "s"}{category.pinned_files ? ` · ${category.pinned_files} pinned` : ""}</span></div>
                      <div class="storage-bar"><i style={`width:${Math.max(2, category.local_estimated_bytes / storageCategoryMax * 100)}%`}></i></div>
                      <strong>{fmtSize(category.local_estimated_bytes)} <small>/ {fmtSize(category.logical_bytes)} logical</small></strong>
                    </div>
                  {:else}<p class="muted small">No shared file content is listed.</p>{/each}
                </section>
                <section class="storage-largest">
                  <h3>LARGEST FILES <span>top {storageHealth.largest_files.length}</span></h3>
                  <ol>
                    {#each storageHealth.largest_files as file (file.cid)}
                      <li>
                        <span class="storage-rank">{file.pinned ? "📌" : "·"}</span>
                        <div><b>{file.name}</b><small>{file.path || "root"} · {file.held}/{file.total} chunks local</small></div>
                        <strong>{fmtSize(file.local_estimated_bytes)}<small>{fmtSize(file.logical_bytes)} logical</small></strong>
                      </li>
                    {:else}<li class="muted small">No files to rank.</li>{/each}
                  </ol>
                </section>
              </div>
              <section class="storage-pinned">
                <span class="storage-pin-icon">📌</span>
                <div><h3>Pinned by the wiki</h3><p class="muted small">{storageHealth.pinned_files} unique file{storageHealth.pinned_files === 1 ? "" : "s"} · {fmtSize(storageHealth.pinned_local_estimated_bytes)} local estimate · {fmtSize(storageHealth.pinned_logical_bytes)} logical. Wiki embeds are retained regardless of their circulation date.</p></div>
              </section>
              <section class="repair-card">
                <div><h3>Authenticated repair</h3><p class="muted small">Re-fetches only missing or unreadable CIDs. A peer signs the response and the bytes must hash to the requested address before they replace a corrupt local record.</p><span class:ok-t={storageHealth.has_peers} class:fail-t={!storageHealth.has_peers}>{storageHealth.has_peers ? "A member was connected at check time" : "No member was connected at check time"}</span></div>
                <button disabled={storageRepairing || (!storageHealth.missing_chunks && !storageHealth.unreadable_chunks)} onclick={repairStorage}>{storageRepairing ? "Repairing…" : "Repair now"}</button>
              </section>
              {#if storageRepairNote}<p class="storage-note">{storageRepairNote}</p>{/if}
            {:else if storageChecking}<p class="muted">Reading and authenticating this server's local file records…</p>{:else}<p class="muted">The storage snapshot could not be loaded.</p>{/if}
          </div>
        {:else if view === "connectivity"}
          <div class="operations-pane tab-pane">
            <header class="ops-head"><div><span class="stx-crumb">SERVER // OPERATIONS // CONNECTIVITY</span><h2>Connectivity assistant</h2><p class="muted small">What this device can prove, what it merely attempted, and what to try next.</p></div><button class="ghost small" onclick={refreshConnectivity}>Refresh</button></header>
            {#if connectivity?.action}
              {@render connDetail(connectivity)}
              <div class="connect-actions"><button class="ghost small" onclick={copyConnectivity}>{connCopied ? "Copied" : "Copy diagnostic"}</button><button class="ghost small" onclick={() => openSettings("network")}>Open network settings</button><button class="ghost small" onclick={() => openSettings("diagnostics")}>Debug logging</button></div>
            {:else}<section class="repair-card"><div><h3>No attempt recorded this session</h3><p class="muted small">Founding or joining a server populates the detailed action log. Live peer presence above is still current.</p></div><button onclick={() => (showAdd = true)}>Add or join a server</button></section>{/if}
            <section class="connection-hosting" aria-labelledby="connection-hosting-title">
              <header>
                <div>
                  <h3 id="connection-hosting-title">GROUP HOSTING</h3>
                  <p><b>{Math.max(onlineCount - 1, 0)}</b> of <b>{Math.max(members - 1, 0)}</b> other members are connected now.</p>
                </div>
                <span class="hosting-state" data-ready={(switchboardStatus?.online.length ?? 0) > 0}>{switchboardStatus?.online.length ?? 0} SWITCHBOARD{switchboardStatus?.online.length === 1 ? "" : "S"} ONLINE</span>
              </header>
              <div class="hosting-depths">
                <article>
                  <span class="hosting-depth">ONE-TIME</span>
                  <div><b>Help with this join</b><p>An eligible member may approve one short reply window. It forwards only the inviter's admission handshake, then remains the joiner's first live group path.</p></div>
                  <span class="ok-t">available below</span>
                </article>
                <article>
                  <span class="hosting-depth">STANDING</span>
                  <div><b>Switchboard role</b><p>Fresh invites may advertise opted-in, connected members as reusable fallbacks after the inviter's direct route fails. No device is enrolled silently.</p></div>
                  <button class="ghost small" class:danger-btn={switchboardStatus?.offered} disabled={switchboardBusy || (!switchboardStatus?.offered && !switchboardStatus?.eligible)} title={switchboardStatus?.reason ?? "Reading this device's route eligibility"} onclick={toggleSwitchboard}>{switchboardBusy ? "Saving…" : switchboardStatus?.offered ? "Stop hosting" : "Offer to host from this device"}</button>
                </article>
              </div>
              {#if switchboardStatus?.online.length}
                <ul class="switchboard-list">
                  {#each switchboardStatus.online as host}
                    <li><span class="switchboard-badge">⇄ switchboard</span><b>{nameOf(host.fingerprint)}</b><span>{host.addresses} advertised candidate route{host.addresses === 1 ? "" : "s"}</span></li>
                  {/each}
                </ul>
              {/if}
              <p class="muted small hosting-disclosure">A switchboard is already a group member, so it gains no new content access, but carrying a connection uses its bandwidth and exposes the joiner's IP address and timing. Fresh assisted invites disclose this host's stable device fingerprint, transport identity, and advertised public or relay candidate addresses to their recipients. Admission still requires the invite's named inviter to sign the Welcome. Turning hosting off refuses new forwards immediately; an already-cached signed offer can remain visible in newly copied invites, and already-copied invites can retain the old address, only until that offer's short deadline.</p>
            </section>
            <section class="connection-fallback" aria-labelledby="connection-fallback-title">
              <div>
                <h3 id="connection-fallback-title">FALLBACK NODE</h3>
                <b>No Mewtual-operated fallback required</b>
                <p class="muted small">Mewtual starts without an owned service. A group can use direct routes and opted-in member switchboards, or configure its own relay/rendezvous address. The first two mutually unreachable people still need a public route, router mapping, or third party.</p>
              </div>
              <button class="ghost small" onclick={() => openSettings("network")}>Configure your own</button>
            </section>
            {#if activeServerId !== null}
              <section class="repair-card reply-card">
                <div>
                  <h3>One-time connection help</h3>
                  <p class="muted small">If a joiner sent back a <code>mewtual-reply-v1</code> code, paste it here. The assistant validates the original signed invite and dials only the reply's claimed public TCP/QUIC routes for the remaining 60-second window; Noise still requires the claimed peer identity. The named inviter admits directly. Another current member may help only when it is already connected to that inviter: it forwards the admission handshake and keeps the resulting member connection alive, but it never makes the admission decision.</p>
                  <textarea class="invite-code" rows="3" bind:value={joinReplyInput} placeholder="paste connection reply"></textarea>
                  {#if joinReplyNeedsReplace}<p class="muted small fail-t">This invite already has a different active joiner. Replace it only if you deliberately switched people.</p>{/if}
                </div>
                <div class="connect-actions">
                  <button class="ghost small" disabled={joinReplyApplying || !joinReplyInput.trim()} onclick={() => applyJoinReply(false)}>{joinReplyApplying ? "Dialling…" : "Dial joiner"}</button>
                  {#if joinReplyNeedsReplace}<button class="ghost small danger-btn" disabled={joinReplyApplying} onclick={() => applyJoinReply(true)}>Confirm replacement</button>{/if}
                </div>
              </section>
            {/if}
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
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <ul
            class="file-list tab-pane"
            class:drag-over={dragOver}
            ondragover={(e) => { e.preventDefault(); dragOver = true; }}
            ondragleave={() => (dragOver = false)}
            ondrop={onFilesDrop}
          >
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
          <h2>Announcements</h2>
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
            {@render textEffectButton("announcement", "Announcement text effects")}
            <textarea bind:this={announcementInputEl} bind:value={statusDraft} rows="1" onselect={() => onTextEffectSelection("announcement")} placeholder={uploading ? "Uploading…" : dragOver ? "Drop to embed…" : "Write an announcement…"}></textarea>
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
              <li class="muted">No announcements yet.</li>
            {/each}
          </ul>
        {:else if view === "wiki"}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="wiki"
            class:drag-over={dragOver}
            ondragover={(e) => { if (activeWikiPage) { e.preventDefault(); dragOver = true; } }}
            ondragleave={() => (dragOver = false)}
            ondrop={onWikiDrop}
          >
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
                      {#if mayEditWikiStructure(wikiReviewDays, canModerate)}
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
                    {@render textEffectButton("wiki", "Wiki text effects")}
                    <span class="wiki-tb-sep"></span>
                    <div class="wiki-ip-anchor">
                      <button
                        class="wiki-tb"
                        class:active={showInsert && insertTarget === "wiki"}
                        title="Link or embed a shared file, an announcement, another page, or an event"
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
                    <textarea bind:this={wikiTextarea} bind:value={wikiBody} oninput={() => (wikiDirty = true)} onselect={() => onTextEffectSelection("wiki")} onkeydown={onWikiEditKey} rows="18"
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
          <div class="profile-workspace">
            {@render profileEditor()}
            <aside class="stx-prev profile-surface-preview">
              {@render profilePreview()}
            </aside>
          </div>
        {:else if view === "events"}
          <h2>Events</h2>
          <div class="events-tab tab-pane">
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <form
              class="event-form"
              class:drag-over={dragOver}
              ondragover={(e) => { e.preventDefault(); dragOver = true; }}
              ondragleave={() => (dragOver = false)}
              ondrop={onEventDrop}
              onsubmit={(e) => { e.preventDefault(); createEvent(); }}
            >
              <div class="text-fx-input-row"><input bind:this={eventTitleEl} bind:value={evTitle} maxlength="120" placeholder="Event title" onselect={() => onTextEffectSelection("event-title")} />{@render textEffectButton("event-title", "Event title effects")}</div>
              <div class="event-times">
                <label><span class="muted small">Starts</span><input type="datetime-local" bind:value={evStart} /></label>
                <label><span class="muted small">Ends (optional)</span><input type="datetime-local" bind:value={evEnd} /></label>
              </div>
              <div class="text-fx-input-row"><textarea bind:this={eventBodyEl} bind:value={evBody} rows="2" maxlength="1024" placeholder="Details (optional)" onselect={() => onTextEffectSelection("event-body")}></textarea>{@render textEffectButton("event-body", "Event detail effects")}</div>
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
            <ul class="event-list" use:richClicks>
              {#each upcomingEvents as e (e.id)}
                <li class="event-row" class:flash={flashEventId === e.id}>
                  <div class="ev-when">{fmtEventWhen(e)}</div>
                  <div class="ev-main">
                    <div class="ev-title">{@html renderMessage(e.title, "")}</div>
                    {#if e.body}<div class="ev-body">{@html renderMessage(e.body, "")}</div>{/if}
                    <div class="ev-meta">by {@render nameTag(e.author)}</div>
                  </div>
                  {#if e.image && mediaUrls[e.image]}
                    <img class="ev-poster" src={mediaUrls[e.image]} alt={`Poster for ${plainSummary(e.title, 100)}`} />
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
              <ul class="event-list past" use:richClicks>
                {#each pastEvents as e (e.id)}
                  <li class="event-row">
                    <div class="ev-when">{fmtEventWhen(e)}</div>
                    <div class="ev-main">
                      <div class="ev-title">{@html renderMessage(e.title, "")}</div>
                      {#if e.body}<div class="ev-body">{@html renderMessage(e.body, "")}</div>{/if}
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
          <h2>Transfers</h2>
          <div class="downloads-tab tab-pane">
            <button class="transfer-health" onclick={() => switchView("storage")}>
              {@render icoStorage()}
              {#if storageHealth}
                <span><b>{storageHealth.missing_chunks || storageHealth.unreadable_chunks || storageHealth.invalid_manifests ? "Storage needs attention" : "Storage verified"}</b><small>{storageHealth.verified_chunks}/{storageHealth.referenced_chunks} chunks · {storageHealth.missing_chunks + storageHealth.unreadable_chunks} need repair</small></span>
              {:else}
                <span><b>Storage health</b><small>Verify seals, addresses and file keys</small></span>
              {/if}
              <i>Open →</i>
            </button>
            {#if transferList.length === 0}
              <p class="muted">No transfers yet. Shared files and downloads will appear here.</p>
            {:else}
              <div class="dl-toolbar">
                <span class="muted small">
                  {movingTransfers} active{#if waitingTransfers} · {waitingTransfers} waiting{/if} · {transferList.length} total
                </span>
                <button class="ghost small" disabled={finishedTransfers === 0} onclick={clearFinishedTransfers}>Clear finished</button>
              </div>
              <div class="transfer-legend" aria-label="Transfer piece colours">
                <span><i class="transfer-piece held"></i>ready</span>
                <span><i class="transfer-piece active"></i>transferring</span>
                <span><i class="transfer-piece pending"></i>pending</span>
                <span><i class="transfer-piece offline"></i>no connection</span>
              </div>
              <ul class="dl-list">
                {#each transferList as d (d.key)}
                  {@const pieces = transferPieceStates(d)}
                  <li class="dl-item">
                    <div class="dl-item-main">
                      <span class="dl-item-name">{#if d.direction === "upload"}↑{:else}↓{/if} {d.name}</span>
                      <span class="muted small">
                        {#if d.direction === "upload"}
                          {#if d.path}sharing to /{d.path}{:else}sharing to the group{/if}
                        {:else if d.provider}receiving from {nameOf(d.provider)}
                        {:else if !transferConnected(d)}waiting for a connected member
                        {:else}shared by {nameOf(d.author)}{/if}
                      </span>
                    </div>
                    <div class="dl-item-status {transferTone(d)}">
                      {#if transferIsActive(d)}<span class="transfer-pulse" aria-hidden="true"></span>{/if}
                      {transferStatus(d)}
                    </div>
                    {#if d.status === "failed" && d.error}
                      <span class="dl-item-error" title={d.error}>{d.error}</span>
                    {/if}
                    <div
                      class="transfer-piece-bar"
                      role="progressbar"
                      aria-label={`Transfer progress for ${d.name}`}
                      aria-valuemin="0"
                      aria-valuemax={d.total}
                      aria-valuenow={Math.min(d.done, d.total)}
                      title={transferHover(d)}
                    >
                      {#each pieces as piece}
                        <span class="transfer-piece {piece}" aria-hidden="true"></span>
                      {/each}
                    </div>
                    <span class="transfer-piece-count muted" title={transferHover(d)}>
                      {Math.min(d.done, d.total)}/{d.total} chunks
                    </span>
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
            <p class="muted small">{groupLoading ? "Loading members…" : rosterFilter.trim() ? "No matching members." : "No members to show."}</p>
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
      {#if transferList.length}
        <button
          class="seg sb-transfers"
          class:active={view === "downloads"}
          title="Open Transfers"
          onclick={() => switchView("downloads")}
        >
          {#if movingTransfers}
            <span class="transfer-live"><span class="transfer-pulse" aria-hidden="true"></span>⇅ {movingTransfers} active</span>
            {#if waitingTransfers}<span class="fail-t">· {waitingTransfers} waiting</span>{/if}
            {#if failedTransfers}<span class="fail-t">· ✗ {failedTransfers} failed</span>{/if}
          {:else if waitingTransfers}
            <span class="fail-t">○ {waitingTransfers} waiting for connection</span>
          {:else if failedTransfers}
            <span class="fail-t">✗ {failedTransfers} failed</span>
          {:else}
            <span class="ok-t">✓ {finishedTransfers} finished</span>
          {/if}
        </button>
      {/if}
      <span class="sb-spacer"></span>
      {#if myFp}<span class="seg" title="Your fingerprint on this server: click a member and compare out of band to verify">id {myFp.slice(0, 4)}·{myFp.slice(4, 8)}</span>{/if}
    </footer>

    <!--
      The voice stage. Two shapes of the same dock: a collapsed bar (glanceable) and the expanded
      stage (per-peer control). Mute is the one state that must never be ambiguous, so it gets a
      danger treatment plus an empty meter in both shapes.
    -->
    {#if inCall && !stageOpen && !focusOpen}
      <div class="call-bar" class:top={callDockTop} class:away={callElsewhere}>
        <span class="call-dot">{@render icoSpeaker()}</span>
        <span class="call-title">Voice</span>
        {@render callServerTag()}
        <span class="call-status">{callStatusText}</span>
        {#if roomPath}
          <span
            class="stage-path {roomPath}"
            title={roomPath === "relayed"
              ? "Relayed: end to end encrypted, but a relay carries it and sees who, when, and from which IP"
              : "Direct: peer to peer, nobody in the media path"}
          >{roomPath === "relayed" ? "RLY" : "P2P"}</span>
        {/if}
        <div class="call-avatars">
          {@render callAvatarTag(callSelfFp)}
          {#each callParticipants as fp}{@render callAvatarTag(fp)}{/each}
        </div>
        {@render micMeter()}
        {#if videoAnnounced}
          <button class="ghost focus-chip" title="Open the video focus view" onclick={openFocus}>{@render icoCam()}<span class="stage-label">FOCUS</span></button>
        {/if}
        {#if micOn}
          <button class="ghost small btn-ico stage-mute" class:muted={callMuted} title={callMuted ? "Unmute" : "Mute"} onclick={toggleMute}>{#if callMuted}{@render icoMicOff()} Muted{:else}{@render icoMic()} Mute{/if}</button>
        {:else}
          <button class="ghost small btn-ico stage-mute nomic" title="You are in this room without a microphone: the jukebox and the instruments still work. Click to turn a mic on." onclick={enableMic}>{@render icoMicOff()} No mic</button>
        {/if}
        <button class="call-hangup btn-ico" title="Leave voice" onclick={leaveVoice}>{@render icoHangup()} Leave</button>
        <button class="ghost stage-chev" title="Open the voice stage" aria-label="Open the voice stage" onclick={() => (stageOpen = true)}>{#if callDockTop}{@render icoChevDown()}{:else}{@render icoChevUp()}{/if}</button>
      </div>
    {/if}

    {#if inCall && stageOpen && !focusOpen}
      <div class="stage" class:top={callDockTop} class:away={callElsewhere}>
        <header class="stage-head">
          <span class="stage-live"></span>
          <span class="stage-label">VOICE</span>
          {@render callServerTag()}
          <span class="stage-spacer"></span>
          {#if roomPath}
            <button
              class="ghost stage-sec {roomPath}"
              aria-expanded={secInfoOpen}
              title={roomPath === "relayed"
                ? "At least one leg goes through a relay: click for what that means"
                : "Every leg is peer to peer"}
              onclick={() => (secInfoOpen = !secInfoOpen)}
            >E2E · {roomPath === "relayed" ? "RELAY" : "DIRECT"}</button>
          {/if}
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
          <button
            class="ghost stage-chev"
            title={callDockTop ? "Dock the voice bar at the bottom" : "Dock the voice bar at the top"}
            aria-label={callDockTop ? "Dock the voice bar at the bottom" : "Dock the voice bar at the top"}
            onclick={toggleDockSlot}
          >{@render icoDock()}</button>
          <button class="ghost stage-chev" title="Collapse to the call bar" aria-label="Collapse to the call bar" onclick={() => (stageOpen = false)}>{#if callDockTop}{@render icoChevUp()}{:else}{@render icoChevDown()}{/if}</button>
        </header>

        {#if secInfoOpen}
          <p class="stage-secinfo">
            Media is end to end encrypted on every path; no relay or server can hear or decrypt it.
            A relayed leg passes through a TURN relay, which can see who is talking to whom, when,
            and from which IP address. A direct leg has nobody in the path at all.
          </p>
        {/if}

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
                  <span class="stage-av" class:talking={speaking[fp]}>{@render catEars(fp)}{@render callAvatarTag(fp)}</span>
                  <span class="stage-nm">{@render callNameTag(fp)}</span>
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
                  {#if link === "est" && peerTransport[fp]}
                    <span
                      class="stage-path {peerTransport[fp]}"
                      title={peerTransport[fp] === "relayed"
                        ? "Relayed: end to end encrypted, but a relay carries it and sees who, when, and from which IP"
                        : "Direct: peer to peer, nobody in the media path"}
                    >{peerTransport[fp] === "relayed" ? "RLY" : "P2P"}</span>
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
            <span class="stage-av" class:talking={speaking.me}>{@render callAvatarTag(callSelfFp)}</span>
            <span class="stage-nm">{@render callNameTag(callSelfFp)}</span>
            <span class="stage-fp">{callSelfFp.slice(0, 4)}·{callSelfFp.slice(4, 8)}</span>
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
            <button class="ghost stage-act" class:on={myVideo === "cam"} aria-pressed={myVideo === "cam"} title={myVideo === "cam" ? "Stop your camera" : "Send your camera"} onclick={() => toggleVideo("cam")}>
              {@render icoCam()}
              <span class="stage-act-lbl">Cam</span>
            </button>
            <button class="ghost stage-act" class:on={myVideo === "screen"} aria-pressed={myVideo === "screen"} title={myVideo === "screen" ? "Stop sharing your screen" : "Share your screen"} onclick={() => toggleVideo("screen")}>
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
          <!-- Says what is actually true. The media key is derived but the frame layer that would
               use it is not implemented, so today's guarantee is DTLS-SRTP plus signalling that
               cannot be MITM'd, which is E2E but is not MLS-keyed media. -->
          {#if roomPath}
            <span
              class="focus-e2e {roomPath}"
              title={roomPath === "relayed"
                ? "End to end encrypted. At least one leg goes through a relay, which sees who and when, never the frames"
                : "End to end encrypted, peer to peer, nobody in the media path"}
            >E2E · {roomPath === "relayed" ? "RELAY" : "DIRECT"}</span>
          {:else}
            <span class="focus-e2e" title="Every frame rides an end-to-end encrypted peer link">E2E</span>
          {/if}
          <button class="ghost focus-exit" title="Leave focus: back to chat and the voice dock" aria-label="Leave focus" onclick={exitFocus}>{@render icoFocusOut()}</button>
        </header>

        <!-- The shared screen: a video the room is watching together gets the top band, and the
             faces drop to a filmstrip underneath rather than competing with it. -->
        {#if jukeNow && jukeKind === "video"}
          <div class="focus-deck" use:jukeHost>
            {#if jukeFetch}
              <div class="focus-deck-load">
                <span class="stage-label">
                  {jukeFetch.source === "network" ? "PULLING" : "LOADING"} {jukeFetch.percent}%
                </span>
                <div class="juke-bar load {jukeFetch.source}">
                  <i class="juke-bar-fill" style={`width:${jukeFetch.percent}%`}></i>
                </div>
              </div>
            {/if}
          </div>
        {/if}

        <div class="focus-grid" class:strip={jukeNow && jukeKind === "video"} style={`--focus-cols:${focusCols}`}>
          {#each focusTiles as fp (fp)}
            {@const me = fp === callSelfFp}
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
                <span class="focus-face">{@render catEars(me ? "me" : fp)}{@render callAvatarTag(fp)}</span>
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
          <button class="ghost focus-btn" class:on={myVideo === "cam"} aria-pressed={myVideo === "cam"} title={myVideo === "cam" ? "Stop your camera" : "Send your camera"} aria-label="Camera" onclick={() => toggleVideo("cam")}>
            {@render icoCam()}
          </button>
          <button class="ghost focus-btn" class:on={myVideo === "screen"} aria-pressed={myVideo === "screen"} title={myVideo === "screen" ? "Stop sharing your screen" : "Share your screen"} aria-label="Share your screen" onclick={() => toggleVideo("screen")}>
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
              <p class="juke-pick-empty">no audio or video in this server's share yet: drop a file in chat or the Files surface to share it</p>
            {:else}
              <ul class="juke-pick-list">
                {#each jukeAudioFiles as f (f.cid + "|" + f.path)}
                  {@const days = jukeExpiryDays(f.cid)}
                  <li class="juke-pick-row">
                    <span class="juke-ext" class:vid={mediaKind(f.name, f.mime) === "video"}>{jukeExt(f)}</span>
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

    {#if textEffectTarget && !showTextEffectCatalog && textEffectSelection.start !== textEffectSelection.end}
      {@const fxTarget = textEffectTarget}
      <div
        class="text-fx-selection-bar"
        style={`left:${textEffectBubble.x}px;top:${textEffectBubble.y}px`}
        role="toolbar"
        aria-label={`Apply a text effect to selected ${textEffectTargetLabel(fxTarget)}`}
      >
        {#each quickTextEffects as effect (effect.id)}
          <button
            type="button"
            class="text-fx-aa"
            aria-label={`Apply ${effect.label}`}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => applyTextEffect(effect.id, fxTarget)}
          >
            <span class="text-fx-aa-live" aria-hidden="true">{@html textEffectHtml(effect.id, "Aa")}</span>
            <span class="text-fx-speech" role="tooltip"><strong>{effect.label}</strong>{effect.description}{#if textEffectKeybinds[effect.id]}<kbd>{textEffectKeybinds[effect.id]}</kbd>{/if}</span>
          </button>
        {/each}
        <button type="button" class="text-fx-aa more" title="Every text effect and copyable code" onmousedown={(e) => e.preventDefault()} onclick={() => (showTextEffectCatalog = true)}>＋</button>
      </div>
    {/if}

    {#if textEffectTarget && showTextEffectCatalog}
      {@const fxTarget = textEffectTarget}
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) { showTextEffectCatalog = false; textEffectTarget = null; } }}>
        <div class="overlay-card text-fx-catalog" role="dialog" aria-modal="true" aria-labelledby="text-fx-title">
          <header class="overlay-head">
            <div><span class="name-studio-label">TEXT EFFECTS // {textEffectTargetLabel(fxTarget).toUpperCase()}</span><h2 id="text-fx-title">Make the selected words act</h2></div>
            <button class="ghost" title="Close" onclick={() => { showTextEffectCatalog = false; textEffectTarget = null; }}>✕</button>
          </header>
          <div class="text-fx-catalog-intro">
            <p>Select an Aa preview to wrap the current selection. Hover any preview for its plain-language behavior.</p>
            <code>[fx:cyber]copy/pasteable text[/fx]</code>
            <span class="muted small">Full mode animates and reacts to the pointer. Low is static and silent. Plain removes the decoration. Censor stays concealed until revealed.</span>
          </div>
          <input class="text-fx-search" bind:value={textEffectQuery} placeholder="Find shaky, trans pride, cyber, CRT…" aria-label="Search text effects" />
          <div class="text-fx-catalog-scroll">
            {#each TEXT_EFFECT_GROUPS as group}
              {@const effects = filteredTextEffects.filter((effect) => effect.group === group)}
              {#if effects.length}
                <section class="text-fx-group">
                  <h3>{group}</h3>
                  <div class="text-fx-grid">
                    {#each effects as effect (effect.id)}
                      <div class="text-fx-choice">
                        <button type="button" class="text-fx-choice-main" onclick={() => applyTextEffect(effect.id, fxTarget)}>
                          <span class="text-fx-choice-preview">{@html textEffectHtml(effect.id, effect.preview)}</span>
                          <span class="text-fx-choice-name"><strong>{effect.label}</strong><code>[fx:{effect.id}]</code></span>
                          <span class="text-fx-speech" role="tooltip"><strong>{effect.label}</strong>{effect.description}<span>{effect.animated ? "Full: animated · Low: static" : "Static in every motion mode"}</span></span>
                        </button>
                        <button type="button" class="text-fx-copy" title={`Copy ${effect.label} markup`} aria-label={`Copy ${effect.label} markup`} onclick={() => copyText(`[fx:${effect.id}]text[/fx]`)}>⧉</button>
                      </div>
                    {/each}
                  </div>
                </section>
              {/if}
            {/each}
          </div>
        </div>
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
              <p class="pc-desc" use:richClicks>{@html renderMessage(p.description, "")}</p>
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
        class:sp-lassoing={!!spaceLasso}
        class:sp-focusing={spaceEntryPhase === "focus"}
        class:sp-entering={spaceEntryPhase === "zoom"}
        data-backdrop={spaceBackdropEff}
        data-shape={spaceState.shape}
        data-shake={spaceState.hoverShake ? "on" : "off"}
        style={`--sp-server-size:${spaceState.serverSize}px; --sp-ambience:${spaceState.ambience / 100}; --sp-links:${spaceState.links / 100}; --sp-glow:${spaceState.glow / 100}; --sp-glow-pct:${Math.round(spaceState.glow * 0.75)}%; --sp-backdrop-blur:${spaceState.backdropBlur}px; --sp-backdrop-scale:${(1 + spaceState.backdropBlur * 0.004).toFixed(3)}`}
        tabindex="-1"
        aria-label="Server Space"
        bind:this={spaceRoot}
        bind:clientWidth={spaceVw}
        bind:clientHeight={spaceVh}
        onpointerdown={onSpaceDown}
        onpointermove={onSpaceMove}
        onpointerup={onSpaceUp}
        onpointercancel={onSpaceUp}
        onclickcapture={onSpaceClickCapture}
      >
        <div class="sp-world">
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
          <div class="sp-atmosphere" aria-hidden="true"></div>
          <svg class="sp-constellations" viewBox={`0 0 ${spaceVw} ${spaceVh}`} preserveAspectRatio="none" aria-hidden="true">
            {#each spaceZones as zone (zone.cluster.id)}
              <g class="sp-zone" style={`--sp-zone:${zone.cluster.color}`}>
                <ellipse cx={spaceVw / 2 + zone.x} cy={spaceVh / 2 + zone.y} rx={zone.rx} ry={zone.ry} />
              </g>
            {/each}
            {#each spaceLinks as link (link.key)}
              <line x1={spaceVw / 2 + link.x1} y1={spaceVh / 2 + link.y1} x2={spaceVw / 2 + link.x2} y2={spaceVh / 2 + link.y2} />
            {/each}
          </svg>

          <div class="sp-zone-actions" aria-label="Visible neighbourhoods">
            {#each spaceZones as zone (zone.cluster.id)}
              <button
                type="button"
                class:drop-target={spaceClusterDrop === zone.cluster.id}
                data-space-cluster={zone.cluster.id}
                style={`left:${spaceVw / 2 + zone.x}px; top:${spaceVh / 2 + zone.y - zone.ry + 13}px; --sp-zone:${zone.cluster.color}`}
                aria-label={`Focus ${zone.cluster.name} neighbourhood, ${zone.count} server${zone.count === 1 ? "" : "s"}`}
                title="Click to focus · drag servers here to assign"
                onpointerdown={(e) => { if (!spaceCarried) e.stopPropagation(); }}
                onclick={() => focusSpaceCluster(zone.cluster.id)}
              >
                <i></i><span>{zone.cluster.name}</span><b>{zone.count}</b>
              </button>
            {/each}
          </div>

          <svg class="sp-reticle" viewBox="0 0 40 40" aria-hidden="true">
            <circle cx="20" cy="20" r="9" />
            <path d="M20 4v8M20 28v8M4 20h8M28 20h8" />
          </svg>

          <div class="sp-icons">
            {#each spacePlaced as it (it.s.id)}
              {@const online = spaceOnlineCounts[it.s.id] ?? 1}
              {@const mentions = spaceMentionCounts[it.s.id] ?? 0}
              {@const voice = spaceVoiceCount(it.s.id)}
              {@const recent = it.s.dot || it.s.unread.length > 0 || (spaceActivityAt[it.s.id] ?? 0) > nowTick - 5 * 60_000}
              <button
                class="sp-srv"
                class:sp-unread={it.s.unread.length > 0 || it.s.dot}
                class:sp-recent={recent}
                class:sp-carried={it.carried}
                class:sp-enter-target={spaceEntering === it.s.id}
                class:sp-focus-target={spaceFocusedServer === it.s.id}
                class:sp-search-dim={!!spaceSearch && !spaceSearchMatches.some((s) => s.id === it.s.id)}
                style={`left:${spaceVw / 2 + it.x}px; top:${spaceVh / 2 + it.y}px; --sp-s:${it.scale.toFixed(3)}; --sp-delay:${-((it.s.id % 13) * 0.17).toFixed(2)}s;${spaceAccents[it.s.id] ? ` --sp-a:${spaceAccents[it.s.id]};` : ""}`}
                data-name={it.s.name}
                title={`${it.s.name} · ${online} online${mentions ? ` · ${mentions} mention${mentions === 1 ? "" : "s"}` : ""}${voice ? ` · ${voice} in voice` : ""}`}
                onpointerdown={(e) => onSpaceServerDown(e, it.s.id)}
                onclick={() => spaceIconClick(it.s.id)}
                use:contextMenu={() => spaceServerMenu(it.s)}
              >
                {#if serverIcons[it.s.id] && appearance.icons !== "flat"}
                  <img class="rail-img" src={imgSrc(serverIcons[it.s.id])} alt="" draggable="false" />
                {:else}
                  {monogram(it.s.name)}
                {/if}
                {#if it.s.unread.length}
                  <span class="rail-badge">{it.s.unread.length}</span>
                {/if}
                {#if online > 1}
                  <span class="sp-orbiters" aria-label={`${online} online`}>
                    {#each Array(Math.min(8, online - 1)) as _, i}<i style={`--sp-dot:${i}; --sp-dots:${Math.min(8, online - 1)}`}></i>{/each}
                    {#if online > 9}<b>+{online - 9}</b>{/if}
                  </span>
                {/if}
                {#if mentions}<span class="sp-mention-flare" title={`${mentions} unseen mention${mentions === 1 ? "" : "s"}`}>!</span>{/if}
                {#if voice}<span class="sp-voice-signal" title={`${voice} in voice`}><i></i><i></i><i></i></span>{/if}
              </button>
            {/each}
          </div>

          {#if spaceLasso}
            <svg class="sp-lasso" viewBox={`0 0 ${spaceVw} ${spaceVh}`} preserveAspectRatio="none" aria-hidden="true">
              <path d={spaceLassoPath(spaceLasso.points)} />
            </svg>
          {/if}
        </div>

        {#if spaceSearchOpen}
          <div class="sp-search" onpointerdown={(e) => e.stopPropagation()}>
            <span>⌕</span>
            <input bind:this={spaceSearchEl} bind:value={spaceSearch} placeholder="Find a server or neighbourhood" oninput={() => (spaceSearchIdx = 0)} onkeydown={onSpaceSearchKey} />
            <kbd>enter</kbd>
            {#if spaceSearch}
              <div class="sp-search-results">
                {#each spaceSearchMatches.slice(0, 8) as s, i (s.id)}
                  <button class:active={i === spaceSearchIdx} onclick={() => { spaceSearchIdx = i; pickSpaceSearch(true); }} onmouseenter={() => { spaceSearchIdx = i; pickSpaceSearch(false); }}>
                    <span>{s.name}</span>
                    <small>{spaceState.clusters.find((c) => c.id === spaceState.serverClusters[s.id])?.name ?? (spaceState.placements[s.id] ? "Unsorted" : "Unplaced")}</small>
                  </button>
                {/each}
                {#if !spaceSearchMatches.length}<p>No server or neighbourhood matches.</p>{/if}
              </div>
            {/if}
          </div>
        {/if}

        {#if spaceState.showMinimap}
          <div class="sp-minimap" aria-label="Space compass">
            <div class="sp-minimap-head"><span>COMPASS</span><b>{Math.round(spaceCam.yaw)}°</b></div>
            <div class="sp-minimap-field">
              <span class="sp-map-centre"></span>
              {#each spaceMapServers as item (item.s.id)}
                <button
                  class:active={spaceFocusedServer === item.s.id}
                  style={`left:${item.x}%; top:${Math.max(8, Math.min(92, item.y))}%`}
                  title={item.s.name}
                  onclick={() => focusSpaceServer(item.s.id)}
                ></button>
              {/each}
            </div>
            <div class="sp-cardinals"><span>−180</span><span>FRONT</span><span>+180</span></div>
          </div>
        {/if}

        {#if spaceState.clusters.length}
          <aside class="sp-neighbourhoods" aria-label="Neighbourhoods" onpointerdown={(e) => { if (!spaceCarried) e.stopPropagation(); }}>
            <div class="sp-neighbourhood-head">
              <span>NEIGHBOURHOODS</span>
              <small>drop servers here</small>
            </div>
            <div class="sp-neighbourhood-list">
              {#each spaceState.clusters as cluster (cluster.id)}
                {@const clusterIds = spaceClusterServerIds(cluster.id)}
                <div
                  class="sp-neighbourhood"
                  class:open={spaceClusterOpen === cluster.id}
                  class:drop-target={spaceClusterDrop === cluster.id}
                  data-space-cluster={cluster.id}
                  style={`--sp-zone:${cluster.color}`}
                >
                  <div class="sp-neighbourhood-row">
                    <button type="button" class="sp-neighbourhood-focus" onclick={() => focusSpaceCluster(cluster.id)} title={`Focus ${cluster.name}`}>
                      <i></i>
                      <span>{cluster.name}<small>{clusterIds.length} server{clusterIds.length === 1 ? "" : "s"}</small></span>
                    </button>
                    <button
                      type="button"
                      class="sp-neighbourhood-edit"
                      class:active={spaceClusterOpen === cluster.id}
                      aria-label={`${spaceClusterOpen === cluster.id ? "Close" : "Edit"} ${cluster.name}`}
                      title="Add or remove servers"
                      onclick={(e) => { e.stopPropagation(); spaceClusterOpen = spaceClusterOpen === cluster.id ? null : cluster.id; }}
                    >{spaceClusterOpen === cluster.id ? "−" : "+"}</button>
                  </div>
                  {#if spaceClusterOpen === cluster.id}
                    <div class="sp-neighbourhood-editor">
                      <p>Click to add or remove. You can also drag a server onto this neighbourhood.</p>
                      {#if railServers.length}
                        <div class="sp-neighbourhood-servers">
                          {#each railServers as server (server.id)}
                            {@const assigned = spaceState.serverClusters[server.id] === cluster.id}
                            <button
                              type="button"
                              class:assigned
                              onclick={(e) => { e.stopPropagation(); toggleSpaceClusterServer(server.id, cluster.id); }}
                              title={assigned ? `Remove ${server.name} from ${cluster.name}` : `Add ${server.name} to ${cluster.name}`}
                            >
                              <span>{assigned ? "✓" : "+"}</span>{server.name}
                            </button>
                          {/each}
                        </div>
                      {:else}
                        <p>No servers yet.</p>
                      {/if}
                    </div>
                  {/if}
                </div>
              {/each}
            </div>
            <button type="button" class="sp-neighbourhood-tidy" onclick={tidySpace}>Arrange neighbourhoods</button>
          </aside>
        {:else}
          <aside class="sp-neighbourhoods sp-neighbourhood-empty" aria-label="Neighbourhoods" onpointerdown={(e) => e.stopPropagation()}>
            <div class="sp-neighbourhood-head"><span>NEIGHBOURHOODS</span></div>
            <p>Group related servers into visible areas of your Space.</p>
            <button type="button" onclick={() => openSettings("space")}>Create a neighbourhood…</button>
          </aside>
        {/if}

        <div class="sp-hud">
          <div class="sp-hud-line">orbit · {spaceBackdropEff === "custom" ? "custom" : (SPACE_BACKDROP_TILES.find((b) => b.id === spaceBackdropEff)?.name ?? spaceBackdropEff)} · {spaceState.shape}</div>
          <div class="sp-hud-sub">yaw {Math.round(spaceCam.yaw)}° · pitch {Math.round(spaceCam.pitch)}°</div>
          {#if spaceCarried}
            <div class="sp-hud-carry">carrying {Object.keys(spaceCarried).length} · click to drop · esc cancels</div>
          {/if}
        </div>

        <div class="sp-keys">
          <span class="sp-key"><b>[drag]</b> look</span>
          <span class="sp-key"><b>[icon drag]</b> move</span>
          <span class="sp-key"><b>[hold + draw]</b> lasso</span>
          <button class="sp-key sp-key-btn" onclick={undoSpaceLayout} disabled={!spaceUndo.length}><b>[ctrl+z]</b> undo</button>
          <button class="sp-key sp-key-btn" onclick={tidySpace}><b>[tidy]</b> arrange</button>
          <button class="sp-key sp-key-btn" onclick={() => startSpaceSearch()}><b>[/]</b> find</button>
          <button class="sp-key sp-key-btn" class:active={spaceTray} onclick={() => (spaceTrayPinned = !spaceTrayPinned)}><b>[t]</b> tray</button>
          <button class="sp-key sp-key-btn" onclick={toggleSpace}><b>[esc]</b> exit</button>
        </div>

        {#if spaceTray}
          <div class="sp-tray" onpointerdown={(e) => e.stopPropagation()}>
            <div class="sp-tray-head">
              <span class="sp-micro">server tray</span>
              <span class="sp-chip">{railServers.length} servers · {spaceUnplaced.length} unplaced</span>
              <label class="sp-tray-size">
                <span>size {spaceState.serverSize}</span>
                <input type="range" min="32" max="88" step="2" value={spaceState.serverSize} aria-label="Server size" oninput={(e) => setSpaceServerSize(+e.currentTarget.value)} />
              </label>
              <span class="sp-tray-hint">drag into the room · tap places at the reticle</span>
            </div>
            {#if railServers.length}
              <div class="sp-tray-row">
                {#each railServers as s (s.id)}
                  <div class="sp-tray-slot">
                    <button class="sp-tray-item" class:sp-already-placed={!!spaceState.placements[s.id]} onpointerdown={(e) => onSpaceTrayServerDown(e, s.id)} onclick={() => placeFromTray(s.id)}>
                      <span class="sp-disc" style={spaceAccents[s.id] ? `--sp-a:${spaceAccents[s.id]}` : ""}>
                        {#if serverIcons[s.id] && appearance.icons !== "flat"}
                          <img class="rail-img" src={imgSrc(serverIcons[s.id])} alt="" draggable="false" />
                        {:else}
                          {monogram(s.name)}
                        {/if}
                        {#if spaceState.placements[s.id]}<span class="sp-placed-mark" aria-hidden="true">✓</span>{/if}
                      </span>
                      <span class="sp-tray-name">{s.name}</span>
                    </button>
                    {#if spaceState.clusters.length}
                      <select aria-label={`Neighbourhood for ${s.name}`} value={spaceState.serverClusters[s.id] ?? ""} onchange={(e) => assignSpaceCluster(s.id, e.currentTarget.value)}>
                        <option value="">Unsorted</option>
                        {#each spaceState.clusters as cluster (cluster.id)}<option value={cluster.id}>{cluster.name}</option>{/each}
                      </select>
                    {/if}
                  </div>
                {/each}
              </div>
            {:else}
              <p class="sp-tray-empty">Your servers will appear here.</p>
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
            {#each ["Help", "Account", "App", "Connection"] as cat (cat)}
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
            {#if settingsPage === "guide"}
              <div class="stx-crumb">SETTINGS // HELP // FEATURE GUIDE</div>
              <h1>Feature Guide</h1>
              <p class="muted small">Everything currently in this build, with the shortest route to it. Search by feature, action or location.</p>
              <label class="feature-search">
                <span class="muted small">Find a feature</span>
                <input bind:value={featureQuery} placeholder="Try wiki, invite, screen share, diagnostics…" />
              </label>
              {#each FEATURE_GUIDE_GROUPS as group (group)}
                {@const items = filteredFeatures.filter((item) => item.group === group)}
                {#if items.length}
                  <section class="set-section feature-section">
                    <h3>{group}</h3>
                    <div class="feature-list">
                      {#each items as item (item.title)}
                        <article class="feature-item">
                          <div class="feature-copy">
                            <strong>{item.title}</strong>
                            <p>{item.detail}</p>
                            <span class="feature-path">{item.where}</span>
                          </div>
                          <div class="feature-actions">
                            {#if item.shortcut}<kbd>{item.shortcut}</kbd>{/if}
                            {#if item.target}<button type="button" class="ghost small" onclick={() => item.target && openFeatureTarget(item.target)}>Open</button>{/if}
                          </div>
                        </article>
                      {/each}
                    </div>
                  </section>
                {/if}
              {/each}
              {#if !filteredFeatures.length}
                <section class="set-section">
                  <p class="muted small">No feature matches “{featureQuery}”. Try a broader word, or send the idea through Feedback if it is not here yet.</p>
                </section>
              {/if}
            {:else if settingsPage === "profile"}
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
                <h3>Linked devices</h3>
                <p class="muted small">
                  Link another device to your identity. The new device gets its own key: nothing
                  is copied: and nothing at all happens until you approve it here on this device.
                </p>
                <button class="ghost" onclick={() => (showLinkDevice = true)}>⛓ Link a new device…</button>
                <p class="muted small">Your linked devices are listed per server (each server sees its own identity): find them under a server's Settings → Devices.</p>
              </section>
              <!--
                MIDI hardware, deliberately on the same page as linked devices: from where the user
                stands both are "things I plugged into Mewtual". Nothing here touches identity, and
                the panel says so, because a keyboard listed next to key-bearing companions would
                otherwise read as something that can see your messages.
              -->
              <section class="set-section">
                <h3>MIDI controllers</h3>
                <p class="muted small">
                  A USB or Bluetooth music keyboard plays the melody unlock lock and the instrument
                  in a voice call. It is input hardware only: it carries no identity, joins no
                  server, and Mewtual never sends it anything. Notes reach other people only when
                  you are in a call, mixed the same way your voice is.
                </p>
                <div class="midi-status" data-level={midiStat.level}>
                  <span class="midi-dot" aria-hidden="true"></span>
                  <span class="midi-verdict">
                    <strong>{midiStat.title}</strong>
                    <small class="muted">{midiStat.detail}</small>
                  </span>
                  <button
                    type="button"
                    class="ghost small"
                    disabled={midiBusy || !midiSupported}
                    onclick={() => void initMidi(true)}
                  >{midiBusy ? "Scanning…" : midiRequested ? "Rescan" : "Turn on MIDI input"}</button>
                </div>
                {#if midiDevices.length}
                  <ul class="dev-panel midi-list">
                    {#each midiDevices as d (d.id)}
                      <li class:gone={!d.connected} title={d.id}>
                        <span class="midi-live" class:on={d.connected && d.routed} aria-hidden="true"></span>
                        <span class="midi-nm">{d.label}</span>
                        {#if d.maker}<span class="dev-tag">· {d.maker}</span>{/if}
                        <span class="stage-spacer"></span>
                        <span class="midi-state">{#if !d.connected}unplugged{:else if !d.routed}filtered out{:else if d.listening}routed{:else}opening{/if}</span>
                      </li>
                    {/each}
                  </ul>
                {/if}
                <label class="field midi-route">
                  <span class="muted small">
                    Input routing: leave this on every input unless one specific port is the one
                    carrying your keys.
                  </span>
                  <select value={midiInput} onchange={(e) => setMidiInput(e.currentTarget.value)}>
                    <option value="">Every connected input</option>
                    {#each midiDevices as d (d.id)}
                      <option value={d.id}>{d.label}{d.connected ? "" : " (not connected)"}</option>
                    {/each}
                    <!-- A pinned port saved by name, or one that has since vanished entirely: keep
                         it selectable so the picker never looks empty and can always be undone. -->
                    {#if midiInput && !midiDevices.some((d) => d.id === midiInput)}
                      <option value={midiInput}>{midiInput} (saved, not found)</option>
                    {/if}
                  </select>
                </label>
                <div class="midi-mon" role="log" aria-live="polite" aria-label="Incoming MIDI messages">
                  {#if midiMonitor.length}
                    {#each midiMonitor as line (line.seq)}
                      <span class="midi-mon-line" class:ignored={!line.routed}>
                        <b>{line.port}</b><i>{line.text}</i>{#if !line.routed}<em>filtered out</em>{/if}
                      </span>
                    {/each}
                  {:else if midiRealtime}
                    <span class="muted">The cable is alive ({midiRealtime} timing messages) but no notes have arrived. Play a key, and check nothing else is holding the port.</span>
                  {:else}
                    <span class="muted">Play a key on the controller: everything it sends shows up here, whether or not anything is listening for it.</span>
                  {/if}
                </div>
                <div class="midi-actions">
                  <button type="button" class="ghost small" onclick={releaseMidiNotes}>Release stuck notes</button>
                  <button type="button" class="ghost small" onclick={() => { midiMonitor = []; midiRealtime = 0; }}>Clear monitor</button>
                  {#if midiLastAt}<span class="muted small">last message {relTime(Math.max(0, nowTick - midiLastAt))} ago</span>{/if}
                </div>
              </section>
              <section class="set-section">
                <h3>Setting up a controller</h3>
                <ol class="midi-help">
                  {#each MIDI_SETUP_STEPS as step (step.title)}
                    <li><strong>{step.title}</strong><span class="muted small">{step.detail}</span></li>
                  {/each}
                </ol>
                <h4 class="midi-subhead">When it does not work</h4>
                <ul class="midi-help fixes">
                  {#each MIDI_FIXES as fix (fix.title)}
                    <li><strong>{fix.title}</strong><span class="muted small">{fix.detail}</span></li>
                  {/each}
                </ul>
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
                <h3>Change vault secret</h3>
                <p class="muted small">Authenticate with the current passphrase, sigil or tune, then choose and confirm any new method. Mewtual atomically rewraps the same random root key under a fresh Argon2 salt: server history and files never become plaintext and do not need a risky bulk rewrite.</p>
                <button onclick={beginVaultSecretChange}>Change vault secret…</button>
                <p class="muted small">This changes the live vault only. Every backup made earlier remains protected by—and openable with—the secret it had when exported.</p>
              </section>
            {:else if settingsPage === "backup"}
              <div class="stx-crumb">SETTINGS // ACCOUNT // BACKUP &amp; RECOVERY</div>
              <h1>Backup &amp; Recovery</h1>
              <section class="set-section backup-hero">
                <div class="backup-lock">{@render icoLock()}</div>
                <div>
                  <h3>Encrypted offline backup</h3>
                  <p class="muted small">Creates a coherent copy of the entire sealed vault in Downloads: identities, server history, files, drafts and read positions. The export remains encrypted by your current vault secret.</p>
                </div>
                <button disabled={backupBusy} onclick={createBackup}>{backupBusy ? "Creating…" : "Create backup"}</button>
              </section>
              {#if backupResult}
                <section class="set-section backup-result">
                  <span class="ok-t">✓ Backup created</span>
                  <strong>{backupResult.files} files · {fmtSize(backupResult.bytes)}</strong>
                  <code>{backupResult.path}</code>
                  {#if backupResult.warning}<p class="fail-t small">{backupResult.warning}</p>{/if}
                </section>
              {/if}
              <section class="set-section">
                <h3>Recovery contract</h3>
                <p class="muted small">Keep the vault secret somewhere separate: the backup cannot reset or bypass it. Automated restore is intentionally unavailable while the app is unlocked; safe restore needs a locked-screen, staged verification and rollback flow. For now, keep this folder intact as the recoverable source copy.</p>
              </section>
              <section class="set-section backup-risk">
                <h3>What exporting changes</h3>
                <p class="muted small"><strong>No plaintext is exported</strong>, so the cryptographic confidentiality of each record is unchanged. The tradeoff is exposure: Downloads now holds another offline target that can be copied and guessed indefinitely, plus visible folder names, file sizes, timestamps and blob layout.</p>
                <ul class="muted small">
                  <li>It preserves the state and key material present at backup time, including material later deleted from the live vault.</li>
                  <li>Changing the live vault secret does not revoke an older copy; that copy continues to use its old secret.</li>
                  <li>Record authentication detects tampering when opened, but this export does not yet include a separately verified whole-backup manifest.</li>
                  <li>A compromised unlocked process, malware or keylogger remains outside at-rest encryption's protection.</li>
                </ul>
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
                    <span class="muted small">Interface text size: {appearance.scale || 100}%</span>
                    <input
                      type="range"
                      min="70"
                      max="200"
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
              <section class="set-section">
                <h3>Message text effects</h3>
                <p class="muted small">
                  Your local comfort setting for effects other people add to prose. The operating
                  system's reduced-motion preference also forces Low. Censored text always stays
                  concealed until you reveal it.
                </p>
                <div class="field">
                  <span class="muted small">Playback</span>
                  <div class="stx-seg text-fx-mode">
                    <button type="button" class:on={!appearance.textEffects} onclick={() => (appearance = { ...appearance, textEffects: "" })}>FULL</button>
                    <button type="button" class:on={appearance.textEffects === "low"} onclick={() => (appearance = { ...appearance, textEffects: "low" })}>LOW</button>
                    <button type="button" class:on={appearance.textEffects === "off"} onclick={() => (appearance = { ...appearance, textEffects: "off" })}>PLAIN</button>
                  </div>
                </div>
                <p class="muted small">
                  Full includes animation, pointer reactions, and authored effect audio. Low keeps
                  a static visual identity and stays silent. Plain shows ordinary readable text.
                </p>
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
                    checked={appearance.messageMotion !== "off"}
                    onchange={() => (appearance = { ...appearance, messageMotion: appearance.messageMotion === "off" ? "" : "off" })}
                  />
                  <span>Message arrivals: let each member's messages use that member's chosen entrance</span>
                </label>
                <label class="toggle">
                  <input
                    type="checkbox"
                    checked={appearance.flat}
                    disabled={!CHAT_MESSAGE_FRAMES_ENABLED}
                    onchange={() => (appearance = { ...appearance, flat: !appearance.flat })}
                  />
                  <span>{CHAT_MESSAGE_FRAMES_ENABLED
                    ? "Flatten other members' custom message frames (mine stays visible)"
                    : "Custom message frames are temporarily disabled in live chats"}</span>
                </label>
                <p class="muted small">Arrival motion remains available. Frame choices and previews are preserved while live-chat backgrounds are paused.</p>
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
                <h3>Room</h3>
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
                  <button type="button" class="ghost small" onclick={tidySpace}>Auto-arrange</button>
                  <button type="button" class="ghost small" onclick={undoSpaceLayout} disabled={!spaceUndo.length}>Undo move</button>
                  <button type="button" class="ghost small" onclick={redoSpaceLayout} disabled={!spaceRedo.length}>Redo</button>
                  <button type="button" class="ghost small" onclick={forgetSpacePlacements}>Forget placements</button>
                </div>
                <div class="stx-duo space-controls">
                  <div class="field">
                    <span class="muted small">Viewport shape</span>
                    <div class="stx-seg">
                      <button type="button" class:on={spaceState.shape === "circle"} onclick={() => setSpaceShape("circle")}>CIRCLE</button>
                      <button type="button" class:on={spaceState.shape === "square"} onclick={() => setSpaceShape("square")}>SQUARE</button>
                    </div>
                  </div>
                  <label class="field">
                    <span class="muted small">Server size: {spaceState.serverSize}px</span>
                    <input type="range" min="32" max="88" step="2" value={spaceState.serverSize} oninput={(e) => setSpaceServerSize(+e.currentTarget.value)} />
                  </label>
                </div>
                <label class="toggle">
                  <input type="checkbox" checked={spaceState.zoomOnOpen} onchange={() => { spaceState.zoomOnOpen = !spaceState.zoomOnOpen; saveSpace(); }} />
                  <span>Zoom through a server when opening it</span>
                </label>
                <label class="toggle">
                  <input type="checkbox" checked={spaceState.entrySound} onchange={() => { spaceState.entrySound = !spaceState.entrySound; saveSpace(); }} />
                  <span>Play the painted-portal sound when entering a server</span>
                </label>
                <label class="toggle">
                  <input type="checkbox" checked={spaceState.showMinimap} onchange={() => { spaceState.showMinimap = !spaceState.showMinimap; saveSpace(); }} />
                  <span>Show the orientation compass and off-screen servers</span>
                </label>
                <p class="muted small">Backdrop, shape, size, and placements stay on this device, like desktop icon positions. Drops automatically make room so servers cannot overlap.</p>
                <div class="space-layout-actions">
                  <button type="button" class="ghost" onclick={exportSpaceLayout} disabled={spaceLayoutBusy}>{spaceLayoutBusy ? "Exporting…" : "Export layout"}</button>
                  <label class="ghost space-file">
                    Import layout…
                    <input bind:this={spaceLayoutInput} type="file" accept="application/json,.json" onchange={(e) => importSpaceLayout(e.currentTarget.files)} />
                  </label>
                </div>
                <div class="space-input-help" aria-label="Server Space controls">
                  <span><kbd>Arrows</kbd> look</span>
                  <span><kbd>Tab</kbd> cycle servers</span>
                  <span><kbd>Enter</kbd> open focused</span>
                  <span><kbd>/</kbd> search</span>
                  <span><kbd>Gamepad</kbd> left stick · bumpers · A/X/B</span>
                </div>
              </section>
              <section class="set-section">
                <h3>Neighbourhoods</h3>
                <p class="muted small">Group servers into named areas. In Space, use the neighbourhood panel's ＋ list or drag a server onto a neighbourhood; auto-arrange gives every group its own part of the sky.</p>
                <div class="space-cluster-add">
                  <input value={spaceNewCluster} maxlength="32" placeholder="Friends, Games, Work…" oninput={(e) => (spaceNewCluster = e.currentTarget.value)} onkeydown={(e) => { if (e.key === "Enter") addSpaceCluster(); }} />
                  <input type="color" value={spaceNewClusterColor} aria-label="Neighbourhood colour" oninput={(e) => (spaceNewClusterColor = e.currentTarget.value)} />
                  <button type="button" class="ghost" onclick={addSpaceCluster} disabled={!spaceNewCluster.trim()}>Add neighbourhood</button>
                </div>
                {#if spaceState.clusters.length}
                  <div class="space-cluster-list">
                    {#each spaceState.clusters as cluster (cluster.id)}
                      <div class="space-cluster-row">
                        <input type="color" value={cluster.color} aria-label={`${cluster.name} colour`} oninput={(e) => updateSpaceCluster(cluster.id, { color: e.currentTarget.value })} />
                        <input value={cluster.name} maxlength="32" aria-label="Neighbourhood name" oninput={(e) => updateSpaceCluster(cluster.id, { name: e.currentTarget.value.slice(0, 32) })} />
                        <span>{Object.values(spaceState.serverClusters).filter((id) => id === cluster.id).length} servers</span>
                        <button type="button" class="ghost small" onclick={() => removeSpaceCluster(cluster.id)}>Remove</button>
                      </div>
                    {/each}
                  </div>
                {/if}
              </section>
              <section class="set-section">
                <h3>Atmosphere &amp; motion</h3>
                <div class="stx-duo space-fx-controls">
                  <label class="field"><span class="muted small">Ambient particles: {spaceState.ambience}%</span><input type="range" min="0" max="100" step="5" value={spaceState.ambience} oninput={(e) => { spaceState.ambience = +e.currentTarget.value; saveSpace(); }} /></label>
                  <label class="field"><span class="muted small">Constellation lines: {spaceState.links}%</span><input type="range" min="0" max="100" step="5" value={spaceState.links} oninput={(e) => { spaceState.links = +e.currentTarget.value; saveSpace(); }} /></label>
                  <label class="field"><span class="muted small">Server glow &amp; rings: {spaceState.glow}%</span><input type="range" min="0" max="100" step="5" value={spaceState.glow} oninput={(e) => { spaceState.glow = +e.currentTarget.value; saveSpace(); }} /></label>
                  <label class="field"><span class="muted small">Backdrop blur: {spaceState.backdropBlur}px</span><input type="range" min="0" max="12" step="1" value={spaceState.backdropBlur} oninput={(e) => { spaceState.backdropBlur = +e.currentTarget.value; saveSpace(); }} /></label>
                </div>
                <label class="toggle"><input type="checkbox" checked={spaceState.hoverShake} onchange={() => { spaceState.hoverShake = !spaceState.hoverShake; saveSpace(); }} /><span>Shake a server strongly on hover</span></label>
              </section>
              <section class="set-section">
                <h3>Custom image guide</h3>
                <div class="space-guide">
                  <div>
                    <strong>Use a 2:1 equirectangular panorama</strong>
                    <p>2048 × 1024 works best. Keep important detail near the horizon and inside the middle half; make the left and right edges seamless. Other aspect ratios are centre-cropped automatically.</p>
                    <p>The template marks all four viewing directions, cube seams, and circle-safe areas. It saves to Downloads and opens in your default image viewer; use it as an overlay in your image editor, then hide the guide before exporting.</p>
                    {#if spaceImageNote}<p class="space-image-note">{spaceImageNote}</p>{/if}
                  </div>
                  <button type="button" class="ghost" onclick={downloadSpaceTemplate} disabled={spaceGuideSaving}>
                    {spaceGuideSaving ? "Opening guide…" : "Save & open 2048 × 1024 guide"}
                  </button>
                </div>
                {#if spaceState.custom}
                  <div class="space-pano-tools">
                    <div class="space-pano-preview" style={`background-image:url(${spaceState.custom})`}>
                      <span class="sp-pano-horizon"></span>
                      {#each [[0, "BACK"], [25, "LEFT"], [50, "FRONT"], [75, "RIGHT"], [100, "BACK"]] as [left, label]}
                        <span class="sp-pano-cardinal" style={`left:${left}%`}>{label}</span>
                      {/each}
                    </div>
                    <label class="field"><span class="muted small">Preview direction: {spacePanoYaw}°</span><input type="range" min="0" max="270" step="90" value={spacePanoYaw} oninput={(e) => (spacePanoYaw = +e.currentTarget.value)} /></label>
                    <div class="space-pano-window" style={`background-image:url(${spaceState.custom}); background-position:${panoPos(spacePanoYaw)}`}></div>
                    <label class="toggle"><input type="checkbox" checked={spaceSeamPreview} onchange={() => (spaceSeamPreview = !spaceSeamPreview)} /><span>Show the left/right seam check</span></label>
                    {#if spaceSeamPreview}
                      <div class="space-seam-preview">
                        <span style={`background-image:url(${spaceState.custom}); background-position:left center`}></span>
                        <span style={`background-image:url(${spaceState.custom}); background-position:right center`}></span>
                        <b>SEAM</b>
                      </div>
                    {/if}
                  </div>
                {/if}
              </section>
            {:else if settingsPage === "notifications"}
              <div class="stx-crumb">SETTINGS // APP // NOTIFICATIONS</div>
              <h1>Notifications</h1>
              <section class="set-section">
                <h3>Global sound defaults</h3>
                <label class="toggle">
                  <input type="checkbox" checked={soundOn} onchange={toggleSound} />
                  <span>Play app sounds on this device</span>
                </label>
                <p class="muted small">This master switch silences notification tones and Server Space effects. Each server can inherit or override the three notification categories below.</p>
              </section>
              <section class="set-section">
                <h3>Notification tones</h3>
                <div class="sound-settings-list">
                  {#each NOTIFICATION_SOUND_KINDS as kind (kind)}
                    {@const pref = globalSoundPrefs[kind]}
                    <article class="sound-setting-row">
                      <div class="sound-setting-head">
                        <div><strong>{SOUND_LABELS[kind].title}</strong><span>{SOUND_LABELS[kind].detail}</span></div>
                        <label class="toggle compact">
                          <input type="checkbox" checked={pref.enabled} onchange={(e) => setGlobalSoundEnabled(kind, e.currentTarget.checked)} />
                          <span>{pref.enabled ? "On" : "Off"}</span>
                        </label>
                      </div>
                      <div class="sound-setting-controls">
                        <label>
                          <span>Tone</span>
                          <select value={pref.tone} onchange={(e) => setGlobalToneMode(kind, e.currentTarget.value as "default" | "custom")}>
                            <option value="default">Built-in</option>
                            <option value="custom" disabled={!pref.custom}>Custom{pref.custom ? ` · ${pref.custom.name}` : ""}</option>
                          </select>
                        </label>
                        <button type="button" class="ghost small" disabled={!soundOn || !pref.enabled} onclick={() => playConfiguredSound(kind, null)}>Test</button>
                        <label class="ghost small sound-file">
                          {pref.custom ? "Replace custom…" : "Choose custom…"}
                          <input type="file" accept="audio/mpeg,audio/wav,audio/ogg,audio/webm,audio/mp4,audio/aac,audio/flac,.mp3,.wav,.ogg,.webm,.m4a,.aac,.flac" onchange={(e) => { const input = e.currentTarget; void importCustomTone("global", kind, input.files).finally(() => (input.value = "")); }} />
                        </label>
                        {#if pref.custom}
                          <button type="button" class="ghost small" onclick={() => removeCustomTone("global", kind)}>Remove custom</button>
                        {/if}
                      </div>
                    </article>
                  {/each}
                </div>
                <p class="muted small">Custom tones stay on this device. MP3/WAV/OGG/WebM/M4A/AAC/FLAC · up to {MAX_CUSTOM_TONE_SECONDS}s and {Math.round(MAX_CUSTOM_TONE_BYTES / 1024)} KiB.</p>
              </section>
              <section class="set-section">
                <h3>Current server</h3>
                {#if activeServerId !== null && cur}
                  <div class="sound-effective-list">
                    {#each NOTIFICATION_SOUND_KINDS as kind (kind)}
                      {@const effective = soundPolicy(kind, activeServerId)}
                      <span><b>{SOUND_LABELS[kind].title}</b><i class:on={effective.enabled}>{effective.enabled ? "enabled" : "disabled"}</i><small>{effective.source}</small></span>
                    {/each}
                    <span><b>Voice-call banner</b><i class:on={acceptCallsHere}>{acceptCallsHere ? "enabled" : "disabled"}</i><small>server setting</small></span>
                  </div>
                  <button type="button" class="ghost small" onclick={() => { showSettings = false; openServerSettings(null, "notifications"); }}>Open {cur.name} overrides</button>
                {:else}
                  <p class="muted small">Open a server to inspect or change its overrides.</p>
                {/if}
              </section>
            {:else if settingsPage === "voice"}
              <div class="stx-crumb">SETTINGS // APP // VOICE &amp; CALLS</div>
              <h1>Voice &amp; Calls</h1>
              <section class="set-section">
                <h3>Devices</h3>
                <p class="muted small">Microphone and output pickers live on the call stage (they swap live, mid-call) and are remembered here between calls.</p>
                <p class="muted small">A MIDI keyboard for the call instrument is set up in Settings → Devices, along with a monitor for checking one that is not behaving.</p>
                <button type="button" class="ghost small" onclick={() => (settingsPage = "devices")}>Open Devices</button>
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
                  <li><code>[fx:cyber]signal[/fx]</code> → a text effect; select words or use the Aa FX button for the catalog</li>
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
                  <li><kbd>Ctrl+1…7</kbd><span>Surfaces: chat, files, announcements, wiki, profile, downloads, events</span></li>
                  <li><kbd>Ctrl+B / Ctrl+I</kbd><span>Bold / italic in the composer</span></li>
                  <li><kbd>Ctrl+Shift+F</kbd><span>Search with the filter panel open</span></li>
                  <li><kbd>Ctrl+L</kbd><span>Lock the session</span></li>
                  <li><kbd>Ctrl+O</kbd><span>The 360° server space</span></li>
                  <li><kbd>Alt+← / →</kbd><span>Back / forward through where you have been</span></li>
                  <li><kbd>T</kbd> <span>(held, in the space) the tray of unplaced servers</span></li>
                  <li><kbd>Z / X</kbd><span>Piano octave down / up (lock screen and instrument drawer)</span></li>
                  <li><kbd>Esc</kbd><span>Close the topmost thing, one layer at a time</span></li>
                </ul>
              </section>
              <section class="set-section text-fx-keybinds">
                <div class="text-fx-keybind-head">
                  <div>
                    <h3>Text-effect shortcuts</h3>
                    <p class="muted small">Select text in any supported editor, then use its shortcut. Custom bindings stay on this device.</p>
                  </div>
                  <button type="button" class="ghost small" onclick={resetTextEffectKeybinds}>Reset defaults</button>
                </div>
                {#if textEffectKeyError}<p class="form-error" role="alert">{textEffectKeyError}</p>{/if}
                {#each TEXT_EFFECT_GROUPS as group}
                  <h4 class="text-fx-keygroup">{group}</h4>
                  <ul class="text-fx-keylist">
                    {#each TEXT_EFFECTS.filter((effect) => effect.group === group) as effect (effect.id)}
                      <li>
                        <span class="text-fx-key-preview" aria-hidden="true">{@html textEffectHtml(effect.id, "Aa")}</span>
                        <span class="text-fx-key-name"><strong>{effect.label}</strong><small>{effect.description}</small></span>
                        <kbd>{textEffectKeybinds[effect.id] || "Unassigned"}</kbd>
                        <button
                          type="button"
                          class="ghost small text-fx-record"
                          class:active={recordingTextEffect === effect.id}
                          onclick={() => { recordingTextEffect = effect.id; textEffectKeyError = ""; }}
                          onkeydown={(event) => recordTextEffectKey(event, effect.id)}
                        >{recordingTextEffect === effect.id ? "Press keys…" : "Change"}</button>
                        <button type="button" class="ghost small" disabled={!textEffectKeybinds[effect.id]} onclick={() => clearTextEffectKeybind(effect.id)}>Clear</button>
                      </li>
                    {/each}
                  </ul>
                {/each}
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
            {:else if settingsPage === "diagnostics"}
              <div class="stx-crumb">SETTINGS // CONNECTION // DIAGNOSTICS</div>
              <h1>Diagnostics</h1>
              <section class="set-section">
                <h3>Connection report</h3>
                {#if connectivity && connectivity.action}
                  {@const reach = reachabilitySummary(connectivity)}
                  <p class="muted small">
                    The last thing this app tried: <b>{connectivity.action === "found" ? "founding" : "joining"}</b>
                    {connectivity.subject ? ` (${connectivity.subject})` : ""}, {fmtLocal(connectivity.at)}.
                  </p>
                  <p class="muted small">Observed reachability: <b>{reach.verdict}</b>. {reach.detail}</p>
                  <div class="invite-actions">
                    <button class="ghost small" onclick={refreshConnectivity}>Refresh</button>
                    <button class="ghost small" onclick={copyConnectivity}>{connCopied ? "Copied!" : "Copy report"}</button>
                  </div>
                  {@render connDetail(connectivity)}
                {:else}
                  <p class="muted small">Nothing has been founded or joined since the app started, so there is nothing to report yet.</p>
                {/if}
              </section>
              <section class="set-section">
                <h3>Debug log</h3>
                <p class="muted small">
                  Off by default. When on, Mewtual writes a text log next to its data so you can
                  reproduce a problem and send the file to someone who can read it.
                </p>
                {#if debugLog}
                  <label class="toggle">
                    <input type="checkbox" checked={debugLog.enabled} disabled={debugLogBusy}
                      onchange={(e) => toggleDebugLog(e.currentTarget.checked)} />
                    <span>Keep a debug log</span>
                  </label>
                  {#if debugLog.enabled && !debugLog.active}
                    <p class="muted small">Restart Mewtual to start writing: a log can only be opened when the app starts.</p>
                  {:else if !debugLog.enabled && debugLog.active}
                    <p class="muted small">Still writing this session's log. It stops at the next restart.</p>
                  {/if}
                  <div class="field">
                    <span class="muted small">Log folder</span>
                    <input readonly value={debugLog.dir} />
                  </div>
                  {#if debugLog.file}
                    <p class="muted small">This session's file: <span class="fp">{debugLog.file}</span></p>
                  {:else}
                    <p class="muted small">Files are named <code>debug_log_&lt;date&gt;_&lt;time&gt;.txt</code>.</p>
                  {/if}
                  <div class="invite-actions">
                    <button class="ghost small" onclick={() => copyText(debugLog?.dir ?? "")}>Copy folder path</button>
                  </div>
                {:else}
                  <p class="muted small">This build cannot report the log setting.</p>
                {/if}
                <p class="muted small">
                  <b>Before you share one:</b> a debug log can contain your LAN and public IP
                  addresses and port, the addresses of peers you connected to, peer and device
                  identifiers, and when you were online and how much you transferred. It does
                  <b>not</b> contain message text, file contents, names or any key material.
                  Treat it as "who I talked to and when".
                </p>
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
              {@render profilePreview()}
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
              </section>
            {:else if serverSettingsPage === "notifications"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // NOTIFICATIONS</div>
              <h1>Notifications</h1>
              <section class="set-section">
                <h3>Sound overrides</h3>
                <p class="muted small">Inherit follows Settings → Notifications. “On” or “Off” overrides that category for this server; the device-wide master switch can still silence everything.</p>
                <div class="sound-settings-list">
                  {#each NOTIFICATION_SOUND_KINDS as kind (kind)}
                    {@const pref = serverSoundPrefs[kind]}
                    {@const effective = soundPolicy(kind, activeServerId)}
                    <article class="sound-setting-row">
                      <div class="sound-setting-head">
                        <div><strong>{SOUND_LABELS[kind].title}</strong><span>{SOUND_LABELS[kind].detail}</span></div>
                        <span class="sound-effective" class:on={effective.enabled}>{effective.enabled ? "Enabled" : "Disabled"} · {effective.source}</span>
                      </div>
                      <div class="sound-setting-controls">
                        <label>
                          <span>Enabled</span>
                          <select value={pref.enabled} onchange={(e) => setServerSoundEnabled(kind, e.currentTarget.value as SoundOverride)}>
                            <option value="inherit">Inherit global</option>
                            <option value="on">On for this server</option>
                            <option value="off">Off for this server</option>
                          </select>
                        </label>
                        <label>
                          <span>Tone</span>
                          <select value={pref.tone} onchange={(e) => setServerToneMode(kind, e.currentTarget.value as ToneOverride)}>
                            <option value="inherit">Inherit global</option>
                            <option value="default">Built-in</option>
                            <option value="custom" disabled={!pref.custom}>Custom{pref.custom ? ` · ${pref.custom.name}` : ""}</option>
                          </select>
                        </label>
                        <button type="button" class="ghost small" disabled={!effective.enabled} onclick={() => playConfiguredSound(kind, activeServerId)}>Test</button>
                        <label class="ghost small sound-file">
                          {pref.custom ? "Replace custom…" : "Choose custom…"}
                          <input type="file" accept="audio/mpeg,audio/wav,audio/ogg,audio/webm,audio/mp4,audio/aac,audio/flac,.mp3,.wav,.ogg,.webm,.m4a,.aac,.flac" onchange={(e) => { const input = e.currentTarget; void importCustomTone("server", kind, input.files).finally(() => (input.value = "")); }} />
                        </label>
                        {#if pref.custom}
                          <button type="button" class="ghost small" onclick={() => removeCustomTone("server", kind)}>Remove custom</button>
                        {/if}
                      </div>
                    </article>
                  {/each}
                </div>
              </section>
              <section class="set-section">
                <h3>Voice calls</h3>
                <label class="toggle">
                  <input type="checkbox" checked={acceptCallsHere} onchange={toggleAcceptCalls} />
                  <span>Notify me when a voice room becomes active on this server</span>
                </label>
                <p class="muted small">The voice banner uses the effective Mentions &amp; replies tone. Turning this off suppresses the banner and its sound.</p>
              </section>
              <section class="set-section">
                <button type="button" class="ghost small" onclick={() => { showServerSettings = false; openSettings("notifications"); }}>Open global sound defaults</button>
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
            {:else if serverSettingsPage === "joinlog"}
              <div class="stx-crumb">SERVER // {cur?.name?.toUpperCase()} // JOIN LOG</div>
              <h1>Join Log</h1>
              <section class="set-section">
                <p class="muted small">
                  Every join request this device answered since it started, newest first, and why
                  each one was refused. Match an entry to the invite you sent by its invite code:
                  the first bytes of the invite's one-time nonce.
                </p>
                <p class="muted small">
                  The person joining only ever sees "rejected": the reason is deliberately not
                  put on the wire, so nobody can probe your invites. This is the other half of it.
                  Nothing here is kept after you close the app.
                </p>
                <div class="invite-actions">
                  <button class="ghost small" onclick={refreshJoinAttempts}>Refresh</button>
                  <button class="ghost small" disabled={!joinAttempts.length} onclick={copyJoinLog}>
                    {joinLogCopied ? "Copied!" : "Copy as text"}
                  </button>
                </div>
              </section>
              <section class="set-section">
                {#if !joinAttempts.length}
                  <p class="muted small">
                    No join requests have reached this device yet. If someone says their invite was
                    rejected and nothing appears here, their app never reached you at all: that is a
                    connectivity problem, not an invite problem.
                  </p>
                {:else}
                  <ul class="join-log">
                    {#each joinAttempts as a, i (i)}
                      {@const c = describeOutcome(a.outcome)}
                      <li class={c.tone}>
                        <div class="jl-head">
                          <span class="jl-when">{fmtLocal(a.at)}</span>
                          <span class="jl-what">{c.label}</span>
                          <span class="jl-ids">
                            invite <span class="fp">{a.nonce || "unknown"}</span>
                            · peer <span class="fp">{a.peer || "unknown"}</span>
                          </span>
                        </div>
                        <p class="muted small jl-note">{c.note}</p>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </section>
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

            {#if fileTextKind}
              <div class="file-text">
                <div class="file-text-bar">
                  {#if fileTextKind === "markdown"}
                    <div class="file-text-toggle" role="group" aria-label="Markdown view">
                      <button
                        type="button"
                        class:active={fileTextMode === "render"}
                        aria-pressed={fileTextMode === "render"}
                        onclick={() => (fileTextMode = "render")}
                      >rendered</button>
                      <button
                        type="button"
                        class:active={fileTextMode === "source"}
                        aria-pressed={fileTextMode === "source"}
                        onclick={() => (fileTextMode = "source")}
                      >source</button>
                    </div>
                  {:else}
                    <span class="muted small">{fileInfo.mime || "plain text"}</span>
                  {/if}
                  <span class="file-text-spacer"></span>
                  {#if fileTextState === "ready"}
                    <span class="muted small">{lineCountLabel(fileTextLines)}</span>
                    {#if !fileTextRendered}
                      <div class="file-text-toggle">
                        <button
                          type="button"
                          class:active={fileTextWrap}
                          aria-pressed={fileTextWrap}
                          title="Wrap long lines instead of scrolling sideways"
                          onclick={() => (fileTextWrap = !fileTextWrap)}
                        >↩ wrap</button>
                      </div>
                    {/if}
                  {/if}
                </div>

                {#if fileTextState === "loading"}
                  <p class="muted small file-text-note">Loading…</p>
                {:else if fileTextState === "too-big"}
                  <p class="muted small file-text-note">
                    {fmtSize(fileInfo.size)} is past the {fmtSize(TEXT_PREVIEW_MAX_BYTES)} inline limit.
                    <button class="ghost small" onclick={() => fileInfo && loadFileText(fileInfo, true)}>Read it anyway</button>
                  </p>
                {:else if fileTextState === "binary"}
                  <p class="muted small file-text-note">This isn't readable text: download it and open it in the right app.</p>
                {:else if fileTextState === "error"}
                  <p class="muted small file-text-note">Can't read it: the file isn't downloaded yet and no peer is sharing it right now.</p>
                {:else if fileTextState === "ready"}
                  {#if !fileText.trim()}
                    <p class="muted small file-text-note">This file is empty.</p>
                  {:else if fileTextRendered}
                    <div class="file-text-body wiki-render" use:richClicks>{@html fileTextHtml}</div>
                  {:else}
                    <pre class="file-text-body source" class:wrap={fileTextWrap}>{fileText}</pre>
                  {/if}
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
                      <span class="usage-count">{fileInfoUsage.status_count} announcement{fileInfoUsage.status_count === 1 ? "" : "s"}</span>
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

            {#if activeServerId !== null && downloads[dlKey(activeServerId, fileInfo.cid)] && downloads[dlKey(activeServerId, fileInfo.cid)].status !== "done" && downloads[dlKey(activeServerId, fileInfo.cid)].status !== "failed"}
              {@const di = downloads[dlKey(activeServerId, fileInfo.cid)]}
              <label class="dl-progress">
                <span class="muted small">
                  {#if di.status === "verifying"}Verifying…
                  {:else if !di.provider && onlineCount <= 1 && di.done < di.total}No connection · waiting for a member
                  {:else if di.status === "queued" || di.status === "waiting"}Waiting for source…
                  {:else}Receiving… {Math.round(di.progress * 100)}%{/if}
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
      {#if FeedbackOverlay}
        <FeedbackOverlay
          version={APP_VERSION}
          onclose={() => (showFeedback = false)}
          onerror={(message) => (error = message)}
        />
      {:else}
        <!-- Keep failure local and retryable; a failed optional chunk must not destabilize chat. -->
        <div class="overlay" role="presentation">
          <div class="overlay-card">
            <header class="overlay-head"><h2>Send feedback</h2><button class="ghost" onclick={() => (showFeedback = false)}>✕</button></header>
            <div class="overlay-body">
              <p class="muted">{feedbackOverlayError ? "The feedback view could not be loaded." : "Loading feedback…"}</p>
              {#if feedbackOverlayError}<button class="ghost" onclick={loadFeedbackOverlay}>Retry</button>{/if}
            </div>
          </div>
        </div>
      {/if}
    {/if}

    {#if showWikiHelp}
      {#if WikiHelpOverlay}
        <WikiHelpOverlay onclose={() => (showWikiHelp = false)} />
      {:else}
        <div class="overlay" role="presentation">
          <div class="overlay-card">
            <header class="overlay-head"><h2>Wiki formatting</h2><button class="ghost" onclick={() => (showWikiHelp = false)}>✕</button></header>
            <div class="overlay-body">
              <p class="muted">{wikiHelpLoadError ? "Wiki help could not be loaded." : "Loading wiki help…"}</p>
              {#if wikiHelpLoadError}<button class="ghost" onclick={loadWikiHelpOverlay}>Retry</button>{/if}
            </div>
          </div>
        </div>
      {/if}
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
