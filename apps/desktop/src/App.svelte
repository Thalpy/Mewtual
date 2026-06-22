<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Msg = { author: string; text: string };
  type Channel = { id: string; name: string };

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

  function activeName(): string {
    return channels.find((c) => c.id === active)?.name ?? "";
  }

  async function found() {
    busy = true;
    error = "";
    try {
      const id = await invoke<string>("found_server", { displayName });
      invite = (await invoke<string | null>("get_invite")) ?? "";
      channels = [{ id, name: "general" }];
      active = id;
      mode = "app";
      await refresh();
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
      mode = "app";
      await refresh();
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
      listen<number>("members-changed", (e) => (members = e.payload)),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
  });
</script>

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
            <li><b>{m.author}:</b> {m.text}</li>
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
