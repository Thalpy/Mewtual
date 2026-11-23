<script lang="ts">
  import type { NativeOriginal, NativeReceived } from "./native-text-history";
  let { rows, received = [], archive = null, readArchive, loading, loaded, failure, more, text, busy, send, refresh, loadMore, change }: {
    rows: NativeOriginal[]; received?: NativeReceived[]; loading: boolean; loaded:boolean;failure:string|null;more: boolean; text: string; busy: boolean;
    archive?: { items: NativeReceived[]; note: string | null; loading: boolean; more: boolean } | null;
    readArchive?: (append: boolean) => void;
    send: () => void; refresh: () => void; loadMore: () => void; change: (value: string) => void;
  } = $props();
</script>

<section class="native-saved-chat" aria-label="Chat messages">
  <header>
    <h2>Messages</h2>
    {#if readArchive}<button onclick={() => readArchive(false)} disabled={loading || busy || archive?.loading}>Saved received messages</button>{/if}
    <button onclick={refresh} disabled={loading || busy}>Refresh</button>
  </header>
  <div class="native-saved-rows" aria-busy={loading}>
    {#if failure}<p role="status">{failure}</p>
    {:else if (loading || !loaded) && !rows.length}<p>Loading messages…</p>
    {:else if !rows.length && more}<p>More history is available.</p>
    {:else if !rows.length && !received.length && !archive && loaded}<p>Nothing saved here yet.</p>{/if}
    {#each rows as row (`${row.ordinal}:${row.action}`)}
      <article>
        <small>You · {row.status === "pending" ? "Saved here" : row.status === "submitted" ? "Sent from this device" : "Not sent"}</small>
        {#if row.body.kind === "create"}<p>{row.body.text}</p>
        {:else if row.body.kind === "edit"}<p>Saved edit: {row.body.text}</p>
        {:else if row.body.kind === "delete"}<p>Saved deletion.</p>
        {:else}<p>Saved change.</p>{/if}
      </article>
    {/each}
    {#if archive}
      <section aria-label="Saved received messages" aria-busy={archive.loading}>
        {#if archive.note}<p role="status">{archive.note}</p>{/if}
        {#each archive.items.filter(item => !rows.some(row => row.action === item.action) && !received.some(row => row.action === item.action)) as item (item.action)}
          <article>
            <small>{item.certainty === "provisional" ? "Saved received · Provisional" : "Saved received · Verified"}</small>
            <p>{item.text}</p>
          </article>
        {/each}
        {#if !archive.items.length && !archive.loading && !archive.more && !archive.note}<p>No saved received messages in this view.</p>{/if}
        {#if archive.more && readArchive}<button onclick={() => readArchive(true)} disabled={busy || archive.loading}>Load more received messages</button>{/if}
      </section>
    {/if}
    {#each received.filter(item => !rows.some(row => row.action === item.action)) as item (item.action)}
      <article>
        <small>{item.certainty === "provisional" ? "Received · Provisional" : "Received · Verified"}</small>
        <p>{item.text}</p>
      </article>
    {/each}
    {#if more}<button onclick={loadMore} disabled={loading || busy}>Load more saved messages</button>{/if}
  </div>
  <form onsubmit={(event) => { event.preventDefault(); send(); }}>
    <textarea aria-label="Message" placeholder="Write a message" value={text} oninput={(event) => change(event.currentTarget.value)}></textarea>
    <button type="submit" disabled={busy || !text.trim()}>Send</button>
  </form>
</section>

<style>
  .native-saved-chat{display:flex;flex-direction:column;gap:1rem;min-height:0;height:100%;padding:1rem}
  header{display:flex;justify-content:space-between;align-items:center;gap:1rem}h2{margin:0}small{opacity:.7;font-size:.8rem}
  .native-saved-rows{flex:1;overflow:auto;display:flex;flex-direction:column;gap:.75rem}
  .native-saved-rows section{display:flex;flex-direction:column;gap:.75rem;align-items:stretch}
  article{border:1px solid color-mix(in srgb,currentColor 18%,transparent);border-radius:.75rem;padding:.75rem}article p{white-space:pre-wrap;overflow-wrap:anywhere;margin:.4rem 0 0}
  form{display:flex;gap:.75rem;align-items:flex-end}textarea{flex:1;min-height:5rem;resize:vertical}button{white-space:nowrap}
</style>
