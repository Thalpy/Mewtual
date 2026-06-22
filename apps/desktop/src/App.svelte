<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Msg = { author: string; text: string; ts: number };
  type Channel = { id: string; name: string };
  type Member = { fingerprint: string; you: boolean };
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string; avatar: string };
  type UiFile = { name: string; size: number; mime: string; cid: string; author: string };
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
  let statuses = $state<Msg[]>([]);
  let statusDraft = $state("");

  // Wiki (main-pane view toggles between chat and wiki).
  let view = $state<"chat" | "wiki">("chat");
  let wikiPages = $state<string[]>([]);
  let activeWikiPage = $state("");
  let wikiBody = $state("");
  let newWikiPage = $state("");
  let wikiDirty = $state(false); // unsaved edits in the open page (avoid clobbering on live updates)

  // Profile editor.
  let pName = $state("");
  let pColor = $state("#4f8cff");
  let pFont = $state("system");
  let pEffect = $state("none");
  let pAvatar = $state("");

  let cur = $derived(servers.find((s) => s.id === activeServerId) ?? null);
  let myFp = $derived(roster.find((r) => r.you)?.fingerprint ?? "");

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
    // Each server has its own wiki; reset the wiki view to the new server's.
    view = "chat";
    activeWikiPage = "";
    wikiBody = "";
    wikiDirty = false;
    await Promise.all([
      refresh(),
      refreshMembers(),
      refreshProfiles(),
      refreshFiles(),
      refreshStatuses(),
      refreshInvite(),
    ]);
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

  function switchView(v: "chat" | "wiki") {
    view = v;
    if (v === "wiki") refreshWiki();
  }
  async function refreshWiki() {
    if (activeServerId === null) return;
    try {
      wikiPages = await invoke<string[]>("get_wiki_pages", { server: activeServerId });
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
    } catch (e) {
      error = String(e);
    }
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

  async function uploadFile(fileList: FileList | null) {
    const file = fileList?.[0];
    if (!file || activeServerId === null) return;
    uploading = true;
    try {
      const base64 = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onerror = () => reject(new Error("could not read file"));
        reader.onload = () => {
          const r = reader.result;
          resolve(typeof r === "string" ? (r.split(",")[1] ?? "") : "");
        };
        reader.readAsDataURL(file);
      });
      await invoke("add_file", {
        server: activeServerId,
        name: file.name,
        mime: file.type || "application/octet-stream",
        data: base64,
      });
      await refreshFiles();
    } catch (e) {
      error = String(e);
    } finally {
      uploading = false;
    }
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

  onMount(() => {
    const subs: Promise<UnlistenFn>[] = [
      listen<{ server: number; channel: string }>("channel-updated", (e) => {
        const { server, channel } = e.payload;
        if (server === activeServerId && channel === cur?.active) {
          refresh();
          return;
        }
        const s = servers.find((x) => x.id === server);
        if (s && s.channels.some((c) => c.id === channel)) {
          if (!s.unread.includes(channel)) s.unread.push(channel);
          if (server !== activeServerId) s.dot = true;
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
      </nav>

      <aside class="sidebar">
        <h3>Channels</h3>
        <ul class="channel-list">
          {#each cur?.channels ?? [] as c}
            <li>
              <button class:active={c.id === cur?.active} onclick={() => switchTo(c.id)}>
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
                {#if m.you}<span class="you-badge">you</span>{/if}
              </li>
            {/each}
          </ul>
        </div>

        <details class="profile-editor">
          <summary>Your profile</summary>
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
        </details>

        <details class="files-panel">
          <summary>Files <span class="muted">({files.length})</span></summary>
          <label class="upload">
            <span class="muted">{uploading ? "Uploading…" : "Share a file"}</span>
            <input type="file" disabled={uploading} onchange={(e) => uploadFile(e.currentTarget.files)} />
          </label>
          <ul class="file-list">
            {#each files as f}
              <li>
                <button class="file-name" title={"from " + nameOf(f.author)} onclick={() => downloadFile(f)}>
                  ↓ {f.name}
                </button>
                <span class="muted file-size">{fmtSize(f.size)}</span>
              </li>
            {:else}
              <li class="muted">No files shared yet.</li>
            {/each}
          </ul>
        </details>

        <details class="status-panel">
          <summary>Status <span class="muted">({statuses.length})</span></summary>
          <form onsubmit={(e) => { e.preventDefault(); postStatus(); }}>
            <input bind:value={statusDraft} placeholder="Post a status…" />
          </form>
          <ul class="status-list">
            {#each statuses as s}
              <li>
                <span class="status-head">
                  {@render nameTag(s.author)}
                  <span class="time">{fmtTime(s.ts)}</span>
                </span>
                <span class="status-text">{s.text}</span>
              </li>
            {:else}
              <li class="muted">No status posts yet.</li>
            {/each}
          </ul>
        </details>

        {#if cur?.invite}
          <details>
            <summary>Invite someone</summary>
            <p class="muted">Single-use — open a second window and paste it:</p>
            <textarea readonly rows="3" value={cur.invite}></textarea>
            <button onclick={copyInvite}>{copied ? "Copied!" : "Copy invite"}</button>
          </details>
        {/if}

        {#if activeServerId !== null}
          <button class="ghost leave" onclick={() => activeServerId !== null && leaveServer(activeServerId)}>
            Leave server
          </button>
        {/if}
      </aside>

      <section class="channel">
        <div class="view-tabs">
          <button class:active={view === "chat"} onclick={() => switchView("chat")}>Chat</button>
          <button class:active={view === "wiki"} onclick={() => switchView("wiki")}>Wiki</button>
        </div>

        {#if view === "chat"}
          <h2>#{activeName()} <span class="muted">· {members} member(s)</span></h2>
          <ul class="messages" bind:this={messagesEl}>
            {#each messages as m}
              <li class:own={m.author === myFp}>
                <span class="author">
                  {@render avatarTag(m.author)}
                  {@render nameTag(m.author)}
                  <span class="time">{fmtTime(m.ts)}</span>
                </span>
                <span class="text">{m.text}</span>
              </li>
            {:else}
              <li class="muted">No messages yet — say hello.</li>
            {/each}
          </ul>
          <form class="composer" onsubmit={(e) => { e.preventDefault(); send(); }}>
            <input bind:value={draft} placeholder={"Message #" + activeName()} />
            <button type="submit">Send</button>
          </form>
        {:else}
          <div class="wiki">
            <div class="wiki-pages">
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
                <h2>{activeWikiPage} {#if wikiDirty}<span class="muted">· unsaved</span>{/if}</h2>
                <textarea bind:value={wikiBody} oninput={() => (wikiDirty = true)} rows="16"
                  placeholder="Write this page…"></textarea>
                <button onclick={saveWikiPage} disabled={!wikiDirty}>Save page</button>
              </div>
            {:else}
              <p class="muted wiki-empty">Select a page on the left, or create one.</p>
            {/if}
          </div>
        {/if}
        {#if error}<p class="muted" style="color:#ff6b6b">{error}</p>{/if}
      </section>
    </div>
  {/if}
</main>
