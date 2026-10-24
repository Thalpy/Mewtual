<script lang="ts">
  import { pendingAcceptanceWarning, type PendingSends, type PendingResolution } from "./pending-sends.ts";
  let { pending, recovered, busy = false, ready, conversationName, activeLabel = null,
    onresolve, onuse, onremove }:
    { pending: PendingSends; recovered: PendingSends; busy?: boolean; ready: boolean;
      conversationName: (server: number, channel: string) => string; activeLabel?: string | null;
      onresolve: (token: string, action: PendingResolution) => Promise<void>;
      onuse: (token: string) => Promise<void>; onremove: (token: string) => Promise<void> } = $props();
  let confirmation = $state<{ token: string; action: PendingResolution | "remove" } | null>(null);
  let error = $state("");
  async function confirm() {
    const choice = confirmation;
    if (!choice || busy || !ready) return;
    error = "";
    try {
      if (choice.action === "remove") await onremove(choice.token);
      else await onresolve(choice.token, choice.action);
      confirmation = null;
    } catch (failure) { error = String(failure); }
  }
  async function copy(text: string) {
    try { await navigator.clipboard.writeText(text); error = ""; }
    catch { error = "Copy was unavailable. Select the text below and copy it manually."; }
  }
  async function use(token: string) {
    try { await onuse(token); error = ""; }
    catch (failure) { error = String(failure); }
  }
</script>

<section aria-label="Vault pending messages">
  <h2>Pending messages</h2>
  <p>These requests belong to this vault, including conversations you have left. Resolving a request does not send a replacement message.</p>
  {#if !ready}<p role="status">Unlock and load the vault before resolving messages.</p>{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#if !Object.keys(pending).length}<p>No pending requests.</p>{/if}
  {#each Object.values(pending) as item (item.token)}
    <article data-pending-token={item.token}>
      <h3>{conversationName(item.server, item.channel)}</h3>
      <textarea aria-label="Pending message text" readonly value={item.text}></textarea>
      {#if item.replyTo}<p>Original reply: {item.replyTo}</p>{/if}
      {#if item.retryBlock}<p>Automatic retry paused: {item.retryBlock === "context_changed" ? "Conversation changed" : item.retryBlock === "conflict" ? "Retry identity conflict" : "Request cannot be accepted"}.</p>{/if}
      <p>{pendingAcceptanceWarning(item)}</p>
      {#if confirmation?.token === item.token && confirmation.action !== "remove"}
        <p>{confirmation.action === "recover" ? "Keep the full text in saved drafts and stop retrying this request?" : "Stop retrying and remove the stored request and its text after the vault saves? Recover to a saved draft first if you want to keep this copy."}</p>
        <button type="button" disabled={busy || !ready} onclick={confirm}>{confirmation.action === "recover" ? "Confirm recovery" : "Confirm stop retrying"}</button>
        <button type="button" disabled={busy} onclick={() => confirmation = null}>Keep pending</button>
      {:else}
        <button type="button" disabled={busy || !ready} onclick={() => confirmation = { token: item.token, action: "recover" }}>Recover to saved draft</button>
        <button type="button" disabled={busy || !ready} onclick={() => confirmation = { token: item.token, action: "cancel" }}>Stop retrying</button>
      {/if}
    </article>
  {/each}
  <h2>Recovered drafts</h2>
  <p>These saved copies never retry or publish. Review the conversation before sending their text again.</p>
  {#if !Object.keys(recovered).length}<p>No recovered drafts.</p>{/if}
  {#each Object.values(recovered) as item (item.token)}
    <article data-recovered-token={item.token}>
      <h3>{conversationName(item.server, item.channel)}</h3>
      <textarea aria-label="Recovered draft text" readonly value={item.text}></textarea>
      {#if item.replyTo}<p>Original reply: {item.replyTo}</p>{/if}
      <p>{pendingAcceptanceWarning(item)}</p>
      <button type="button" onclick={() => copy(item.text)}>Copy text</button>
      {#if activeLabel}<button type="button" disabled={busy || !ready} onclick={() => use(item.token)}>Use in {activeLabel} draft</button>{/if}
      {#if confirmation?.token === item.token && confirmation.action === "remove"}
        <p>Remove this saved copy? Copy any text you still need first.</p>
        <button type="button" disabled={busy || !ready} onclick={confirm}>Confirm remove draft</button>
        <button type="button" disabled={busy} onclick={() => confirmation = null}>Keep draft</button>
      {:else}
        <button type="button" disabled={busy || !ready} onclick={() => confirmation = { token: item.token, action: "remove" }}>Remove saved draft</button>
      {/if}
    </article>
  {/each}
</section>

<style>
  article { border: 1px solid var(--border, #777); border-radius: .5rem; padding: 1rem; margin: 1rem 0; }
  textarea { display: block; box-sizing: border-box; width: 100%; min-height: 6rem; resize: vertical; }
  button { margin: .25rem .5rem .25rem 0; }
  p { overflow-wrap: anywhere; }
</style>
