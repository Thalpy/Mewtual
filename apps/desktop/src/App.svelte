<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Msg = { author: string; text: string };
  type Channel = { id: string; name: string };
  type Member = { fingerprint: string; you: boolean };
  type Prof = { fingerprint: string; name: string; color: string; font: string; effect: string };

  let mode = $state<"start" | "app">("start");
  let busy = $state(false);
  let error = $state("");
  let displayName = $state("me");
  let invite = $state(""); // the invite to share (founder)
  let joinInvite = $state(""); // pasted invite (joiner)
  let copied = $state(false);

  let channels = $state<Channel[]>([]);
  let active = $state(""); // active channel id
  let unread = $state<Set<string>>(new Set());
  let newChannel = $state("");

  let messages = $state<Msg[]>([]);
  let draft = $state("");
  let members = $state(1);
  let roster = $state<Member[]>([]);
  let profiles = $state<Record<string, Prof>>({});

  // The local device's fingerprint (the roster entry flagged `you`).
  let myFp = $derived(roster.find((r) => r.you)?.fingerprint ?? "");

  // Profile editor.
  let pName = $state("");
  let pColor = $state("#4f8cff");
  let pFont = $state("system");
  let pEffect = $state("none");

  function activeName(): string {
    return channels.find((c) => c.id === active)?.name ?? "";
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

  async function found() {
    busy = true;
    error = "";
    try {
      const id = await invoke<string>("found_server", { displayName });
      invite = (await invoke<string | null>("get_invite")) ?? "";
      channels = [{ id, name: "general" }];
      active = id;
      pName = displayName;
      mode = "app";
      await refresh();
      await refreshMembers();
      await refreshProfiles();
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
      const id = await invoke<string>("join_server", {
        inviteHex: joinInvite.trim(),
        displayName,
      });
      channels = [{ id, name: "general" }];
      active = id;
      pName = displayName;
      mode = "app";
      await refresh();
      await refreshMembers();
      await refreshProfiles();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function addChannel() {
    const name = newChannel.trim().replace(/^#/, "");
    if (!name) return;
    newChannel = "";
    try {
      const id = await invoke<string>("open_channel", { name });
      if (!channels.some((c) => c.id === id)) channels = [...channels, { id, name }];
      switchTo(id);
    } catch (e) {
      error = String(e);
    }
  }

  function switchTo(id: string) {
    active = id;
    if (unread.has(id)) {
      unread.delete(id);
      unread = new Set(unread);
    }
    refresh();
  }

  async function refresh() {
    if (!active) return;
    try {
      messages = await invoke<Msg[]>("get_messages", { channel: active });
    } catch (e) {
      error = String(e);
    }
  }

  async function refreshMembers() {
    try {
      roster = await invoke<Member[]>("get_members");
    } catch (e) {
      error = String(e);
    }
  }

  async function refreshProfiles() {
    try {
      const list = await invoke<Prof[]>("get_profiles");
      const map: Record<string, Prof> = {};
      for (const p of list) map[p.fingerprint] = p;
      profiles = map;
      // Seed the editor from my own stored profile (once it exists).
      const mine = profiles[myFp];
      if (mine) {
        if (mine.name) pName = mine.name;
        if (mine.color) pColor = mine.color;
        if (mine.font) pFont = mine.font;
        if (mine.effect) pEffect = mine.effect;
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function saveProfile() {
    try {
      await invoke("set_profile", {
        name: pName.trim() || displayName,
        color: pColor,
        font: pFont,
        effect: pEffect,
      });
      await refreshProfiles();
    } catch (e) {
      error = String(e);
    }
  }

  async function send() {
    const text = draft.trim();
    if (!text || !active) return;
    draft = "";
    try {
      await invoke("send_message", { channel: active, text });
    } catch (e) {
      error = String(e);
    }
  }

  async function copyInvite() {
    try {
      await navigator.clipboard.writeText(invite);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      // Clipboard may be unavailable in the webview — the textarea allows manual copy.
    }
  }

  onMount(() => {
    const subs: Promise<UnlistenFn>[] = [
      listen<string>("channel-updated", (e) => {
        const id = e.payload;
        if (id === active) {
          refresh();
        } else if (channels.some((c) => c.id === id)) {
          unread.add(id);
          unread = new Set(unread);
        }
      }),
      listen<number>("members-changed", (e) => {
        members = e.payload;
        refreshMembers();
      }),
      listen("profiles-updated", () => refreshProfiles()),
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

<main>
  {#if mode === "start"}
    <div class="start">
      <h1>CatComs</h1>
      <label class="field">
        <span class="muted">Display name</span>
        <input bind:value={displayName} placeholder="display name" />
      </label>
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
      <aside class="sidebar">
        <h3>Channels</h3>
        <ul class="channel-list">
          {#each channels as c}
            <li>
              <button class:active={c.id === active} onclick={() => switchTo(c.id)}>
                #{c.name}
                {#if unread.has(c.id)}<span class="dot">●</span>{/if}
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
          <p class="preview">
            Preview: {@render styledName(pName || displayName, pColor, pFont, pEffect)}
          </p>
          <button onclick={saveProfile}>Save profile</button>
        </details>

        {#if invite}
          <details>
            <summary>Invite someone</summary>
            <p class="muted">Single-use — open a second window and paste it:</p>
            <textarea readonly rows="3" value={invite}></textarea>
            <button onclick={copyInvite}>{copied ? "Copied!" : "Copy invite"}</button>
          </details>
        {/if}
      </aside>

      <section class="channel">
        <h2>#{activeName()} <span class="muted">· {members} member(s)</span></h2>
        <ul class="messages">
          {#each messages as m}
            <li class:own={m.author === myFp}>
              <span class="author">{@render nameTag(m.author)}</span>
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
        {#if error}<p class="muted" style="color:#ff6b6b">{error}</p>{/if}
      </section>
    </div>
  {/if}
</main>
