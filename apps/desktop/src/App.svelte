<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Msg = { author: string; text: string };

  let founded = $state(false);
  let busy = $state(false);
  let error = $state("");
  let displayName = $state("me");
  let messages = $state<Msg[]>([]);
  let draft = $state("");
  let members = $state(1);

  async function found() {
    busy = true;
    error = "";
    try {
      await invoke("found_server", { displayName });
      founded = true;
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function refresh() {
    try {
      messages = await invoke<Msg[]>("get_messages");
    } catch (e) {
      error = String(e);
    }
  }

  async function send() {
    const text = draft.trim();
    if (!text) return;
    draft = "";
    try {
      await invoke("send_message", { text });
    } catch (e) {
      error = String(e);
    }
  }

  onMount(() => {
    const subs: Promise<UnlistenFn>[] = [
      listen("channel-updated", () => refresh()),
      listen<number>("members-changed", (e) => (members = e.payload)),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
  });
</script>

<main>
  {#if !founded}
    <h1>CatComs</h1>
    <p class="muted">Found a server to start an end-to-end-encrypted channel.</p>
    <input bind:value={displayName} placeholder="display name" />
    <button onclick={found} disabled={busy}>
      {busy ? "Founding…" : "Found a server"}
    </button>
  {:else}
    <h2>#general <span class="muted">· {members} member(s)</span></h2>
    <ul class="messages">
      {#each messages as m}
        <li><b>{m.author}:</b> {m.text}</li>
      {:else}
        <li class="muted">No messages yet — say hello.</li>
      {/each}
    </ul>
    <form
      class="composer"
      onsubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <input bind:value={draft} placeholder="Type a message…" />
      <button type="submit">Send</button>
    </form>
  {/if}

  {#if error}
    <p class="muted" style="color:#ff6b6b">{error}</p>
  {/if}
</main>
