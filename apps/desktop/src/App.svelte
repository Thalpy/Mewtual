<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { renderMessage, renderWiki } from "./render";

  type Msg = { author: string; text: string; ts: number };
  type Channel = { id: string; name: string };
  type Member = { fingerprint: string; you: boolean };
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string; avatar: string };
  type UiFile = { name: string; size: number; mime: string; cid: string; author: string; path: string };
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
  let showSettings = $state(false); // the Settings overlay
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
  let joinInvite = $state(""); // pasted invite (joiner)
  let copied = $state(false);
  let newChannel = $state("");

  let messages = $state<Msg[]>([]);
  let messagesEl = $state<HTMLUListElement | undefined>(undefined);
  let draft = $state("");
  let members = $state(1);
  let roster = $state<Member[]>([]);
  let profiles = $state<Record<string, Prof>>({});
  let files = $state<UiFile[]>([]);
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
  type Tab = "chat" | "files" | "status" | "wiki" | "profile";
  let view = $state<Tab>("chat");
  let wikiPages = $state<string[]>([]);
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

  function activeName(): string {
    return cur?.channels.find((c) => c.id === cur?.active)?.name ?? "";
  }
  function nameOf(fp: string): string {
    return profiles[fp]?.name?.trim() || fp;
  }
  function fontClass(font: string): string {
    return font === "serif" ? "font-serif" : font === "mono" ? "font-mono" : "";
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

  // Resolve inline media embeds + custom emoji whenever content or the file index changes.
  $effect(() => {
    void messages;
    void statuses;
    void files;
    void emojiUrls;
    resolveMedia(messagesEl);
    resolveEmoji(messagesEl);
    resolveMedia(statusEl);
    resolveEmoji(statusEl);
  });

  // Resolve embeds + emoji + mark missing [[links]] in the rendered wiki page (read mode).
  $effect(() => {
    void wikiBody;
    void wikiPages;
    void wikiEdit;
    void files;
    void emojiUrls;
    if (!wikiEdit) {
      resolveMedia(wikiEl);
      resolveEmoji(wikiEl);
      resolveWikiLinks(wikiEl);
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
      const r = await invoke<Found>("found_server", { displayName, advertise, relay });
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
    if (activeServerId === null) return;
    try {
      roster = await invoke<Member[]>("get_members", { server: activeServerId });
      members = roster.length;
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
      files = await invoke<UiFile[]>("get_files", { server: activeServerId });
    } catch (e) {
      error = String(e);
    }
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
    view = v;
    if (v === "wiki") refreshWiki();
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
    node.addEventListener("click", h);
    return { destroy: () => node.removeEventListener("click", h) };
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
    try {
      const base64 = await invoke<string>("download_file", { server: activeServerId, cid: f.cid });
      const a = document.createElement("a");
      a.href = `data:${f.mime || "application/octet-stream"};base64,${base64}`;
      a.download = f.name;
      document.body.appendChild(a);
      a.click();
      a.remove();
    } catch (e) {
      error = String(e);
    }
  }

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
        if (e.payload.server === activeServerId) refreshMembers();
      }),
      listen<{ server: number }>("profiles-updated", (e) => {
        if (e.payload.server === activeServerId) refreshProfiles();
      }),
      listen<{ server: number }>("files-updated", (e) => {
        if (e.payload.server === activeServerId) refreshFiles();
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
      listen<{ server: number }>("server-closed", (e) => {
        servers = servers.filter((s) => s.id !== e.payload.server);
        if (activeServerId === e.payload.server) {
          activeServerId = servers.length ? servers[0].id : null;
          if (activeServerId !== null) switchServer(activeServerId);
        }
      }),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
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
          >
            {s.name.slice(0, 1).toUpperCase()}
            {#if s.dot}<span class="rail-dot">●</span>{/if}
          </button>
        {/each}
        <button class="server-icon add" title="Add a server" onclick={() => (showAdd = true)}>+</button>
        <button class="server-icon gear" title="Settings" onclick={() => (showSettings = true)}>⚙</button>
      </nav>

      <aside class="sidebar">
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

        <div class="roster">
          <h3>Members <span class="muted">({members})</span></h3>
          <ul>
            {#each roster as m}
              <li title={m.fingerprint}>
                {@render avatarTag(m.fingerprint)}
                {@render nameTag(m.fingerprint)}
                {#if roles[m.fingerprint] && roles[m.fingerprint] !== "member"}
                  <span class="role-badge {roles[m.fingerprint]}">{roles[m.fingerprint]}</span>
                {/if}
                {#if m.you}<span class="you-badge">you</span>{/if}
              </li>
            {/each}
          </ul>
        </div>

        {#if cur?.invite}
          <button class="ghost invite-quick" onclick={() => (showSettings = true)}>＋ Invite someone</button>
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
        </div>

        {#if view === "chat"}
          <h2>#{activeName()} <span class="muted">· {members} member(s)</span></h2>
          <ul class="messages" bind:this={messagesEl} use:richClicks>
            {#each messages as m}
              <li class:own={m.author === myFp}>
                <span class="author">
                  {@render avatarTag(m.author)}
                  {@render nameTag(m.author)}
                  <span class="time">{fmtTime(m.ts)}</span>
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
              <input bind:value={draft} placeholder={uploading ? "Uploading…" : dragOver ? "Drop to embed…" : "Message #" + activeName()} />
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
              <li>
                <button class="file-name" title={"from " + nameOf(f.author)} onclick={() => downloadFile(f)}>
                  ↓ {f.name}
                </button>
                <span class="muted file-size">{fmtSize(f.size)} · {nameOf(f.author)}</span>
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
              {#each wikiPages as p}
                <button class:active={p === activeWikiPage} onclick={() => openWikiPage(p)}>{p}</button>
              {:else}
                <span class="muted">No pages yet.</span>
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
              <h3>Server</h3>
              <p>{cur?.name ?? "—"} <span class="role-badge {myRole}">{myRole}</span></p>
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
                    {/if}
                  </li>
                {/each}
              </ul>
              <p class="muted small">
                Roles are a display + policy aid, not a hard security control yet — a modified
                client can still set its own role. The owner is the founder (the MLS committer).
              </p>
              {#if myRole !== "owner"}
                <p class="muted small">Only the owner can change roles.</p>
              {/if}
            </section>

            {#if cur?.invite}
              <section class="set-section">
                <h3>Invite someone</h3>
                <p class="muted small">Single-use — share it with one person to join this server.</p>
                <textarea readonly rows="3" value={cur.invite}></textarea>
                <button onclick={copyInvite}>{copied ? "Copied!" : "Copy invite"}</button>
              </section>
            {/if}

            <section class="set-section">
              <h3>Notifications</h3>
              <label class="toggle">
                <input type="checkbox" checked={soundOn} onchange={toggleSound} />
                <span>Play a sound for new messages</span>
              </label>
              <button class="ghost small" onclick={playNotify} disabled={!soundOn}>Test sound</button>
            </section>

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

            <section class="set-section">
              <h3>Network</h3>
              <p class="muted small">Reachability (LAN address / relay) is chosen when you found a server.</p>
            </section>

            {#if activeServerId !== null}
              <section class="set-section danger">
                <button class="ghost leave" onclick={() => { const id = activeServerId; showSettings = false; if (id !== null) leaveServer(id); }}>
                  Leave this server
                </button>
              </section>
            {/if}
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
  {/if}
</main>
