<script lang="ts">
  import type { NativeOriginal, NativeOwnMessage, NativeReceived } from "./native-text-history";
  let { rows, own = {}, readOwn, mutate, received = [], archive = null, readArchive, loading, loaded, failure, more, text, busy, send, refresh, loadMore, change, replyTo = null, reply }: {
    rows: NativeOriginal[]; received?: NativeReceived[]; loading: boolean; loaded:boolean;failure:string|null;more: boolean; text: string; busy: boolean;
    own?: Record<string, NativeOwnMessage>;
    readOwn?: (action: string) => void;
    mutate?: (action: string, revision: string, text: string | null) => void;
    archive?: { items: NativeReceived[]; note: string | null; loading: boolean; more: boolean } | null;
    readArchive?: (append: boolean) => void;
    send: () => void; refresh: () => void; loadMore: () => void; change: (value: string) => void;
    replyTo?: string | null; reply?: (action: string | null) => void;
  } = $props();
  const ownRows = $derived(rows.filter(row => row.body.kind === "create"));
  const shownOwn = $derived(ownRows.filter(row => !own[row.action] || own[row.action].scanning || own[row.action].item));
  const shownArchive = $derived(archive && (archive.items.length || archive.loading || archive.note || archive.more) ? archive : null);
  const replyText = $derived((replyTo && own[replyTo]?.item?.text)
    || received.find(row => row.action === replyTo)?.text || archive?.items.find(row => row.action === replyTo)?.text);
  let changeTarget = $state<{ action: string; revision: string; editing: boolean } | null>(null);
  let editText = $state("");
  function begin(action: string, revision: string, text: string | null) {
    changeTarget = { action, revision, editing: text !== null };
    editText = text ?? "";
  }
  function cancel() {
    changeTarget = null;
    editText = "";
  }
  $effect(() => {
    if (changeTarget && own[changeTarget.action]?.item?.revision !== changeTarget.revision) cancel();
  });
  function submit() {
    if (busy || !changeTarget || !mutate || own[changeTarget.action]?.item?.revision !== changeTarget.revision) return;
    const { action, revision, editing } = changeTarget;
    if (editing && !editText.trim()) return;
    const value = editing ? editText : null;
    cancel();
    mutate(action, revision, value);
  }
</script>

<section class="native-saved-chat" aria-label="Chat messages">
  <header>
    <h2>Messages</h2>
    {#if readArchive}<button onclick={() => readArchive(false)} disabled={loading || busy || archive?.loading}>Saved received messages</button>{/if}
    <button onclick={refresh} disabled={loading || busy}>Refresh</button>
  </header>
  <div class="native-saved-rows" aria-busy={loading}>
    {#if failure}<p role="status">{failure}</p>
    {:else if (loading || !loaded) && !shownOwn.length}<p>Loading messages…</p>
    {:else if !shownOwn.length && more}<p>More history is available.</p>
    {:else if !shownOwn.length && !received.length && !shownArchive && loaded}<p>Nothing saved here yet.</p>{/if}
    {#each shownOwn as row (row.action)}
      {@const view = own[row.action]?.item}
      <article>
        {#if view}
          <small>You · {view.status === "pending" ? "Saved here" : view.status === "submitted" ? "Sent from this device" : "Not sent"}</small>
          {#if changeTarget?.action === row.action && changeTarget.revision === view.revision}
            <form class="native-mutation" onsubmit={(event) => { event.preventDefault(); submit(); }}>
              {#if changeTarget.editing}
                <textarea aria-label="Edit message" bind:value={editText}></textarea>
              {:else}
                <p>Delete this message?</p>
              {/if}
              <button type="submit" disabled={busy || (changeTarget.editing && !editText.trim())}>{changeTarget.editing ? "Save edit" : "Delete message"}</button>
              <button type="button" onclick={cancel}>Cancel</button>
            </form>
          {:else}
            <p>{view.text}</p>
            {#if reply}<button type="button" onclick={() => reply(row.action)} disabled={busy}>Reply</button>{/if}
            {#if mutate}
              <button type="button" onclick={() => begin(row.action, view.revision, view.text)} disabled={busy}>Edit</button>
              <button type="button" onclick={() => begin(row.action, view.revision, null)} disabled={busy}>Delete</button>
            {/if}
          {/if}
        {:else if readOwn}
          <button type="button" onclick={() => readOwn(row.action)} disabled={busy}>Load saved message</button>
        {/if}
      </article>
    {/each}
    {#if shownArchive}
      <section aria-label="Saved received messages" aria-busy={shownArchive.loading}>
        {#if shownArchive.note}<p role="status">{shownArchive.note}</p>{/if}
        {#each shownArchive.items.filter(item => !rows.some(row => row.action === item.action) && !received.some(row => row.action === item.action)) as item (item.action)}
          <article>
            <small>{item.certainty === "provisional" ? "Saved received · Provisional" : "Saved received · Verified"}</small>
            <p>{item.text}</p>
            {#if reply}<button type="button" onclick={() => reply(item.action)} disabled={busy}>Reply</button>{/if}
          </article>
        {/each}
        {#if shownArchive.more && readArchive}<button onclick={() => readArchive(true)} disabled={busy || shownArchive.loading}>Load more received messages</button>{/if}
      </section>
    {/if}
    {#each received.filter(item => !rows.some(row => row.action === item.action)) as item (item.action)}
      <article>
        <small>{item.certainty === "provisional" ? "Received · Provisional" : "Received · Verified"}</small>
        <p>{item.text}</p>
        {#if reply}<button type="button" onclick={() => reply(item.action)} disabled={busy}>Reply</button>{/if}
      </article>
    {/each}
    {#if more}<button onclick={loadMore} disabled={loading || busy}>Load more saved messages</button>{/if}
  </div>
  {#if replyTo && reply}
    <div class="native-reply" role="status">
      <span>{replyText ? `Replying to ${replyText}` : "Replying to a message"}</span>
      <button type="button" onclick={() => reply(null)} disabled={busy}>Cancel reply</button>
    </div>
  {/if}
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
  .native-reply{display:flex;gap:.75rem;align-items:center;justify-content:space-between}.native-reply span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
  .native-mutation{margin-top:.4rem}
  form{display:flex;gap:.75rem;align-items:flex-end}textarea{flex:1;min-height:5rem;resize:vertical}button{white-space:nowrap}
</style>
