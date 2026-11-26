<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { readNativeAudit, type NativeAuditPage } from "./native-text-history.ts";

  let { server, channel, context }: { server: number; channel: string; context: string } = $props();
  let dialog: HTMLDialogElement;
  let target = $state<string | null>(null);
  let after = $state<string | null>(null);
  let listAfter: string | null = null;
  let page = $state<NativeAuditPage | null>(null);
  let loading = $state(false);
  let failure = $state("");
  let request = 0;

  async function load(action: string | null, cursor: string | null) {
    const original = ++request;
    const current = () => original === request && dialog.open;
    target = action;
    after = cursor;
    page = null;
    loading = true;
    failure = "";
    try {
      const result = await readNativeAudit(invoke, { server, channel, context }, action, cursor, current);
      if (current()) page = result;
    } catch {
      if (current()) failure = "Could not read saved history. Try again.";
    } finally {
      if (current()) loading = false;
    }
  }

  function clear() {
    request++;
    page = null;
    loading = false;
    failure = "";
  }

  onDestroy(clear);

  const labels = { create: "Original message", edit: "Edit", delete: "Deletion", join: "Combined revisions" };
</script>

<button type="button" class="ghost small" onclick={() => {
  dialog.showModal();
  void load(null, null);
}}>Chat history</button>

<dialog bind:this={dialog} onclose={clear} aria-labelledby="native-audit-title">
  <header>
    <h2 id="native-audit-title">Chat history</h2>
    <button type="button" class="ghost small" onclick={() => { clear(); dialog.close(); }}>Close</button>
  </header>
  <p>Messages and changes saved in this server’s received history on this device.</p>
  {#if after}<p>Return to the first page to include any new entries.</p>{/if}
  {#if target}
    <button type="button" class="ghost small" onclick={() => void load(null, listAfter)}>Back to messages</button>
    <p>Entries are grouped by message. They may appear out of order.</p>
  {/if}
  <div aria-busy={loading}>
    {#if loading}
      <p role="status">Loading history…</p>
    {:else if failure}
      <p role="status">{failure}</p>
      <button type="button" class="ghost small" onclick={() => void load(target, after)}>Retry</button>
    {:else if page?.items === null}
      <p>No received history is available on this device.</p>
    {:else if page?.items?.length === 0}
      <p>No saved entries on this page.</p>
    {:else if page}
      <ul>
        {#each page.items ?? [] as entry (entry.action)}
          <li>
            {#if target}
              <strong>{labels[entry.body.kind]}</strong>
              {#if "text" in entry.body}<p class="message">{entry.body.text}</p>{/if}
              <details>
                <summary>Signed record details</summary>
                <dl>
                  <dt>Device key</dt><dd>{entry.signer}</dd>
                  <dt>Membership record</dt><dd>{entry.admission}</dd>
                  <dt>Message action</dt><dd>{entry.action}</dd>
                  <dt>Original digest</dt><dd>{entry.original_digest}</dd>
                  {#if entry.parents.length}
                    <dt>Previous actions</dt><dd>{entry.parents.join(", ")}</dd>
                  {/if}
                </dl>
              </details>
            {:else if entry.body.kind === "create"}
              <button type="button" class="ghost small" onclick={() => {
                listAfter = after;
                void load(entry.action, null);
              }}>View changes</button>
              <p class="message">{entry.body.text}</p>
            {/if}
          </li>
        {/each}
      </ul>
      {#if page.next}
        {@const next = page.next}
        <button type="button" class="ghost small" onclick={() => void load(target, next)}>Next page</button>
      {/if}
      {#if after}
        <button type="button" class="ghost small" onclick={() => void load(target, null)}>First page</button>
      {/if}
    {/if}
  </div>
</dialog>

<style>
  dialog {
    box-sizing: border-box;
    width: min(38rem, calc(100% - 2rem));
    max-height: calc(100% - 2rem);
    padding: 1rem;
    border: 1px solid var(--line, #3b3746);
    border-radius: .75rem;
    color: var(--text, #eeeaf4);
    background: var(--panel, #211d2c);
  }
  dialog::backdrop { background: #100d17b3; }
  header { display: flex; align-items: center; justify-content: space-between; gap: 1rem; }
  h2 { margin: 0; }
  ul { list-style: none; margin: 1rem 0; padding: 0; }
  li { padding: .7rem 0; border-top: 1px solid var(--line, #3b3746); }
  .message { white-space: pre-wrap; overflow-wrap: anywhere; }
  dd { margin: 0 0 .45rem; overflow-wrap: anywhere; font-family: monospace; }
</style>
