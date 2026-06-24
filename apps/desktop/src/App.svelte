<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { renderMessage, renderWiki } from "./render";

  type Msg = { author: string; text: string; ts: number };
  type Channel = { id: string; name: string };
  type Member = { fingerprint: string; you: boolean };
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string; avatar: string };
  type UiFile = { name: string; size: number; mime: string; cid: string; author: string; path: string; held: number; total: number };
  type Found = { server: number; channel: string };
  type Reloaded = { server: number; name: string; invite: string; channel: string };

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
  };

  let servers = $state<ServerState[]>([]);
  let activeServerId = $state<number | null>(null);
  let showAdd = $state(false); // showing the found/join form to add a server
  let showSettings = $state(false); // the personal/app Settings overlay
  let showServerSettings = $state(false); // the per-server (admin) Settings overlay
  let serverNameDraft = $state("");

  function openServerSettings(id: number | null = null) {
    if (id !== null && id !== activeServerId) switchServer(id);
    serverNameDraft = cur?.name ?? "";
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

  // Persistence (9f): a passphrase gate. On launch the app is locked until the user enters
  // their passphrase, which unlocks the on-disk vault and reloads their servers (or, on
  // first run, sets the passphrase and starts empty).
  let locked = $state(true);
  let passphrase = $state("");
  let unlocking = $state(false);

  let busy = $state(false);
  let error = $state("");
  let displayName = $state("me");
  let advertise = $state(""); // optional reachable address (LAN/public IP) for the founder
  let relay = $state(""); // optional relay-node multiaddr (zero-config NAT traversal)
  // Optional rendezvous multiaddr — when set, the founder registers there so a joiner discovers
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
  let draft = $state("");
  let members = $state(1);
  let roster = $state<Member[]>([]);
  // Fingerprints of members reachable right now (a live connection) — drives the roster's online
  // dots + the online count. Refreshed with the roster and updated live by 'connectivity-changed'.
  let onlineMembers = $state<Set<string>>(new Set());
  let rosterFilter = $state("");
  let filteredRoster = $derived.by(() => {
    const q = rosterFilter.trim().toLowerCase();
    if (!q) return roster;
    return roster.filter(
      (m) => m.fingerprint.toLowerCase().includes(q) || nameOf(m.fingerprint).toLowerCase().includes(q),
    );
  });
  // Members reachable right now (self always counts) — the roster header's "N online".
  let onlineCount = $derived(roster.filter((m) => m.you || onlineMembers.has(m.fingerprint)).length);
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
  let emojiMap = $derived.by(() => {
    const m: Record<string, string> = {};
    for (const f of files) {
      if (f.path === "emoji") {
        const code = f.name.replace(/\.[^.]+$/, "").toLowerCase();
        if (code) m[code] = f.cid;
      }
    }
    return m;
  });

  // The main pane shows one tab at a time.
  type Tab = "chat" | "files" | "status" | "wiki" | "profile" | "downloads";
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
  let pAvatar = $state("");

  let cur = $derived(servers.find((s) => s.id === activeServerId) ?? null);
  let myFp = $derived(roster.find((r) => r.you)?.fingerprint ?? "");
  // Reserved fileshare folder for chat/status media embeds uploaded by this member.
  let myEmbedFolder = $derived(myFp ? `embed/${myFp}` : "embed");
  // Member roles (10h): fingerprint -> "owner"|"admin"|"member".
  let roles = $state<Record<string, string>>({});
  let myRole = $derived(roles[myFp] ?? "member");
  // Owners + admins may invite. Admin invites are owner-serialized end-to-end (the joiner is
  // admitted when the owner is next online), and revocation is replay-proof (THREAT-MODEL item 3),
  // so this is safe to surface to admins.
  let canInvite = $derived(myRole === "owner" || myRole === "admin");
  let confirmRemoveFp = $state(""); // two-click confirm for member removal

  function activeName(): string {
    return cur?.channels.find((c) => c.id === cur?.active)?.name ?? "";
  }
  function nameOf(fp: string): string {
    return profiles[fp]?.name?.trim() || fp;
  }
  function fontClass(font: string): string {
    return font === "serif" ? "font-serif" : font === "mono" ? "font-mono" : "";
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
    return effect && effect !== "none" ? `fx-${effect}` : "";
  }
  function colorStyle(color: string): string {
    return color ? `color:${color}` : "";
  }
  function fmtTime(ts: number): string {
    return ts ? new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "";
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
  });

  // Resolve inline media embeds + custom emoji whenever content or the file index changes.
  // The `tick()` is essential: it waits for Svelte to commit the `{@html renderMessage(...)}`
  // DOM so the [data-embed-cid]/[data-emoji] placeholders exist before we query for them.
  // Without it, on a fresh mount (app restart / HMR / tab switch) this effect runs in the
  // same flush as the {@html} block, finds zero placeholders, and never re-runs — so embeds
  // render on first send but vanish after a restart.
  $effect(() => {
    void messages;
    void statuses;
    void files;
    void emojiUrls;
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
      if (!url) continue; // unknown / not loaded yet — leave :code: text, retry on update
      span.setAttribute("data-resolved", "1");
      const img = document.createElement("img");
      img.src = url;
      img.className = "emoji";
      img.alt = `:${code}:`;
      img.title = `:${code}:`;
      span.replaceWith(img);
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
      await invoke("add_file", {
        server: activeServerId,
        name: code,
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

  // Unlock the vault with the entered passphrase and reload persisted servers (9f). A wrong
  // passphrase fails (the vault won't decrypt) and we stay locked, showing the error.
  async function unlock() {
    unlocking = true;
    error = "";
    try {
      const reloaded = await invoke<Reloaded[]>("unlock", { passphrase });
      for (const r of reloaded) {
        servers = [
          ...servers,
          { id: r.server, name: r.name, channels: [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: r.invite, dot: false },
        ];
      }
      locked = false;
      passphrase = "";
      if (servers.length) switchServer(servers[0].id);
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
      const r = await invoke<Found>("found_server", { displayName, advertise, relay, rendezvous });
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
      const r = await invoke<Found>("join_server", { inviteHex: joinInvite.trim(), displayName });
      addServer(r, displayName);
      joinInvite = "";
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function addServer(r: Found, name: string) {
    servers = [
      ...servers,
      { id: r.server, name, channels: [{ id: r.channel, name: "general" }], active: r.channel, unread: [], invite: "", dot: false },
    ];
    showAdd = false;
    pName = name;
    switchServer(r.server);
  }

  async function switchServer(id: number) {
    activeServerId = id;
    const s = servers.find((x) => x.id === id);
    if (s) s.dot = false;
    // Each server has its own wiki + fileshare; reset per-server view state.
    view = "chat";
    activeWikiPage = "";
    wikiBody = "";
    wikiDirty = false;
    wikiEdit = false;
    folder = "";
    newFolder = "";
    await Promise.all([
      refresh(),
      refreshMembers(),
      refreshProfiles(),
      refreshFiles(),
      refreshStatuses(),
      refreshInvite(),
      refreshRoles(),
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

  function switchTo(id: string) {
    if (!cur) return;
    cur.active = id;
    cur.unread = cur.unread.filter((c) => c !== id);
    refresh();
  }

  async function refresh() {
    if (!cur || !cur.active || activeServerId === null) return;
    try {
      messages = await invoke<Msg[]>("get_messages", { server: activeServerId, channel: cur.active });
    } catch (e) {
      error = String(e);
    }
  }
  async function refreshMembers() {
    const id = activeServerId;
    if (id === null) return;
    try {
      const r = await invoke<Member[]>("get_members", { server: id });
      const online = await invoke<string[]>("get_online_members", { server: id });
      if (activeServerId !== id) return; // server switched mid-fetch — drop stale results
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
  // fetchable from peers / no peers online — or actively downloading. Reactive (reads files,
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
    const el = (e.target as HTMLElement | null)?.closest("[data-wikilink]") as HTMLElement | null;
    if (el) {
      e.preventDefault();
      const page = el.getAttribute("data-wikilink") ?? "";
      if (page) {
        view = "wiki";
        await openWikiPage(page);
      }
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
    return [
      { label: "Copy text", icon: "⧉", onSelect: () => copyText(m.text) },
      { label: "Quote in reply", icon: "❝", onSelect: () => appendToDraft(`> ${nameOf(m.author)}: ${m.text}`) },
      { divider: true },
      { label: "Copy sender fingerprint", icon: "#", onSelect: () => copyText(m.author) },
    ];
  }

  function memberMenu(m: Member): MenuItem[] {
    const items: MenuItem[] = [
      { label: "Copy fingerprint", icon: "#", onSelect: () => copyText(m.fingerprint) },
    ];
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

  // Context menu on rendered rich text: copy/post a [[wikilink]], copy a :emoji:, copy an embed.
  function handleRichContext(e: MouseEvent) {
    const el = (e.target as HTMLElement | null)?.closest(
      "[data-wikilink],[data-emoji],[data-embed-cid]",
    ) as HTMLElement | null;
    if (!el) return;
    if (el.hasAttribute("data-wikilink")) {
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

  async function loadAvatar(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file) return;
    try {
      const url = URL.createObjectURL(file);
      const img = new Image();
      await new Promise((resolve, reject) => {
        img.onload = () => resolve(null);
        img.onerror = () => reject(new Error("could not load image"));
        img.src = url;
      });
      const size = 128;
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
      URL.revokeObjectURL(url);
      pAvatar = canvas.toDataURL("image/jpeg", 0.8).split(",")[1] ?? "";
    } catch (err) {
      error = String(err);
    }
  }

  async function saveProfile() {
    if (activeServerId === null) return;
    try {
      await invoke("set_profile", {
        server: activeServerId,
        name: pName.trim() || displayName,
        color: pColor,
        font: pFont,
        effect: pEffect,
        avatar: pAvatar,
      });
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
        // Brackets in the alt would break the `![alt](cid:…)` marker parse — strip them.
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
  // the group's own content-addressed blobs — never via untrusted innerHTML, so a peer's text
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
      if (!file) continue; // not in the index yet — retry when `files` updates
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
          // Bound the cache (each entry is a full decrypted blob) — FIFO-evict the oldest.
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
      refreshFiles(); // the file's chunks are now held locally — update its availability
    } catch (e) {
      error = String(e);
      if (downloads[key]) downloads[key].status = "failed";
    }
  }

  // The file info pane: click a file to inspect it (preview, availability, uploader, delete).
  let fileInfo = $state<UiFile | null>(null);
  let fileInfoAvail = $state<boolean | null>(null); // null = still checking
  let fileInfoPreview = $state<string>(""); // a data: URL for image/video/audio previews
  let fileInfoBusy = $state(false);
  let confirmDeleteCid = $state(""); // two-click delete confirm in the info pane
  // Tracked downloads keyed by file cid, for the Downloads tab + the file-info progress bar. Driven
  // by 'download-progress' events (per-chunk) from the actor. Only EXPLICIT downloads (the Download
  // button) are tracked here — background embed/preview fetches emit progress but create no entry.
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
  // Advisory eclipse hint for the active server (the node may be isolated — verify a member out of
  // band). Never gates anything; driven by 'eclipse-changed'. Reset when switching servers.
  let eclipseCaution = $state(false);

  async function openFileInfo(f: UiFile) {
    if (activeServerId === null) return;
    fileInfo = f;
    fileInfoAvail = null;
    fileInfoPreview = "";
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
        /* no preview — the availability line already explains why */
      }
    }
  }

  function closeFileInfo() {
    fileInfo = null;
    fileInfoPreview = "";
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

  async function send() {
    const text = draft.trim();
    if (!text || !cur || !cur.active || activeServerId === null) return;
    draft = "";
    try {
      await invoke("send_message", { server: activeServerId, channel: cur.active, text });
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
      await navigator.clipboard.writeText(cur.invite);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // Clipboard may be unavailable in the webview — the textarea allows manual copy.
    }
  }

  let mintingInvite = $state(false);
  // Mint a fresh single-use invite on demand (owner or admin — the backend gates on can_invite).
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
  function playNotify() {
    if (!soundOn) return;
    try {
      audioCtx = audioCtx ?? new AudioContext();
      const ctx = audioCtx;
      if (ctx.state === "suspended") void ctx.resume();
      const now = ctx.currentTime;
      [880, 1318.5].forEach((freq, i) => {
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

  onMount(() => {
    const subs: Promise<UnlistenFn>[] = [
      listen<{ server: number; channel: string }>("channel-updated", (e) => {
        const { server, channel } = e.payload;
        if (server === activeServerId && channel === cur?.active) {
          refresh();
          // You're looking at this channel — only chime if the window isn't focused.
          if (!document.hasFocus()) playNotify();
          return;
        }
        const s = servers.find((x) => x.id === server);
        if (s && s.channels.some((c) => c.id === channel)) {
          if (!s.unread.includes(channel)) s.unread.push(channel);
          if (server !== activeServerId) s.dot = true;
          playNotify();
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
      listen<{ server: number; online: string[] }>("connectivity-changed", (e) => {
        if (e.payload.server === activeServerId) {
          onlineMembers = new Set(e.payload.online);
          refreshFiles(); // a peer came/went — re-evaluate the availability hint (has_peers)
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
    // tabs; Ctrl/Cmd+K jumps to the chat composer.
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (menu) menu = null;
        else if (showEmoji) showEmoji = false;
        else if (fileInfo) closeFileInfo();
        else if (showWikiHelp) showWikiHelp = false;
        else if (showFeedback) showFeedback = false;
        else if (showServerSettings) showServerSettings = false;
        else if (showSettings) showSettings = false;
        return;
      }
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
        const tabs: Tab[] = ["chat", "files", "status", "wiki", "profile", "downloads"];
        if (e.key >= "1" && e.key <= "6") {
          e.preventDefault();
          if (activeServerId !== null) switchView(tabs[Number(e.key) - 1]);
        } else if (e.key.toLowerCase() === "k") {
          e.preventDefault();
          view = "chat";
          composerEl?.focus();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
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

<main>
  {#if eclipseCaution && activeServerId !== null && !locked}
    <div class="eclipse-banner" role="status">
      ⚠ You may be isolated from this server — few members are reachable. Verify a member out of band.
    </div>
  {/if}
  {#if locked}
    <div class="start">
      <h1>CatComs</h1>
      <p class="muted">
        Enter your passphrase to unlock your servers. On first run, the passphrase you
        choose here encrypts everything at rest — there is no recovery if you forget it.
      </p>
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
      {#if error}<p class="error">{error}</p>{/if}
      <button onclick={unlock} disabled={unlocking || !passphrase}>
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
            Reachable address so others can join over a network — your LAN IP (e.g.
            192.168.1.5), or a public IP / host:port if port-forwarded. Leave blank for
            same-machine only.
          </span>
          <input bind:value={advertise} placeholder="LAN/public IP (optional)" />
        </label>
        <label class="field">
          <span class="muted">
            Relay address (optional) — paste a relay node's multiaddr to be reachable over
            the internet with no port-forward (zero-config NAT traversal).
          </span>
          <input bind:value={relay} placeholder="/ip4/…/tcp/…/p2p/… (optional)" />
        </label>
        <label class="field">
          <span class="muted">
            Rendezvous address (optional) — paste a rendezvous node's multiaddr to register there,
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
      <textarea bind:value={joinInvite} rows="3" placeholder="paste invite here"></textarea>
      <button onclick={join} disabled={busy || !joinInvite.trim()}>Join</button>
      {#if error}<p class="muted" style="color:#ff6b6b">{error}</p>{/if}
    </div>
  {:else}
    <div class="app">
      <nav class="rail">
        {#each servers as s}
          <button
            class="server-icon"
            class:active={s.id === activeServerId}
            title={s.name}
            onclick={() => switchServer(s.id)}
            use:contextMenu={() => serverMenu(s)}
          >
            {s.name.slice(0, 1).toUpperCase()}
            {#if s.unread.length}
              <span class="rail-badge">{s.unread.length}</span>
            {:else if s.dot}
              <span class="rail-dot">●</span>
            {/if}
          </button>
        {/each}
        <button class="server-icon add" title="Add a server" onclick={() => (showAdd = true)}>+</button>
        <button class="server-icon feedback-btn" title="Send feedback (bug / feature request)" onclick={() => (showFeedback = true)}>💬</button>
        <button class="server-icon gear" title="Settings" onclick={() => (showSettings = true)}>⚙</button>
      </nav>

      <aside class="sidebar">
        <div class="server-head">
          <strong class="server-title" title={cur?.name}>{cur?.name ?? ""}</strong>
          <button class="ghost icon-btn" title="Server settings" onclick={() => openServerSettings()}>🛠</button>
        </div>
        {#if view === "chat"}
          <h3>Channels</h3>
          <ul class="channel-list">
            {#each cur?.channels ?? [] as c}
              <li>
                <button
                  class:active={c.id === cur?.active && view === "chat"}
                  onclick={() => { switchTo(c.id); view = "chat"; }}
                >
                  #{c.name}
                  {#if cur?.unread.includes(c.id)}<span class="dot">●</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
          <form onsubmit={(e) => { e.preventDefault(); addChannel(); }}>
            <input bind:value={newChannel} placeholder="join #channel…" />
          </form>
        {/if}

        <div class="roster">
          <h3>Members <span class="muted">({members}{#if members > 1} · {onlineCount} online{/if})</span></h3>
          {#if roster.length > 6}
            <input class="list-search" bind:value={rosterFilter} placeholder="Search members…" />
          {/if}
          <ul>
            {#each filteredRoster as m}
              {@const online = m.you || onlineMembers.has(m.fingerprint)}
              <li title={m.fingerprint} class:is-you={m.you} use:contextMenu={() => memberMenu(m)}>
                <span class="presence" class:online title={online ? "online" : "offline"}>●</span>
                {@render avatarTag(m.fingerprint)}
                {@render nameTag(m.fingerprint)}
                {#if roles[m.fingerprint] && roles[m.fingerprint] !== "member"}
                  <span class="role-badge {roles[m.fingerprint]}">{roles[m.fingerprint]}</span>
                {/if}
                {#if m.you}<span class="you-badge">you</span>{/if}
              </li>
            {:else}
              <li class="muted">No matching members.</li>
            {/each}
          </ul>
        </div>

        {#if canInvite || cur?.invite}
          <button class="ghost invite-quick" onclick={() => openServerSettings()}>＋ Invite someone</button>
        {/if}
      </aside>

      <section class="channel">
        <div class="tab-bar">
          <button class:active={view === "chat"} onclick={() => switchView("chat")}>Chat</button>
          <button class:active={view === "files"} onclick={() => switchView("files")}>
            Files {#if files.length}<span class="tab-count">{files.length}</span>{/if}
          </button>
          <button class:active={view === "status"} onclick={() => switchView("status")}>Status</button>
          <button class:active={view === "wiki"} onclick={() => switchView("wiki")}>Wiki</button>
          <button class:active={view === "profile"} onclick={() => switchView("profile")}>Profile</button>
          <button class:active={view === "downloads"} onclick={() => switchView("downloads")}>
            Downloads {#if activeDownloads}<span class="tab-count">{activeDownloads}</span>{/if}
          </button>
        </div>

        {#if view === "chat"}
          <h2>#{activeName()} <span class="muted">· {members} member(s)</span></h2>
          <ul class="messages" bind:this={messagesEl} use:richClicks>
            {#each messages as m}
              <li class:own={m.author === myFp} use:contextMenu={() => messageMenu(m)}>
                <span class="author">
                  {@render avatarTag(m.author)}
                  {@render nameTag(m.author)}
                  <span class="time" title={new Date(m.ts).toLocaleString()}>{fmtTime(m.ts)}</span>
                </span>
                <span class="text">{@html renderMessage(m.text)}</span>
              </li>
            {:else}
              <li class="muted">No messages yet — say hello.</li>
            {/each}
          </ul>
          <div class="composer-wrap">
            {#if showEmoji}
              <div class="emoji-picker">
                {#if Object.keys(emojiMap).length}
                  <div class="emoji-grid">
                    {#each Object.keys(emojiMap) as code}
                      <button class="emoji-pick" type="button" title={":" + code + ":"} onclick={() => insertEmoji(code)}>
                        {#if emojiUrls[code]}<img src={emojiUrls[code]} alt={code} />{:else}<span class="muted">:{code}:</span>{/if}
                      </button>
                    {/each}
                  </div>
                {:else}
                  <p class="muted small">No custom emoji yet — add some in ⚙ Settings → Emoji.</p>
                {/if}
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
              <button type="button" class="attach" title="Emoji" onclick={() => (showEmoji = !showEmoji)}>😀</button>
              <textarea
                bind:this={composerEl}
                bind:value={draft}
                rows="1"
                class="composer-input"
                placeholder={uploading ? "Uploading…" : dragOver ? "Drop to embed…" : "Message #" + activeName()}
                onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); } }}
              ></textarea>
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
          <div class="files-actions">
            <label class="upload">
              <span class="muted">{uploading ? "Uploading…" : "＋ Share a file here"}</span>
              <input type="file" disabled={uploading} onchange={(e) => { uploadFile(e.currentTarget.files); e.currentTarget.value = ''; }} />
            </label>
            <form class="new-folder" onsubmit={(e) => { e.preventDefault(); const n = newFolder.trim(); if (n) { enterFolder(n); newFolder = ''; } }}>
              <input bind:value={newFolder} placeholder="＋ new folder…" />
            </form>
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
              <li>
                <span class="status-head">
                  {@render avatarTag(s.author)}
                  {@render nameTag(s.author)}
                  <span class="time">{fmtTime(s.ts)}</span>
                </span>
                <span class="status-text">{@html renderMessage(s.text)}</span>
              </li>
            {:else}
              <li class="muted">No status posts yet.</li>
            {/each}
          </ul>
        {:else if view === "wiki"}
          <div class="wiki">
            <div class="wiki-pages">
              <div class="wiki-pages-head">
                <span class="muted">Pages</span>
                <button class="wiki-help-btn" title="Formatting help" onclick={() => (showWikiHelp = true)}>?</button>
              </div>
              {#if wikiPages.length > 6}
                <input class="list-search" bind:value={wikiFilter} placeholder="Search pages…" />
              {/if}
              {#each filteredWikiPages as p}
                <button class:active={p === activeWikiPage} onclick={() => openWikiPage(p)} use:contextMenu={() => wikiPageMenu(p)}>{p}</button>
              {:else}
                <span class="muted">{wikiFilter ? "No matching pages." : "No pages yet."}</span>
              {/each}
              <form onsubmit={(e) => { e.preventDefault(); createWikiPage(); }}>
                <input bind:value={newWikiPage} placeholder="+ new page" />
              </form>
            </div>
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
            <label class="field row">
              <span class="muted">Color</span>
              <input type="color" bind:value={pColor} />
            </label>
            <label class="field">
              <span class="muted">Font</span>
              <select bind:value={pFont}>
                <option value="system">System</option>
                <option value="serif">Serif</option>
                <option value="mono">Mono</option>
              </select>
            </label>
            <label class="field">
              <span class="muted">Effect</span>
              <select bind:value={pEffect}>
                <option value="none">None</option>
                <option value="rainbow">Rainbow wave</option>
                <option value="wave">Wave</option>
                <option value="pulse">Pulse</option>
              </select>
            </label>
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
        {:else if view === "downloads"}
          <h2>Downloads</h2>
          <div class="downloads-tab tab-pane">
            {#if downloadList.length === 0}
              <p class="muted">No downloads yet. Open a file and click <strong>↓ Download</strong> to start one.</p>
            {:else}
              <div class="dl-toolbar">
                <span class="muted small">{activeDownloads} active · {downloadList.length} total</span>
                <button class="ghost small" onclick={clearFinishedDownloads}>Clear finished</button>
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
        {#if error}<p class="muted" style="color:#ff6b6b">{error}</p>{/if}
      </section>
    </div>

    {#if showSettings}
      <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
      <div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) showSettings = false; }}>
        <div class="overlay-card">
          <header class="overlay-head">
            <h2>⚙ Settings</h2>
            <button class="ghost" onclick={() => (showSettings = false)}>✕</button>
          </header>
          <div class="overlay-body">
            <section class="set-section">
              <h3>Notifications</h3>
              <label class="toggle">
                <input type="checkbox" checked={soundOn} onchange={toggleSound} />
                <span>Play a sound for new messages</span>
              </label>
              <button class="ghost small" onclick={playNotify} disabled={!soundOn}>Test sound</button>
            </section>

            <section class="set-section">
              <h3>Network</h3>
              <p class="muted small">Reachability (LAN address / relay) is chosen when you found a server.</p>
              <label class="field">
                <span class="muted small">
                  Default rendezvous address — pre-filled when you found a server, so people can
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
            <h2>🛠 Server settings</h2>
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
                protocol-enforced. Admins can invite newcomers — the owner serializes each
                admission, so it completes when the owner is next online — and a demotion is
                replay-proof (a removed admin can't re-grant itself).
              </p>
              {#if myRole !== "owner"}
                <p class="muted small">Only the owner can change roles.</p>
              {/if}
            </section>

            {#if cur?.invite || canInvite}
              <section class="set-section">
                <h3>Invite someone</h3>
                <p class="muted small">Single-use — share it with one person to join this server. Generate a fresh
                  one anytime (after a restart, or once the last one was used).</p>
                {#if myRole === "admin"}
                  <p class="muted small">As an admin, the newcomer is admitted once the owner is next online.</p>
                {/if}
                {#if cur?.invite}
                  <textarea readonly rows="3" value={cur.invite}></textarea>
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
                <label class="upload-btn">
                  {uploading ? "…" : "Upload image"}
                  <input type="file" accept="image/*" disabled={uploading || !newEmojiCode.trim()}
                    onchange={(e) => { addEmoji(e.currentTarget.files); e.currentTarget.value = ''; }} />
                </label>
              </form>
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
                {#if !fileInfoPreview}
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
                  <span class="avail no">○ Not downloaded — fetched from a peer on demand</span>
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
              <span class="wikilink missing">red link</span> means the page doesn't exist yet — click it to create it.</p>
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
