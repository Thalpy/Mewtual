<script lang="ts">
  import type { NativeOriginal } from "./native-text-history";
  let { rows, loading, loaded, failure, more, text, busy, send, refresh, loadMore, change }: {
    rows: NativeOriginal[]; loading: boolean; loaded:boolean;failure:string|null;more: boolean; text: string; busy: boolean;
    send: () => void; refresh: () => void; loadMore: () => void; change: (value: string) => void;
  } = $props();
</script>

<section class="native-saved-chat" aria-label="Messages saved on this device">
  <header><div><h2>Saved messages</h2><p>Messages saved on this device.</p></div><button onclick={refresh} disabled={loading || busy}>Refresh</button></header>
  <div class="native-saved-rows" aria-busy={loading}>
    {#if failure}<p role="status">{failure}</p>
    {:else if (loading || !loaded) && !rows.length}<p>Loading messages…</p>
    {:else if !rows.length && more}<p>More history is available.</p>
    {:else if !rows.length && loaded}<p>Nothing saved here yet.</p>{/if}
    {#each rows as row (`${row.ordinal}:${row.action}`)}
      <article>
        <small>You · {row.status === "pending" ? "Saved here" : row.status === "submitted" ? "Sent from this device" : "Not sent"}</small>
        {#if row.body.kind === "create"}<p>{row.body.text}</p>
        {:else if row.body.kind === "edit"}<p>Saved edit: {row.body.text}</p>
        {:else if row.body.kind === "delete"}<p>Saved deletion.</p>
        {:else}<p>Saved change.</p>{/if}
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
  header{display:flex;justify-content:space-between;align-items:center;gap:1rem}h2,header p{margin:0}header p,small{opacity:.7;font-size:.8rem}
  .native-saved-rows{flex:1;overflow:auto;display:flex;flex-direction:column;gap:.75rem}
  article{border:1px solid color-mix(in srgb,currentColor 18%,transparent);border-radius:.75rem;padding:.75rem}article p{white-space:pre-wrap;overflow-wrap:anywhere;margin:.4rem 0 0}
  form{display:flex;gap:.75rem;align-items:flex-end}textarea{flex:1;min-height:5rem;resize:vertical}button{white-space:nowrap}
</style>
