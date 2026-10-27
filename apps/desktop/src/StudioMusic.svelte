<script module lang="ts">
  /// A Score entry of the channel index as the Music tab lists it. `where` says whether the index
  /// still shows it; a deleted one is listed only to explain a link that still points at it.
  export type MusicTrack = { id: string; title: string; createdBy: string; where: "visible" | "overflow" | "deleted" };
</script>

<script lang="ts">
  // The Flipnote Studio's MUSIC inspector tab: which Score (a "track") this flipnote plays under,
  // and what the flipnote itself brings to playback and export.
  //
  // Everything shown is read from the flipnote's own sound view (`model.score` and its register)
  // and from the channel index (`tracks`). A track's bpm, steps and readiness live in the Score
  // object, which this tab does not read: linking is `set_header score`, and the native side checks
  // that the id names a real typed Score in this channel at link time (Gate 7 handoff). Opening the
  // track is how its facts are seen; this tab never claims a readiness it has not read.
  import { FLIPNOTE_MAX_PATCHES } from "./studio-contract.ts";
  import type { StudioSoundModel } from "./studio-sound.ts";
  import type { JamPatch } from "./jam-contract.ts";
  import { instrumentName } from "./studio-sound-ui.ts";

  // The annotated-destructuring form: `$props<T>()` leaves every prop implicitly `any` under
  // svelte-check, which is how an array callback on a prop goes untyped.
  type Props = {
    model: StudioSoundModel;
    frameCount?: number;
    fps?: number;
    /// Score entries of this channel's index (visible first). The deleted ones stay listed only to
    /// explain a link that points at them; they cannot be picked.
    tracks?: readonly MusicTrack[];
    patchLibrary?: readonly { name: string; patch: JamPatch }[];
    who: (identity: string) => string;
    tint: (identity: string) => string;
    readonlyReason?: string;
    onlink: (score: string | null) => void;
    onopen: (score: string) => void;
    oncreate: () => void;
  };
  let { model, frameCount = 0, fps = 12, tracks = [], patchLibrary = [], who, tint, readonlyReason = "", onlink, onopen, oncreate }: Props = $props();

  // "change" opens the picker under a linked track; a successful link (the register value moves)
  // folds it again, which the effect below does by watching the linked id.
  let picking = $state(false);
  const linked = $derived(model.score);
  $effect(() => { void linked; picking = false; });
  const linkedTrack = $derived(linked ? tracks.find((t) => t.id === linked) ?? null : null);
  // The register holds whole `set_header` operations; the linked id is the operation's `value`.
  // A clear (`value: null`) is a real alternative too: "use this" on it unlinks on purpose.
  const linkOf = (v: { value: { op: string; value?: unknown } }): string | null =>
    v.value.op === "set_header" && typeof v.value.value === "string" ? v.value.value : null;
  const conflicts = $derived(model.scoreRegister?.conflicts ?? []);
  const selected = $derived(model.scoreRegister?.selected ?? null);
  const pickable = $derived(tracks.filter((t) => t.where !== "deleted"));
  const liveRecipes = $derived(model.patchSlots.filter((s) => !s.removed));
  const sfxRecipeNames = $derived([...new Set(model.sfx.map((s) => s.patch))].map((id) => instrumentName(model.patches[id], patchLibrary)));
  const disabled = $derived(!!readonlyReason);
  // Export waits on exactly the things the PIXA preparation refuses on: unresolved sound evidence,
  // two concurrent links, or a link to a track this device does not hold.
  const exportBlockers = $derived.by(() => {
    const out: string[] = [];
    if (!model.ready) out.push("sounds need attention");
    if (conflicts.length) out.push("two tracks chosen");
    if (linked && !linkedTrack) out.push("linked track not on this device");
    return out;
  });
  function titleOf(id: string | null): string {
    if (!id) return "none";
    const t = tracks.find((x) => x.id === id);
    return t ? (t.title || "untitled track") : `${id.slice(0, 8)}…`;
  }
  function seconds(n: number): string { return (n / Math.max(1, fps)).toFixed(2); }
</script>

<section class="music" aria-label="Music track">
  {#if readonlyReason}<div class="banner warn">{readonlyReason}</div>{/if}
  <div class="sec">soundtrack</div>
  {#if conflicts.length && selected}
    <div class="card warn">
      <span>another track chosen by {#each conflicts as c, i (c.source.opId)}{i ? ", " : ""}<b style="color: {tint(c.source.author)}">{who(c.source.author)}</b>{/each}</span>
      <span class="alt"><i style="background: {tint(selected.source.author)}"></i><span>{titleOf(linkOf(selected))} · {who(selected.source.author)}</span><span class="grow"></span><button type="button" class="lnk" disabled={disabled} onclick={() => onlink(linkOf(selected))}>keep this</button></span>
      {#each conflicts as c (c.source.opId)}
        <span class="alt"><i style="background: {tint(c.source.author)}"></i><span>{titleOf(linkOf(c))} · {who(c.source.author)}</span><span class="grow"></span><button type="button" class="lnk" disabled={disabled} onclick={() => onlink(linkOf(c))}>use this</button></span>
      {/each}
    </div>
  {/if}
  {#if linked}
    <div class="row on" class:warn={!linkedTrack}>
      <svg class="ic" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="3" width="13" height="10" rx="1"></rect><path d="M4.5 3v6M8 3v6M11.5 3v6"></path></svg>
      <span class="col">
        <span class="nm">{titleOf(linked)}</span>
        <span class="sub">{linkedTrack ? who(linkedTrack.createdBy) : "not on this device"}</span>
      </span>
      <span class="grow"></span>
      {#if linkedTrack}<span class="chip ok"><i></i><span class="micro">linked</span></span>{:else}<span class="chip warn"><i></i><span class="micro">unavailable</span></span>{/if}
    </div>
    <div class="acts">
      <button type="button" class="btn" onclick={() => onopen(linked)}>{linkedTrack ? "open track" : "read again"}</button>
      <button type="button" class="btn ghost" disabled={disabled} onclick={() => (picking = !picking)}>change</button>
      <span class="grow"></span>
      <button type="button" class="btn ghost" disabled={disabled} onclick={() => onlink(null)}>unlink</button>
    </div>
  {:else}
    <div class="row dashed"><span class="sub">no track</span></div>
  {/if}
  {#if !linked || picking}
    <div class="sec nxt">pick a track <span class="n">· this channel</span></div>
    {#each pickable as t (t.id)}
      <button type="button" class="row pick" class:on={t.id === linked} disabled={disabled || t.id === linked} onclick={() => onlink(t.id)}>
        <svg class="ic" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="3" width="13" height="10" rx="1"></rect><path d="M4.5 3v6M8 3v6M11.5 3v6"></path></svg>
        <span class="col"><span class="nm">{t.title || "untitled track"}</span><span class="sub">{who(t.createdBy)}{#if t.where === "overflow"} · overflow{/if}</span></span>
      </button>
    {:else}
      <p class="micro">no tracks in this channel yet</p>
    {/each}
    <button type="button" class="btn dashed" disabled={disabled} onclick={oncreate}>+ new track</button>
  {/if}

  <div class="sec nxt">timing</div>
  <div class="opt"><span class="lb">frames</span><span class="mono">{frameCount} at {fps} fps · {seconds(frameCount)} s</span></div>
  <div class="opt"><span class="lb">track</span><span class="mono" class:faint={!linked}>{titleOf(linked)}</span></div>

  <div class="sec nxt">recipes <span class="n">· {liveRecipes.length} of {FLIPNOTE_MAX_PATCHES}</span></div>
  <div class="bar"><i style="width: {Math.min(100, (liveRecipes.length / FLIPNOTE_MAX_PATCHES) * 100)}%"></i></div>
  <div class="opt"><span class="lb">sfx</span><span class="mono wrap" class:faint={!sfxRecipeNames.length}>{sfxRecipeNames.length ? sfxRecipeNames.join(" · ") : "none"}</span></div>

  <div class="sec nxt">export</div>
  <div class="opt">
    <i class="dot" class:ok={!exportBlockers.length} class:warn={exportBlockers.length > 0}></i>
    <span class="mono" class:okc={!exportBlockers.length} class:warnc={exportBlockers.length > 0}>{exportBlockers.length ? `waits · ${exportBlockers.join(" · ")}` : `ready · frames${model.sfx.length ? " + sounds" : ""}${linked ? " + track" : ""}`}</span>
  </div>
</section>

<style>
  .music { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.wrap { white-space: normal; overflow-wrap: anywhere; }
  .mono.faint { color: var(--faint); }
  .mono.okc { color: var(--ok); } .mono.warnc { color: var(--warn); }
  .grow { flex: 1; }
  .sec { display: flex; align-items: center; gap: 6px; padding: 7px 0 4px; font-family: var(--mono); font-size: 0.62rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); }
  .sec.nxt { border-top: 1px solid var(--border-soft); margin-top: 6px; }
  .sec .n { color: var(--faint); font-weight: 400; }
  .opt { display: flex; align-items: center; gap: 6px; }
  .opt .lb { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); width: 44px; flex: none; }
  .banner { margin: 4px 0 2px; padding: 6px 9px; border-radius: var(--r); font-size: 0.78rem; }
  .banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); color: var(--text-2); }
  .row { display: flex; align-items: center; gap: 8px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); padding: 6px 8px; color: inherit; text-align: left; width: 100%; box-sizing: border-box; }
  .row.on { border-color: var(--accent); background: var(--accent-dim); }
  .row.warn { border-color: var(--warn-brd); }
  .row.dashed { border-style: dashed; background: transparent; }
  .row.pick:disabled { opacity: 0.6; }
  .row .ic { width: 13px; height: 13px; flex: none; fill: none; stroke: var(--muted); stroke-width: 1.5; stroke-linejoin: round; }
  .row.on .ic { stroke: var(--accent); }
  .row.warn .ic { stroke: var(--warn); }
  .col { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .nm { font-family: var(--mono); font-size: 0.68rem; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { font-family: var(--mono); font-size: 0.56rem; letter-spacing: 0.06em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 7px; white-space: nowrap; }
  .chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .chip .micro { font-size: 0.52rem; }
  .chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); } .chip.ok .micro { color: var(--ok); } .chip.ok i { background: var(--ok); }
  .chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); } .chip.warn .micro { color: var(--warn); } .chip.warn i { background: var(--warn); }
  .acts { display: flex; gap: 6px; margin-top: 6px; align-items: center; }
  .btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; }
  .btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .btn.dashed { border-style: dashed; background: transparent; color: var(--muted); margin-top: 4px; }
  .btn:disabled { opacity: 0.45; }
  .card { display: flex; flex-direction: column; gap: 3px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); padding: 7px 9px; font-size: 0.78rem; color: var(--text); margin-bottom: 4px; }
  .card.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .alt { display: flex; align-items: center; gap: 6px; font-size: 0.72rem; color: var(--text-2); }
  .alt i { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .lnk { background: transparent; border: none; padding: 0; font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); text-decoration: underline; }
  .lnk:disabled { opacity: 0.45; text-decoration: none; }
  .bar { height: 4px; border-radius: 999px; background: var(--bg-elev); overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--muted); flex: none; }
  .dot.ok { background: var(--ok); } .dot.warn { background: var(--warn); }
</style>
