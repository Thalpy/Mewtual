<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { ChatLedgerReader, type LedgerReads, type LedgerSnapshot, type LegacyMutation } from "./chat-admin-ledger.ts";

  // One immutable authorized mount. The parent keys this to server/UI/view/role and unmounts
  // on lock/access loss. Callback closures capture their original scalar scope before any IPC.
  let { reads, nameOfIdentity, channelName, refreshTick = 0 }: {
    reads: LedgerReads; nameOfIdentity: (fullIdentity: string) => string; channelName: (channel: string) => string; refreshTick?: number;
  } = $props();
  const originalReads = untrack(() => reads);
  let snapshot = $state<LedgerSnapshot>({ page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" });
  const reader = new ChatLedgerReader(originalReads, (next) => { snapshot = next; });
  let mounted = false;
  let observedRefresh = untrack(() => refreshTick);
  // Proof/decision-only updates reuse the body-free moderation event. Ordinary chat appends
  // do not reset this finite captured view on every message; the user can refresh between pages.
  $effect(() => {
    const next = refreshTick;
    if (next !== observedRefresh) {
      observedRefresh = next;
      if (mounted) untrack(() => { void load(); });
    }
  });
  let retry: ReturnType<typeof setTimeout> | null = null;
  function cancelRetry() { if (retry !== null) clearTimeout(retry); retry = null; }
  async function load(cursor: string | null = null) {
    cancelRetry();
    const outcome = await reader.load(cursor);
    if (mounted && outcome === "busy") retry = setTimeout(() => { retry = null; void load(cursor); }, 100);
  }
  async function open(rowId: string, offset = 0) {
    cancelRetry();
    const outcome = await reader.open(rowId, offset);
    if (mounted && outcome === "busy") retry = setTimeout(() => { retry = null; void open(rowId, offset); }, 100);
  }
  function closeDetail() { cancelRetry(); reader.closeDetail(); }
  onMount(() => {
    mounted = true; void load();
    return () => { mounted = false; cancelRetry(); reader.dispose(); };
  });
  const signatureLabels = { valid: "Signature verified", invalid: "Invalid signature",
    pending: "Checking signature", "not-checked": "Signature not checked" };
  const operationLabels = { "make-map": "Created object", "make-list": "Created list", "make-text": "Created text",
    "make-table": "Created table", set: "Set", delete: "Deleted", increment: "Changed counter",
    mark: "Added formatting", unmark: "Removed formatting", unknown: "Other change" };
  function fieldName(change: LegacyMutation) {
    return change.key.kind === "map" ? change.key.value : change.insert ? "New item" : "List item";
  }
</script>

<section class="chat-admin-ledger" aria-labelledby="chat-history-heading">
  <header>
    <div><h3 id="chat-history-heading">Chat history</h3>
      <p>Original posts, edits and deletions on this device.</p></div>
    <button class="ghost small" disabled={snapshot.loading} onclick={() => load()}>{snapshot.loading ? "Loading…" : "Refresh history"}</button>
  </header>
  {#if snapshot.message}<p role="status">{snapshot.message}</p>{/if}
  {#if snapshot.stale}<p class="history-note">This page is out of date.</p>{/if}
  {#if snapshot.page?.availability === "incomplete"}<p class="history-note">Some records or author details may be missing.</p>{/if}
  {#if snapshot.page}
    <p class="history-count">Showing {snapshot.page.rows.length} of {snapshot.page.knownRowCount} local records.</p>
    <ol class="history-records">
      {#each snapshot.page.rows as row (row.rowId)}
        <li data-history-row={row.rowId}>
          <button class="history-record" aria-pressed={snapshot.detail?.row.rowId === row.rowId}
            disabled={snapshot.loading || snapshot.detailLoading || snapshot.stale} onclick={() => open(row.rowId)}>
            <!-- A short fingerprint is only a display abbreviation. Even a valid signature
                 must resolve the name by the complete signer identity: prefixes can collide. -->
            <span class="record-meta"><strong>{row.authorId === null ? "Unknown author" : row.signature === "valid" ? nameOfIdentity(row.authorId) : "Unverified author"}</strong>
              <span>#{channelName(row.channel)}</span>
              {#if row.presentedMs !== null}<time datetime={new Date(row.presentedMs).toISOString()}>{new Date(row.presentedMs).toLocaleString()}</time>{/if}
            </span>
            {#if row.preview}<span class="record-preview">{row.preview.text}{row.preview.truncated ? "…" : ""}</span>{:else}<span>Chat change</span>{/if}
            <span class="record-proof">{signatureLabels[row.signature]}</span>
          </button>
        </li>
      {:else}
        <li class="history-note">{snapshot.page.availability === "complete" ? "No local chat records." : "No records are available on this page."}</li>
      {/each}
    </ol>
    {#if snapshot.page.nextCursor}
      <button disabled={snapshot.loading || snapshot.stale} onclick={() => load(snapshot.page!.nextCursor)}>Next page</button>
    {/if}
  {/if}
  {#if snapshot.detailLoading}<p role="status">Loading original…</p>{/if}
  {#if snapshot.detail}
    {@const detail = snapshot.detail}
    <section class="history-detail" aria-label="Original chat record">
      <header><h4>Original text and changes</h4><button class="ghost small" onclick={closeDetail}>Close details</button></header>
      {#if detail.legacyChange?.status === "decoded"}
        <ol class="original-changes">
          {#each detail.legacyChange.mutations as change (change.operationId)}
            <li><div><strong>{operationLabels[change.operation]}</strong> <span>{fieldName(change)}</span></div>
              {#if change.value !== null}<pre class="original-value">{change.value.text}</pre>{/if}
              <details><summary>Record IDs</summary><dl><dt>Operation</dt><dd>{change.operationId}</dd><dt>Object</dt><dd>{change.objectId}</dd>
                {#if change.predecessors.length}<dt>Replaces</dt><dd>{change.predecessors.join("\n")}</dd>{/if}</dl></details>
            </li>
          {/each}
        </ol>
        {#if detail.legacyChange.mutationsTruncated || detail.evidenceTruncated}<p class="history-note">The original contains more details than can be shown here.</p>{/if}
      {:else}<p>This record cannot be displayed as text. Its original bytes are available below.</p>{/if}
      <details class="original-proof"><summary>Author and original bytes</summary>
        <dl><dt>{detail.row.signature === "valid" ? "Author" : "Claimed author"}</dt><dd>{detail.row.authorId ?? "Not recorded"}</dd>
          <dt>Signature</dt><dd>{signatureLabels[detail.row.signature]}</dd>
          <dt>Past server membership</dt><dd>Not proven by this original</dd>
          <dt>Record</dt><dd>{detail.row.opHash}</dd>
          <dt>Stored channel</dt><dd>{detail.row.channel}</dd>
          {#if detail.row.scope === "mismatched"}<dt>Signed channel</dt><dd>{detail.row.signedDocId} (different stored scope)</dd>{/if}
        </dl>
        <p>Bytes {detail.original.offset}–{detail.original.offset + detail.original.bytes.length / 2} of {detail.original.totalBytes}</p>
        <pre class="original-bytes">{detail.original.bytes}</pre>
        {#if detail.original.nextOffset !== null}
          <button disabled={snapshot.detailLoading || snapshot.stale} onclick={() => open(detail.row.rowId, detail.original.nextOffset!)}>Next part</button>
        {/if}
      </details>
    </section>
  {/if}
</section>

<style>
  .chat-admin-ledger { display: grid; gap: 0.75rem; min-width: 0; }
  header, .record-meta { display: flex; justify-content: space-between; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
  h3, h4, p { margin: 0; }
  header p, .history-count, .history-note, .record-proof { color: var(--muted, #aaa); font-size: 0.85rem; }
  .history-records, .original-changes { display: grid; gap: 0.5rem; list-style: none; margin: 0; padding: 0; }
  .history-record { width: 100%; display: grid; gap: 0.35rem; text-align: left; white-space: normal; padding: 0.75rem; }
  .history-record[aria-pressed="true"] { outline: 2px solid var(--accent, #7db7eb); outline-offset: 1px; }
  .record-meta { justify-content: flex-start; }
  .record-preview, .original-value { white-space: pre-wrap; overflow-wrap: anywhere; }
  .history-detail { border: 1px solid var(--line, #4b4259); border-radius: 0.5rem; padding: 0.9rem; display: grid; gap: 0.75rem; min-width: 0; }
  .original-changes li { border-bottom: 1px solid var(--line, #4b4259); padding: 0.5rem 0; min-width: 0; }
  pre { margin: 0.4rem 0; font-family: inherit; max-height: 18rem; overflow: auto; }
  .original-bytes { font-family: monospace; font-size: 0.75rem; white-space: pre-wrap; overflow-wrap: anywhere; }
  summary { cursor: pointer; }
  dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 0.4rem 0.75rem; font-size: 0.8rem; }
  dt { color: var(--muted, #aaa); } dd { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }
  @media (max-width: 560px) { dl { grid-template-columns: 1fr; } }
</style>
