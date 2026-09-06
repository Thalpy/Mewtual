<script lang="ts">
  // The studio's contextual sidebar: this channel's flipnotes, scores and exports with their
  // settlement state (design-creative-suite.md section 5, "Studio").
  import { bump, ensureStudio, studio } from "./studio-state.svelte.ts";
  import { PixRaster } from "./pix-canvas.ts";
  import { DEFAULT_PALETTE } from "./studio-store.ts";
  import { FLIPNOTE_H, FLIPNOTE_W, type Settlement } from "./studio-contract.ts";

  let { me, onopen } = $props<{ me: string; onopen: () => void }>();

  // At init, not in a derived: it writes shared state once, keyed on the identity at mount.
  // svelte-ignore state_referenced_locally
  const store = ensureStudio(me);
  const objects = $derived.by(() => {
    void studio.rev;
    return Object.entries(store.index.objects)
      .filter(([, o]) => !o.deleted)
      .map(([id, o]) => ({ id, ...o, settlement: store.settlement.get(id) ?? null, root: store.objects.get(id) ?? null }));
  });
  const exportsList = $derived.by(() => {
    void studio.rev;
    const out: { id: string; title: string; bytes: number; expiry: number }[] = [];
    for (const [oid, root] of store.objects) {
      for (const [eid, e] of Object.entries(root.exports)) if (!e.deleted) out.push({ id: eid, title: `${root.title}.pixa`, bytes: e.bytes, expiry: e.expiry });
      void oid;
    }
    return out;
  });

  function chip(s: Settlement | null): { text: string; tone: "ok" | "warn" | "danger" } {
    if (!s) return { text: "new", tone: "ok" };
    if (s.gate === "fault") return { text: "fault", tone: "danger" };
    if (s.label === "settled") return { text: "settled", tone: "ok" };
    if (s.label === "rotating" || s.label.startsWith("local edits")) return { text: "rotating", tone: "warn" };
    if (s.label.startsWith("current owner")) return { text: "unconfirmed", tone: "warn" };
    if (s.label === "document full" || s.label === "storage limit reached") return { text: "full", tone: "warn" };
    return { text: s.gate, tone: "warn" };
  }

  function open(id: string) {
    studio.selected = id;
    onopen();
  }

  function newFlipnote() {
    const id = store.createFlipnote(`flipnote ${objects.length + 1}`);
    // A flipnote opens on a blank first frame, never on nothing.
    store.insertFrame(id, null, new PixRaster(FLIPNOTE_W, FLIPNOTE_H, DEFAULT_PALETTE.map((e) => ({ ...e }))).encode());
    bump();
    open(id);
  }

  function fmtBytes(n: number): string {
    return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} mib` : `${Math.round(n / 1024)} kib`;
  }

  function relDays(ts: number): string {
    if (!ts) return "keeps";
    const d = Math.ceil((ts - Date.now()) / 86_400_000);
    return d <= 0 ? "expired" : `expires ${d}d`;
  }
</script>

<h3><span>Studio</span></h3>
<ul class="channel-list studio-nav">
  {#each objects as o (o.id)}
    <li>
      <button type="button" class="studio-obj" class:active={studio.selected === o.id} onclick={() => open(o.id)}>
        <span class="studio-obj-name">
          {#if o.kind === "flipnote"}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5"></rect><path d="M4.5 2.5v11M11.5 2.5v11M1.5 6h3M1.5 10h3M11.5 6h3M11.5 10h3"></path></svg>
          {:else}
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12.5V4l6-1.5V11"></path><circle cx="4.5" cy="12.5" r="1.8"></circle><circle cx="10.5" cy="11" r="1.8"></circle></svg>
          {/if}
          <span class="nm">{o.title}</span>
        </span>
        <span class="studio-obj-meta">
          <span class="micro">
            {#if o.root}{o.root.frames.length} fr · {o.root.fps} fps{:else}score{/if}
          </span>
          <span class="studio-chip {chip(o.settlement).tone}">{chip(o.settlement).text}</span>
        </span>
      </button>
    </li>
  {/each}
</ul>
{#if exportsList.length}
  <h3><span>Exports</span></h3>
  <ul class="channel-list studio-nav">
    {#each exportsList as e (e.id)}
      <li>
        <button type="button" class="studio-obj" title="Export records are listed; fetching one is not connected yet">
          <span class="studio-obj-name">
            <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 9v4h10V9M8 2.5V10M5 7l3 3 3-3"></path></svg>
            <span class="nm">{e.title}</span>
          </span>
          <span class="studio-obj-meta"><span class="micro">{fmtBytes(e.bytes)} · {relDays(e.expiry)}</span></span>
        </button>
      </li>
    {/each}
  </ul>
{/if}
<button type="button" class="ghost small ctx-action studio-new" onclick={newFlipnote}>+ new flipnote</button>
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
  .studio-new {
    border-style: dashed;
    text-align: left;
  }
</style>
