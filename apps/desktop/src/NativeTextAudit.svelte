<script lang="ts">
  import { onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    readNativeAudit, readNativeHeldPosts, readNativeOriginalPage,
    type NativeAuditPage, type NativeHeldPage, type NativeOriginalPage,
  } from "./native-text-history.ts";

  let { server, channel, context }: { server: number; channel: string; context: string } = $props();
  let dialog: HTMLDialogElement;
  let target = $state<string | null>(null);
  let after = $state<string | null>(null);
  let listAfter: string | null = null;
  let page = $state<NativeAuditPage | null>(null);
  let localPage = $state<NativeOriginalPage | null>(null);
  let heldPage = $state<NativeHeldPage | null>(null);
  let heldAfter = $state<string | null>(null);
  let localCursor = $state<{ next: string; end: string | null }>({ next: "0", end: null });
  let view = $state<"received" | "local" | "held">("received");
  let loading = $state(false);
  let failure = $state("");
  let request = 0;

  async function load(action: string | null, cursor: string | null) {
    const original = ++request;
    const current = () => original === request && dialog.open;
    view = "received";
    localPage = null;
    heldPage = null;
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

  async function loadLocal(cursor: { next: string; end: string | null } = { next: "0", end: null }) {
    const original = ++request;
    const current = () => original === request && dialog.open;
    view = "local";
    localCursor = cursor;
    page = null;
    localPage = null;
    heldPage = null;
    loading = true;
    failure = "";
    try {
      const result = await readNativeOriginalPage(invoke, { server, channel, context }, current, cursor);
      if (current()) localPage = result;
    } catch {
      if (current()) failure = "Could not read messages saved on this device. Try again.";
    } finally {
      if (current()) loading = false;
    }
  }

  async function loadHeld(cursor: string | null = null) {
    const original = ++request;
    const current = () => original === request && dialog.open;
    view = "held";
    heldAfter = cursor;
    page = null;
    localPage = null;
    heldPage = null;
    loading = true;
    failure = "";
    try {
      const result = await readNativeHeldPosts(invoke, { server, channel, context }, cursor, current);
      if (current()) heldPage = result;
    } catch {
      if (current()) failure = "Could not read held posts.";
    } finally {
      if (current()) loading = false;
    }
  }

  function clear() {
    request++;
    page = null;
    localPage = null;
    heldPage = null;
    heldAfter = null;
    loading = false;
    failure = "";
  }

  onDestroy(clear);

  const labels = { create: "Original message", edit: "Edit", delete: "Deletion", join: "Combined revisions" };
  const localStatus = { pending: "Saved here", submitted: "Submitted", held: "Held on this device" };
  const reviewChoice = { approve: "Approved", reject: "Rejected", reopen: "Reopened" };
</script>

<button type="button" class="ghost small" onclick={() => {
  dialog.showModal();
  void load(null, null);
}}>Chat history</button>

<dialog bind:this={dialog} onclose={() => { if (!dialog.open) clear(); }} aria-labelledby="native-audit-title">
  <header>
    <h2 id="native-audit-title">Chat history</h2>
    <button type="button" class="ghost small" onclick={() => { clear(); dialog.close(); }}>Close</button>
  </header>
  <div class="history-sources" role="group" aria-label="History source">
    <button type="button" class="ghost small" aria-pressed={view === "received"} onclick={() => void load(null, null)}>Received history</button>
    <button type="button" class="ghost small" aria-pressed={view === "local"} onclick={() => void loadLocal()}>Saved on this device</button>
    <button type="button" class="ghost small" aria-pressed={view === "held"} onclick={() => void loadHeld()}>Held posts</button>
  </div>
  {#if view === "received" && target}
    <button type="button" class="ghost small" onclick={() => void load(null, listAfter)}>Back to messages</button>
  {/if}
  <div aria-busy={loading}>
    {#if loading}
      <p role="status">Loading history…</p>
    {:else if failure}
      <p role="status">{failure}</p>
      <button type="button" class="ghost small" onclick={() => {
        if (view === "held") void loadHeld();
        else if (view === "local") void loadLocal(localCursor);
        else void load(target, after);
      }}>Retry</button>
    {:else if view === "held" && heldPage}
      {#if heldPage.items === null}
        <p>Held posts are unavailable on this device.</p>
      {:else}
        {#if heldPage.unavailable}
          <p role="status">Some posts on this page could not be verified.</p>
        {:else if heldPage.items.length === 0}
          <p>No held posts on this page.</p>
        {/if}
        {#if heldPage.items.length}
          <ul>
            {#each heldPage.items as item (item.caseId)}
              <li>
                <strong>{item.decision.status === "selected"
                  ? reviewChoice[item.decision.choice]
                  : item.decision.status === "undecided" ? "Awaiting review" : "Review status unavailable"}</strong>
                <p class="message">{item.body.text}</p>
                <details>
                  <summary>Signed record details</summary>
                  <dl>
                    <dt>Device key</dt><dd>{item.signer}</dd>
                    <dt>Membership record</dt><dd>{item.admission}</dd>
                    <dt>Message action</dt><dd>{item.action}</dd>
                    <dt>Original digest</dt><dd>{item.originalDigest}</dd>
                    <dt>Removal record</dt><dd>{item.removalHeader}</dd>
                    {#if item.body.reply_to}<dt>Reply to</dt><dd>{item.body.reply_to}</dd>{/if}
                  </dl>
                </details>
              </li>
            {/each}
          </ul>
        {/if}
        {#if heldPage.next}
          {@const next = heldPage.next}
          <button type="button" class="ghost small" onclick={() => void loadHeld(next)}>Next page</button>
        {/if}
        {#if heldAfter}
          <button type="button" class="ghost small" onclick={() => void loadHeld()}>First page</button>
        {/if}
      {/if}
    {:else if view === "local" && localPage}
      {#if localPage.items.length === 0}
        <p>No originals saved on this device on this page.</p>
      {:else}
        <ul>
          {#each localPage.items as entry (entry.action)}
            <li>
              <strong>{labels[entry.body.kind]}</strong>
              <span class="disposition">{localStatus[entry.status]}</span>
              {#if "text" in entry.body}<p class="message">{entry.body.text}</p>{/if}
              <details>
                <summary>Saved record details</summary>
                <dl>
                  <dt>Device key</dt><dd>{entry.signer}</dd>
                  <dt>Message action</dt><dd>{entry.action}</dd>
                  {#if entry.body.kind !== "create"}
                    <dt>Original message</dt><dd>{entry.body.create}</dd>
                  {/if}
                </dl>
              </details>
            </li>
          {/each}
        </ul>
      {/if}
      {#if localPage.next !== localPage.end_exclusive}
        {@const cursor = { next: localPage.next, end: localPage.end_exclusive }}
        <button type="button" class="ghost small" onclick={() => void loadLocal(cursor)}>Next page</button>
      {/if}
      {#if localCursor.next !== "0"}
        <button type="button" class="ghost small" onclick={() => void loadLocal()}>First page</button>
      {/if}
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
  .history-sources { display: flex; flex-wrap: wrap; gap: .4rem; margin: .8rem 0; }
  .history-sources [aria-pressed="true"] { border-color: var(--accent, #729ccf); }
  .disposition { margin-inline-start: .6rem; }
  ul { list-style: none; margin: 1rem 0; padding: 0; }
  li { padding: .7rem 0; border-top: 1px solid var(--line, #3b3746); }
  .message { white-space: pre-wrap; overflow-wrap: anywhere; }
  dd { margin: 0 0 .45rem; overflow-wrap: anywhere; font-family: monospace; }
</style>
