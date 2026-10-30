<script lang="ts">
  import { onMount, untrack } from "svelte";
  import {
    ChatReviewAuditReader, reviewEffectLabel, caseStateLabel, CASE_STATE_LABELS,
    type AuditReads, type AuditSnapshot, type AdminAuditRow,
  } from "./chat-review-audit.ts";

  // The parent MUST key this component to the original server/view/session/role and
  // unmount it on lock or loss of moderator access. Native repeats the real role check.
  let { reads, nameOf }: { reads: AuditReads; nameOf: (fingerprint: string) => string } = $props();
  const originalReads = untrack(() => reads);
  let snapshot = $state<AuditSnapshot>({ page: null, detail: null, loading: false, detailLoading: false, stale: false, message: "" });
  const reader = new ChatReviewAuditReader({
    page: (cursor) => originalReads.page(cursor), detail: (caseId, revision) => originalReads.detail(caseId, revision),
  }, (next) => { snapshot = next; });
  onMount(() => { void reader.load(); return () => reader.dispose(); });
  const actions = { create: "Message", edit: "Edit", delete: "Deletion", join: "History link" };
  const choices = { approve: "Approval", reject: "Rejection", reopen: "Reopening" };
  // Presentation clocks are never decision authority. Paging remains the native immutable
  // row order; this sort affects only the current, explicitly bounded page's display.
  let rows = $derived([...(snapshot.page?.rows ?? [])].sort((a, b) =>
    Number(a.presentedMs === null) - Number(b.presentedMs === null)
      || (a.presentedMs ?? 0) - (b.presentedMs ?? 0) || (a.rowId < b.rowId ? -1 : a.rowId > b.rowId ? 1 : 0)));
  function label(row: AdminAuditRow) {
    return row.kind === "held-action" ? actions[row.actionKind] : choices[row.choice];
  }
</script>

<section class="review-audit" aria-labelledby="chat-review-heading">
  <header>
    <div><h3 id="chat-review-heading">Held messages and reviews</h3>
      {#if snapshot.page}<p>Showing {snapshot.page.rows.length} of {snapshot.page.knownRowCount} entries.</p>{/if}
    </div>
    <button class="ghost small" disabled={snapshot.loading} onclick={() => reader.load()}>
      {snapshot.loading ? "Loading…" : "Refresh"}
    </button>
  </header>
  {#if snapshot.message}<p role="status">{snapshot.message}</p>{/if}
  {#if snapshot.stale}<p class="notice">This view is out of date.</p>{/if}
  {#if snapshot.page?.availability === "incomplete"}<p class="notice">Some review history is still missing.</p>{/if}
  {#if snapshot.page}
    <ol>
      {#each rows as row (row.rowId)}
        <li data-review-row={row.rowId}>
          <div class="review-meta">
            <strong>{label(row)}</strong>
            <span>{nameOf(row.authorFingerprint)}</span>
            {#if row.presentedMs !== null}<time datetime={new Date(row.presentedMs).toISOString()}>{new Date(row.presentedMs).toLocaleString()}</time>{/if}
          </div>
          {#if row.kind === "held-action"}
            <span class="review-state">{caseStateLabel(row)}</span>
            {#if row.preview}<p class="review-text">{row.preview.text}{row.preview.truncated ? "…" : ""}</p>{/if}
          {:else}
            <span class="review-state">{reviewEffectLabel(row)} · {nameOf(row.issuerFingerprint)}</span>
            {#if row.reason}<p class="review-text">{row.reason}</p>{/if}
          {/if}
          <button class="ghost small" disabled={snapshot.loading || snapshot.detailLoading || snapshot.stale} onclick={() => reader.open(row.caseId)}>View details</button>
        </li>
      {:else}
        <li class="muted">{snapshot.page.knownRowCount ? "No entries on this page." : snapshot.page.availability === "complete" ? "No held messages or review decisions." : "Review history is still loading."}</li>
      {/each}
    </ol>
    {#if snapshot.page.nextCursor}
      <button disabled={snapshot.loading || snapshot.stale} onclick={() => reader.load(snapshot.page!.nextCursor)}>Next page</button>
    {/if}
  {/if}
  {#if snapshot.detailLoading}<p role="status">Loading held message…</p>{/if}
  {#if snapshot.detail}
    {@const detail = snapshot.detail}
    <section class="review-detail" aria-label="Held message details">
      <header><h4>{actions[detail.case.actionKind]} · {caseStateLabel(detail.case)}</h4><button class="ghost small" onclick={() => reader.closeDetail()}>Close details</button></header>
      {#if "text" in detail.action}<p class="review-text">{detail.action.text}</p>{/if}
      {#if detail.availability === "incomplete" || detail.evidenceTruncated || detail.missingFactIds.length}<p class="notice">Some evidence is missing.</p>{/if}
      <details>
        <summary>Evidence</summary>
        <dl>
          <dt>Action</dt><dd>{detail.case.actionCid}</dd>
          <dt>Author key</dt><dd>{detail.case.authorPublicKey}</dd>
          <dt>Removal</dt><dd>{detail.case.removalCutCid}</dd>
          <dt>Signature</dt><dd>{detail.case.proof.signature}</dd>
          <dt>Authority</dt><dd>{detail.case.proof.authority}</dd>
          <dt>Reported outcome</dt><dd>{CASE_STATE_LABELS[detail.case.state]}</dd>
        </dl>
        <label>Original signed action<textarea readonly rows="3" value={detail.signedActionHex}></textarea></label>
      </details>
    </section>
  {/if}
</section>

<style>
  .review-audit { border: 1px solid var(--border, #383344); border-radius: 12px; padding: 16px; margin: 16px 0; }
  header, .review-meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  header { justify-content: space-between; }
  h3, h4 { margin: 0; }
  header p { margin: 5px 0; }
  ol { list-style: none; padding: 0; margin: 12px 0; }
  li { padding: 12px 0; border-top: 1px solid var(--border, #383344); }
  .review-meta time { margin-left: auto; font-size: .85em; }
  .review-state { display: block; margin-top: 5px; font-size: .9em; }
  .review-text { white-space: pre-wrap; overflow-wrap: anywhere; }
  .review-detail { margin-top: 16px; border-top: 1px solid var(--border, #383344); padding-top: 16px; }
  .notice { color: var(--muted, #a5a0af); }
  dd { margin: 0 0 8px; overflow-wrap: anywhere; font-family: monospace; }
  dt { font-weight: bold; }
  textarea { box-sizing: border-box; width: 100%; resize: vertical; }
</style>
