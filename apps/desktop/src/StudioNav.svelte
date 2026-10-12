<script lang="ts">
  // The studio's contextual sidebar: this channel's flipnotes and scores with what is actually
  // known about each (design-creative-suite.md section 5, "Studio"), read from the connected
  // session's Index view. Overflow and deleted entries stay visible as what they are.
  import { untrack } from "svelte";
  import { ensureStudio, setStudioScope, studio } from "./studio-state.svelte.ts";
  import { PixRaster } from "./pix-canvas.ts";
  import { DEFAULT_PALETTE } from "./studio-store.ts";
  import { FLIPNOTE_H, FLIPNOTE_W } from "./studio-contract.ts";
  import { reason, type KnownView, type SaveRecord } from "./studio-session.ts";
  import { base64ToBytes, type NativeExpiry } from "./studio-native.ts";

  let { me, server, channel, onopen, onnotice } = $props<{
    me: string;
    server: number | null;
    channel: string;
    onopen: () => void;
    onnotice?: (text: string, kind: "info" | "warn" | "error") => void;
  }>();

  // At init, not in a derived: it writes shared state once, keyed on the identity at mount.
  // svelte-ignore state_referenced_locally
  const session = ensureStudio(me);
  // Tracked on the props only; the scope change itself runs untracked (see studio-state).
  $effect(() => {
    const s = server, c = channel;
    untrack(() => setStudioScope(s, c));
  });

  const model = $derived.by(() => { void studio.rev; return session.indexModel; });
  const indexView = $derived.by(() => { void studio.rev; return session.index; });
  const indexError = $derived.by(() => { void studio.rev; return session.indexError; });
  const indexOverlay = $derived.by(() => { void studio.rev; return session.indexOverlay; });
  const loading = $derived.by(() => { void studio.rev; return session.indexLoading && !session.index; });
  const pendingCreates = $derived.by(() => { void studio.rev; return session.saves.filter((s) => s.kind === "create"); });
  const indexAttention = $derived.by(() => {
    void studio.rev;
    return session.pendingFor(null).filter((s) => s.status === "blocked" || s.status === "uncertain");
  });
  // Read through the revision, and first in the expression below: a non-reactive read that
  // short-circuits on the first render would leave the template never subscribed to the rest.
  const hasScope = $derived.by(() => { void studio.rev; return session.scope !== null; });
  const indexDraftChoices = $derived.by(() => { void studio.rev; return session.overlayChoices("copy", null); });
  const indexDraftKey = $derived.by(() => {
    void studio.rev;
    const branch = indexOverlay?.lifecycle?.branch;
    const scope = session.scope;
    return scope && branch ? `${scope.server}:${scope.channel}:index:${branch.branch}:${branch.content}` : "";
  });
  const indexArchiveKey = $derived.by(() => {
    void studio.rev;
    const archive = indexOverlay?.archive?.archive;
    const scope = session.scope;
    return scope && archive ? `${scope.server}:${scope.channel}:index:${archive}` : "";
  });
  let indexDiscardArmed = $state("");
  let indexReleaseArmed = $state("");
  let indexSaveDiscardArmed = $state("");
  let indexCopyForgetArmed = $state("");
  const indexSaveKey = $derived.by(() => {
    void studio.rev;
    const save = indexAttention[0];
    const scope = session.scope;
    return save && scope ? `${scope.server}:${scope.channel}:index-save:${save.id}` : "";
  });
  const indexCopyForgetKey = $derived.by(() => {
    void studio.rev;
    const copy = indexOverlay?.copy;
    const scope = session.scope;
    return scope && copy?.status === "uncertain" && copy.edit
      ? `${scope.server}:${scope.channel}:index-copy:${copy.edit.nonce}`
      : "";
  });
  $effect(() => {
    const draftNow = indexDraftKey;
    const archiveNow = indexArchiveKey;
    const saveNow = indexSaveKey;
    const copyNow = indexCopyForgetKey;
    untrack(() => {
      if (indexDiscardArmed && indexDiscardArmed !== draftNow) indexDiscardArmed = "";
      if (indexReleaseArmed && indexReleaseArmed !== archiveNow) indexReleaseArmed = "";
      if (indexSaveDiscardArmed && indexSaveDiscardArmed !== saveNow) indexSaveDiscardArmed = "";
      if (indexCopyForgetArmed && indexCopyForgetArmed !== copyNow) indexCopyForgetArmed = "";
    });
  });

  function overlayRetryOf(s: SaveRecord) { return s.kind === "frame" || s.kind === "overlayApply" ? s.overlayRetry : null; }

  async function indexDraftAction(action: () => Promise<unknown>, success: string) {
    try { await action(); onnotice?.(success, "info"); } catch (e) { onnotice?.(reason(e), "warn"); }
  }
  async function copyIndexDraftItem(choice: (typeof indexDraftChoices)[number]) {
    try { await session.copyOverlayChoice(null, choice.choice, choice.label, "copy"); } catch (e) { onnotice?.(reason(e), "warn"); }
  }
  async function retryIndexDraftCopy() {
    try { await session.retryOverlayCopy(null); } catch (e) { onnotice?.(reason(e), "warn"); }
  }
  async function repreviewIndexDraftCopy() {
    try { await session.repreviewOverlayCopy(null); } catch (e) { onnotice?.(reason(e), "warn"); }
  }
  function forgetIndexDraftCopy() {
    // The confirmation-only button supplies consent; the session still refuses to forget any
    // operation that is no longer uncertain by the time this click is handled.
    const forgotten = session.forgetOverlayCopy(null, indexCopyForgetArmed.slice(-32));
    indexCopyForgetArmed = "";
    if (forgotten) onnotice?.("stopped tracking the uncertain index copy; it may already have landed", "warn");
  }
  function downloadIndexDraft() {
    const x = indexOverlay?.exported;
    if (!x?.bytesB64) return;
    try {
      const url = URL.createObjectURL(new Blob([base64ToBytes(x.bytesB64)], { type: "application/octet-stream" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = "studio-index-draft.catcoms-studio-draft-v1.bin";
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) { onnotice?.(`could not hand the index draft archive to the browser: ${reason(e)}`, "warn"); }
  }
  function words(value: string): string { return value.replace(/[A-Z]/g, (c) => ` ${c.toLowerCase()}`); }

  function known(id: string): KnownView | null {
    void studio.rev;
    return session.known.get(id) ?? null;
  }

  function chip(k: KnownView | null): { text: string; tone: "ok" | "warn" | "danger" | "" } {
    if (!k) return { text: "", tone: "" };
    if (k.awaiting) return { text: "preview", tone: "warn" };
    if (k.phase === "fault") return { text: "fault", tone: "danger" };
    if (k.phase === "settled") return { text: "settled", tone: "ok" };
    if (k.phase === "closing") return { text: "rotating", tone: "warn" };
    return { text: "open", tone: "ok" };
  }

  function indexNote(): { text: string; tone: "warn" | "danger" | "" } {
    if (!indexView) return { text: "", tone: "" };
    if (indexView.awaitingTenureReceipt) return {
      text: indexOverlay?.inspection?.kind === "local-draft"
        ? "unconfirmed base · showing this device's local-only index draft"
        : "read-only base · owner has not confirmed this channel's history",
      tone: "warn",
    };
    if (indexView.phase === "fault") return { text: "index history fault", tone: "danger" };
    if (indexView.phase === "closing") return { text: "index rotating · new entries wait for the owner", tone: "warn" };
    return { text: "", tone: "" };
  }

  function open(id: string) {
    studio.selected = id;
    session.open(id);
    onopen();
  }

  function newFlipnote() {
    if (!session.scope) return;
    if (indexView?.awaitingTenureReceipt) { onnotice?.("read-only preview: the current owner has not confirmed this channel's history yet", "warn"); return; }
    try {
      const n = (model?.entries.length ?? 0) + (model?.overflow.length ?? 0) + 1;
      const id = session.createFlipnote(`flipnote ${n}`);
      session.open(id, { read: false });
      // A flipnote opens on a blank first frame, never on nothing: the frame follows the create
      // through the same queue and keeps its own retry identity if either step is uncertain.
      session.insertFrame(id, null, new PixRaster(FLIPNOTE_W, FLIPNOTE_H, DEFAULT_PALETTE.map((e) => ({ ...e }))).encode());
      studio.selected = id;
      onopen();
    } catch (e) {
      onnotice?.(reason(e), "warn");
    }
  }

  function expiryText(e: NativeExpiry): string {
    if (e.kind === "never") return "keeps";
    if (e.kind === "unrecorded") return "";
    const d = Math.ceil((e.ms - Date.now()) / 86_400_000);
    return d <= 0 ? "expired" : `expires ${d}d`;
  }
</script>

<h3><span>Studio</span></h3>
{#if indexNote().text}
  <p class="studio-note {indexNote().tone}">{indexNote().text}</p>
{/if}
{#if indexError}
  <p class="studio-note danger">{indexError}</p>
{/if}
{#if indexAttention.length}
  {@const save = indexAttention[0]}
  <div class="studio-index-draft warn">
    <span><b>{save.status === "blocked" ? "index save not written" : "index save uncertain"}</b> · {save.label}</span>
    <span class="micro wrap">{save.error}{#if save.status === "blocked"} · the backend confirmed that this attempt wrote nothing{/if} · your edit is retained and later Index saves wait behind it</span>
    <span class="index-draft-actions">
      {#if overlayRetryOf(save) !== "newTicket" && overlayRetryOf(save) !== "none"}
        <button type="button" class="ghost small ctx-action" onclick={() => session.retry(save.id)}>{overlayRetryOf(save) === "afterExplicitAccess" ? "access, then retry index save" : "retry exact index save"}</button>
      {/if}
      <button type="button" class="ghost small ctx-action" disabled={overlayRetryOf(save) === "none" || !session.canReauthor(save.id)} onclick={() => session.reauthor(save.id)}>save index edit as new</button>
      {#if indexSaveDiscardArmed === indexSaveKey}
        <button type="button" class="ghost small ctx-action danger" onclick={() => indexSaveDiscardArmed === indexSaveKey && session.discard(save.id)}>discard index edit for real</button>
        <button type="button" class="ghost small ctx-action" onclick={() => (indexSaveDiscardArmed = "")}>cancel</button>
      {:else}
        <button type="button" class="ghost small ctx-action" onclick={() => (indexSaveDiscardArmed = indexSaveKey)}>discard index edit…</button>
      {/if}
    </span>
  </div>
{/if}
{#if indexOverlay?.loading}
  <p class="studio-note">Checking this device for a local-only index draft…</p>
{:else if indexOverlay?.error}
  <div class="studio-index-draft warn">
    <span>Index draft status could not be read</span>
    <span class="micro wrap">{indexOverlay.error}</span>
    <button type="button" class="ghost small ctx-action" onclick={() => session.refreshIndex()}>retry status</button>
  </div>
{:else if indexOverlay?.inspection?.kind === "local-draft"}
  {@const draft = indexOverlay.inspection}
  <div class="studio-index-draft warn">
    <span>local-only index draft · {draft.accepted} saved {draft.accepted === 1 ? "edit" : "edits"}</span>
    <span class="micro wrap">{draft.provenance === "unconfirmed" ? "created while the owner's tenure was unconfirmed" : "retained from a closing rotation"}{#if draft.manualReason} · manual: {words(draft.manualReason)}{/if}</span>
    <span class="index-draft-actions">
      <button type="button" class="ghost small ctx-action" onclick={() => indexDraftAction(() => session.exportOverlay(null), "index draft export ready")}>export index draft</button>
      <button type="button" class="ghost small ctx-action" onclick={() => indexDraftAction(() => session.archiveOverlay(null), "index draft preserved in an archive")}>preserve index draft</button>
      {#if indexOverlay.archive}
        <button type="button" class="ghost small ctx-action" onclick={() => indexDraftAction(() => session.disposeOverlay(null, "preserve"), "live index draft removed; archive preserved")}>remove live index draft</button>
      {/if}
      {#if indexDraftKey && indexDiscardArmed === indexDraftKey}
        <button type="button" class="ghost small ctx-action danger" onclick={() => indexDiscardArmed === indexDraftKey && indexDraftAction(() => session.disposeOverlay(null, "discard"), "local index draft discarded")}>discard index draft for real</button>
        <button type="button" class="ghost small ctx-action" onclick={() => (indexDiscardArmed = "")}>cancel</button>
      {:else}
        <button type="button" class="ghost small ctx-action" onclick={() => (indexDiscardArmed = indexDraftKey)}>discard index draft…</button>
      {/if}
    </span>
    {#if indexView && !indexView.awaitingTenureReceipt && indexView.phase !== "closing" && indexView.phase !== "fault" && indexDraftChoices.length}
      <span class="micro wrap">copy one retained index value into current history; this does not preserve the branch</span>
      <span class="index-draft-actions">
        {#each indexDraftChoices as choice (JSON.stringify(choice.choice))}
          <button type="button" class="ghost small ctx-action" disabled={indexOverlay.copy?.status === "previewing" || indexOverlay.copy?.status === "applying" || indexOverlay.copy?.status === "refused" || indexOverlay.copy?.status === "uncertain"} onclick={() => copyIndexDraftItem(choice)}>{choice.label}</button>
        {/each}
      </span>
    {/if}
  </div>
{:else if indexOverlay?.inspection?.kind === "disposed"}
  <p class="studio-note">Local index draft disposed · {indexOverlay.inspection.mode} · terminal local history, not absence.</p>
{:else if indexOverlay?.lifecycle?.transferred}
  <p class="studio-note">The local index draft was transferred into successor history.</p>
{/if}
{#if indexOverlay?.copy}
  <div class="studio-index-draft warn">
    <span>retained index copy {indexOverlay.copy.label} · {words(indexOverlay.copy.status)}</span>
    <span class="micro wrap">this operation remains available after the source draft changes{#if indexOverlay.copy.error} · {indexOverlay.copy.error}{/if}</span>
    {#if indexOverlay.copy.status === "uncertain"}
      <span class="micro wrap">Stopping tracking does not undo the copy or prove that it never landed.</span>
      <span class="index-draft-actions">
        <button type="button" class="ghost small ctx-action" onclick={retryIndexDraftCopy}>retry exact index copy</button>
        {#if indexCopyForgetKey && indexCopyForgetArmed === indexCopyForgetKey}
          <button type="button" class="ghost small ctx-action danger" onclick={forgetIndexDraftCopy}>stop tracking index copy for real</button>
          <button type="button" class="ghost small ctx-action" onclick={() => (indexCopyForgetArmed = "")}>cancel</button>
        {:else}
          <button type="button" class="ghost small ctx-action" onclick={() => (indexCopyForgetArmed = indexCopyForgetKey)}>stop tracking index copy…</button>
        {/if}
      </span>
    {:else if indexOverlay.copy.status === "refused"}
      <button type="button" class="ghost small ctx-action" onclick={repreviewIndexDraftCopy}>preview again as a new index copy</button>
    {/if}
  </div>
{/if}
{#if indexOverlay?.archive}
  <div class="studio-index-draft">
    <span>retained index draft archive · {indexOverlay.archive.accepted} edits</span>
    <span class="micro wrap">terminal read-only recovery evidence · no authoring authority</span>
    <span class="index-draft-actions">
      <button type="button" class="ghost small ctx-action" onclick={() => indexDraftAction(() => session.exportOverlayArchive(null), "index draft archive export ready")}>export index archive</button>
      {#if indexArchiveKey && indexReleaseArmed === indexArchiveKey}
        <button type="button" class="ghost small ctx-action danger" onclick={() => indexReleaseArmed === indexArchiveKey && indexDraftAction(() => session.releaseOverlayArchive(null), "index archive released and status reconciled")}>release index archive for real</button>
        <button type="button" class="ghost small ctx-action" onclick={() => (indexReleaseArmed = "")}>cancel</button>
      {:else}
        <button type="button" class="ghost small ctx-action" onclick={() => (indexReleaseArmed = indexArchiveKey)}>release index archive…</button>
      {/if}
    </span>
  </div>
{/if}
{#if indexOverlay?.exported?.bytesB64}
  <div class="studio-index-draft">
    <span>index draft backup ready · {indexOverlay.exported.format}</span>
    <button type="button" class="ghost small ctx-action" onclick={downloadIndexDraft}>save index draft file</button>
  </div>
{/if}
<ul class="channel-list studio-nav">
  {#each model?.entries ?? [] as o (o.id)}
    <li>
      <button type="button" class="studio-obj" class:active={studio.selected === o.id} onclick={() => open(o.id)}>
        <span class="studio-obj-name">
          {#if o.kind === "flipnote"}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5"></rect><path d="M4.5 2.5v11M11.5 2.5v11M1.5 6h3M1.5 10h3M11.5 6h3M11.5 10h3"></path></svg>
          {:else}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12.5V4l6-1.5V11"></path><circle cx="4.5" cy="12.5" r="1.8"></circle><circle cx="10.5" cy="11" r="1.8"></circle></svg>
          {/if}
          <span class="nm">{o.title || "untitled"}</span>
          {#if o.titleConflicts}<span class="studio-mark" title="another title was set at the same time; nothing resolved silently">!</span>{/if}
        </span>
        <span class="studio-obj-meta">
          <span class="micro">{o.kind}{#if expiryText(o.expiry)} · {expiryText(o.expiry)}{/if}{#if o.creations > 1} · created twice{/if}</span>
          {#if chip(known(o.id)).text}<span class="studio-chip {chip(known(o.id)).tone}">{chip(known(o.id)).text}</span>{/if}
        </span>
      </button>
    </li>
  {/each}
  {#each pendingCreates as s (s.id)}
    <li>
      <button type="button" class="studio-obj pending" class:active={studio.selected === s.object} onclick={() => { studio.selected = s.object; onopen(); }}>
        <span class="studio-obj-name"><span class="nm">{s.kind === "create" ? s.request.title : ""}</span></span>
        <span class="studio-obj-meta"><span class="micro">{s.status === "uncertain" ? "create uncertain · retry in the editor" : "creating…"}</span></span>
      </button>
    </li>
  {/each}
  {#if loading}
    <li><p class="muted small">Reading this channel's studio…</p></li>
  {:else if model && !model.entries.length && !pendingCreates.length}
    <li><p class="muted small">Nothing here yet.</p></li>
  {/if}
</ul>
{#if model?.overflow.length}
  <h3><span>Beyond the 64 shown</span></h3>
  <ul class="channel-list studio-nav">
    {#each model.overflow as o (o.id)}
      <li>
        <button type="button" class="studio-obj over" class:active={studio.selected === o.id} onclick={() => open(o.id)} title="Past the index display cap; still readable and kept for recovery">
          <span class="studio-obj-name"><span class="nm">{o.title || "untitled"}</span></span>
          <span class="studio-obj-meta"><span class="micro">{o.kind} · overflow</span></span>
        </button>
      </li>
    {/each}
  </ul>
{/if}
{#if model?.deleted.length}
  <p class="muted small studio-deleted">{model.deleted.length} deleted {model.deleted.length === 1 ? "entry is" : "entries are"} kept in history</p>
{/if}
<button type="button" class="ghost small ctx-action studio-new" disabled={!indexView || !hasScope} onclick={newFlipnote}>+ new flipnote</button>
<p class="muted small">Any member can edit. The owner settles history.</p>

<style>
  .studio-nav .studio-obj {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 3px;
    text-align: left;
    padding: 5px 8px;
    border-left: 2px solid transparent;
  }
  .studio-nav .studio-obj.active {
    background: var(--accent-dim);
    border-left-color: var(--accent);
  }
  .studio-nav .studio-obj.over { opacity: 0.6; }
  .studio-nav .studio-obj.pending .nm { font-style: italic; color: var(--muted); }
  .studio-obj-name {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .studio-obj-name svg {
    width: 13px;
    height: 13px;
    flex: none;
    color: var(--faint);
  }
  .studio-obj.active .studio-obj-name svg {
    color: var(--accent);
  }
  .studio-obj-name .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .studio-mark {
    font-family: var(--mono);
    font-size: 0.6rem;
    color: var(--warn);
    border: 1px solid var(--warn-brd);
    border-radius: 999px;
    width: 13px;
    height: 13px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
  }
  .studio-obj-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    min-width: 0;
  }
  .micro {
    font-family: var(--mono);
    font-size: 0.6rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .studio-chip {
    font-family: var(--mono);
    font-size: 0.52rem;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    padding: 0 7px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--muted);
    flex: none;
  }
  .studio-chip.ok { color: var(--ok); border-color: var(--ok-brd); background: var(--ok-dim); }
  .studio-chip.warn { color: var(--warn); border-color: var(--warn-brd); background: var(--warn-dim); }
  .studio-chip.danger { color: var(--danger); border-color: var(--danger-brd); background: var(--danger-dim); }
  .studio-note {
    margin: 0 0 6px;
    padding: 4px 8px;
    border-radius: var(--r);
    font-size: 0.72rem;
    line-height: 1.4;
    color: var(--text-2);
    border: 1px solid var(--border);
  }
  .studio-note.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .studio-note.danger { background: var(--danger-dim); border-color: var(--danger-brd); }
  .studio-index-draft {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 0 0 7px;
    padding: 7px 8px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    color: var(--text-2);
    font-size: 0.72rem;
  }
  .studio-index-draft.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .index-draft-actions { display: flex; flex-wrap: wrap; gap: 4px; }
  .wrap { white-space: normal; overflow: visible; }
  .danger { color: var(--danger); }
  .studio-deleted { margin: 2px 8px 6px; }
  .studio-new {
    border-style: dashed;
    text-align: left;
  }
</style>
