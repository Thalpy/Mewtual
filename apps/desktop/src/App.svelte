<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  type Msg = { author: string; text: string };

  let mode = $state<"start" | "channel">("start");
  let busy = $state(false);
  let error = $state("");
  let displayName = $state("me");
  let invite = $state(""); // the invite to share (founder)
  let joinInvite = $state(""); // pasted invite (joiner)
  let copied = $state(false);
  let messages = $state<Msg[]>([]);
  let draft = $state("");
  let members = $state(1);

  async function found() {
    busy = true;
    error = "";
    try {
      await invoke("found_server", { displayName });
      invite = (await invoke<string | null>("get_invite")) ?? "";
      mode = "channel";
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
      await invoke("join_server", { inviteHex: joinInvite.trim(), displayName });
      mode = "channel";
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
      listen("channel-updated", () => refresh()),
      listen<number>("members-changed", (e) => (members = e.payload)),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
  });
</script>

<main>
  {#if mode === "start"}
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
    <textarea
      bind:value={joinInvite}
      rows="3"
      placeholder="paste invite here"
    ></textarea>
    <button onclick={join} disabled={busy || !joinInvite.trim()}>Join</button>
  {:else}
    <h2>#general <span class="muted">· {members} member(s)</span></h2>

    {#if invite}
      <details>
        <summary>Invite someone</summary>
        <p class="muted">Share this single-use invite, then open a second CatComs window and paste it:</p>
        <textarea readonly rows="3" value={invite}></textarea>
        <button onclick={copyInvite}>{copied ? "Copied!" : "Copy invite"}</button>
      </details>
    {/if}

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
