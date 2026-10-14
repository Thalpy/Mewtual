<script lang="ts">
  import { onDestroy } from "svelte";
  import type { JamPatch } from "./jam-contract.ts";
  import { JAM_MET_BPM_MIN, JAM_MET_BPM_MAX, JAM_MET_BPB_MIN, JAM_MET_BPB_MAX } from "./jam-contract.ts";
  import { SCORE_MAX_STEPS, SCORE_MAX_PATCHES, SCORE_ROWS, type ScoreOp } from "./studio-contract.ts";
  import type { ScoreModel } from "./studio-native.ts";
  import {
    SCORE_PAGE_STEPS, SCORE_DIAGNOSTIC_PAGE, SCORE_ALTERNATIVE_PAGE, scoreCellOperation,
    scoreHeaderOperation, scorePitch, scorePage, scoreCellLabel, scoreContinuations,
    scoreDiagnostics, scoreDiagnosticOperation,
  } from "./score-editor.ts";

  let { model, readonlyReason, loading, error, patchLibrary, onop, onaddpatch,
    onplay, onstop, playing = false, onnotice } = $props<{
    model: ScoreModel | null; readonlyReason: string; loading: boolean; error: string;
    patchLibrary: readonly { name: string; patch: JamPatch }[];
    onop: (op: ScoreOp) => void; onaddpatch: (patch: JamPatch) => Promise<string>;
    onplay?: () => void; onstop?: () => void; playing?: boolean;
    onnotice: (text: string, kind: "info" | "success" | "error") => void;
  }>();

  // Selection/drafts are local view state, never an optimistic replacement for native content.
  let page = $state(0), mode = $state<"note" | "drum">("note"), patch = $state(""), hold = $state(1), pad = $state(0);
  let library = $state(""), adding = $state(false), slotPage = $state(0), inspected = $state(""), alternativePage = $state(0);
  let alive = true;
  onDestroy(() => { alive = false; });
  const blocked = $derived(loading || !model || !!readonlyReason);
  const pages = $derived(Math.ceil((model?.steps ?? 16) / SCORE_PAGE_STEPS));
  const currentPage = $derived(Math.min(page, pages - 1));
  const columns = $derived(model ? scorePage(model.steps, currentPage) : []);
  const continuations = $derived(model ? scoreContinuations(model, currentPage) : new Set<string>());
  const patches = $derived(Object.keys(model?.patches ?? {}).sort());
  // Once explicitly chosen, an unavailable recipe must not silently fall back to another sound.
  // This also holds note entry while a newly queued recipe awaits its native readback.
  const selectedPatch = $derived(patch ? patches.includes(patch) ? patch : "" : patches[0] ?? "");
  const diagnostics = $derived(model ? scoreDiagnostics(model) : []);
  const diagnosticPages = $derived(Math.max(1, Math.ceil(diagnostics.length / SCORE_DIAGNOSTIC_PAGE)));
  const currentSlotPage = $derived(Math.min(slotPage, diagnosticPages - 1));
  const visibleSlots = $derived(diagnostics.slice(currentSlotPage * SCORE_DIAGNOSTIC_PAGE, (currentSlotPage + 1) * SCORE_DIAGNOSTIC_PAGE));
  const inspection = $derived(diagnostics.find(item => item.id === inspected) ?? null);
  const alternativePages = $derived(Math.max(1, Math.ceil((inspection?.alternatives.length ?? 0) / SCORE_ALTERNATIVE_PAGE)));
  const currentAlternativePage = $derived(Math.min(alternativePage, alternativePages - 1));
  const alternatives = $derived(inspection?.alternatives.slice(currentAlternativePage * SCORE_ALTERNATIVE_PAGE, (currentAlternativePage + 1) * SCORE_ALTERNATIVE_PAGE) ?? []);

  function send(make: () => ScoreOp) {
    if (blocked) return;
    try { onop(make()); } catch (cause) { onnotice(cause instanceof Error ? cause.message : String(cause), "error"); }
  }
  function clickCell(step: number, row: number) {
    if (!model) return;
    send(() => scoreCellOperation(model!, step, row, model!.cells[`${step}:${row}`] ? { mode: "clear" } : mode === "drum" ? { mode, pad } : { mode, patch: selectedPatch, len: hold }));
  }
  function header(field: "title" | "bpm" | "bpb" | "steps", event: Event) {
    send(() => scoreHeaderOperation(field, (event.currentTarget as HTMLInputElement).value));
  }
  async function addPatch() {
    const entry = patchLibrary[Number(library)];
    if (blocked || adding || library === "" || !entry || patches.length >= SCORE_MAX_PATCHES) return;
    const before = model;
    adding = true;
    try {
      const id = await onaddpatch(entry.patch);
      // The parent keys this component by the object. An additional reference fence avoids a
      // late completion selecting a recipe in a refreshed/replaced view. The recipe chooser
      // still exposes the read-back value; a successful queue alone never enables note entry.
      if (alive && model === before) patch = id;
    } catch (cause) {
      if (alive) onnotice(cause instanceof Error ? cause.message : String(cause), "error");
    } finally { if (alive) adding = false; }
  }
  function inspect(id: string) { inspected = id; alternativePage = 0; }
</script>

<section class="score-editor" aria-label="Score composer">
  <div class="score-toolbar">
    <span class="eyebrow">Music · sixteenth-step composer</span>
    <span class="grow"></span>
    {#if playing}<button type="button" onclick={onstop} disabled={!onstop}>stop</button>
    {:else}<button type="button" class="primary" onclick={onplay} disabled={loading || !model?.ready || !onplay}>play locally</button>{/if}
  </div>
  {#if error}<p class="warning" role="alert">{error}</p>{/if}
  {#if readonlyReason}<p class="warning">{readonlyReason}</p>{/if}
  {#if loading}<p role="status">Reading native score…</p>{/if}
  {#if model}
    <div class="score-headers">
      <label class="title">Title <input aria-label="Score title" value={model.title} onchange={(event) => header("title", event)} disabled={blocked} /></label>
      <label>BPM <input aria-label="Score BPM" type="number" min={JAM_MET_BPM_MIN} max={JAM_MET_BPM_MAX} step="1" value={model.bpm} onchange={(event) => header("bpm", event)} disabled={blocked} /></label>
      <label>Beats / bar <input aria-label="Score beats per bar" type="number" min={JAM_MET_BPB_MIN} max={JAM_MET_BPB_MAX} step="1" value={model.bpb} onchange={(event) => header("bpb", event)} disabled={blocked} /></label>
      <label>Steps <input aria-label="Score steps" type="number" min="1" max={SCORE_MAX_STEPS} step="1" value={model.steps} onchange={(event) => header("steps", event)} disabled={blocked} /></label>
    </div>
    <div class="score-entry">
      <button type="button" class:on={mode === "note"} aria-pressed={mode === "note"} onclick={() => { mode = "note"; }}>note</button>
      <button type="button" class:on={mode === "drum"} aria-pressed={mode === "drum"} onclick={() => { mode = "drum"; }}>drum</button>
      {#if mode === "note"}
        <label class="recipe">Recipe <select aria-label="Score recipe" value={selectedPatch} onchange={(event) => { patch = event.currentTarget.value; }} disabled={blocked || !patches.length}>
          {#if !selectedPatch}<option value="">{patch ? "chosen recipe is not active yet" : "add a recipe first"}</option>{/if}
          {#each patches as id}<option value={id}>{id}</option>{/each}
        </select></label>
        <label>Hold <input aria-label="Note hold in sixteenth steps" type="number" min="1" max={model.steps} step="1" bind:value={hold} disabled={blocked} /> × 1/16</label>
      {:else}
        <label>Pad <select aria-label="Drum pad" bind:value={pad} disabled={blocked}>{#each Array.from({ length: 10 }, (_, i) => i) as n}<option value={n}>{n}</option>{/each}</select></label>
      {/if}
    </div>
    <div class="score-entry">
      <label class="recipe">Patch library <select aria-label="Patch library recipe" bind:value={library} disabled={blocked || adding}>
        <option value="">choose a saved synth recipe</option>
        {#each patchLibrary as entry, index}<option value={String(index)}>{entry.name}</option>{/each}
      </select></label>
      <button type="button" onclick={addPatch} disabled={blocked || adding || library === "" || patches.length >= SCORE_MAX_PATCHES}>{adding ? "saving recipe…" : "add recipe to score"}</button>
      <span class="micro">{patches.length} / {SCORE_MAX_PATCHES} active recipes</span>
    </div>
    <p class="hint">Click a filled onset to clear it. A line continues its authored hold; it is not another note. New holds must fit the score and cannot overlap the same pitch. Shrinking Steps retains out-of-range cells for cleanup below.</p>
    <div class="score-toolbar">
      <button type="button" aria-label="Previous score page" disabled={currentPage === 0} onclick={() => { page = currentPage - 1; }}>←</button>
      <span class="micro">Steps {columns[0] + 1}–{columns.at(-1)! + 1} / {model.steps} · page {currentPage + 1} / {pages}</span>
      <button type="button" aria-label="Next score page" disabled={currentPage + 1 >= pages} onclick={() => { page = currentPage + 1; }}>→</button>
    </div>
    <div class="score-scroll">
      <table class="score-grid" aria-label="Chromatic score grid C4 to C6">
        <thead><tr><th scope="col">pitch</th>{#each columns as step}<th scope="col" class:beat={step % 4 === 0}>{step + 1}</th>{/each}</tr></thead>
        <tbody>{#each Array.from({ length: SCORE_ROWS }, (_, index) => SCORE_ROWS - 1 - index) as row}
          <tr><th scope="row">{scorePitch(row)}</th>{#each columns as step}
            {@const key = `${step}:${row}`}
            {@const cell = model.cells[key]}
            {@const slot = model.cellSlots[key]}
            {@const issues = model.readiness.cells[key]}
            <td class:beat={step % 4 === 0}><button type="button" data-score-cell={key} class:filled={!!cell} class:held={continuations.has(key)} class:problem={issues && Object.values(issues).some(Boolean)} class:cleared={slot?.removed}
              aria-label={`${scorePitch(row)}, step ${step + 1}: ${cell ? scoreCellLabel(cell) : continuations.has(key) ? "held from an earlier onset" : slot?.removed ? "cleared" : "empty"}`}
              title={cell ? scoreCellLabel(cell) : continuations.has(key) ? "Continuation: clear or shorten its earlier onset before adding a note here" : "Click to author a cell"}
              disabled={blocked || (!cell && mode === "note" && !selectedPatch)} onclick={() => clickCell(step, row)}>{cell ? cell.k === "d" ? `d${cell.pad}` : cell.len ?? 1 : continuations.has(key) ? "—" : slot?.removed ? "×" : ""}</button></td>
          {/each}</tr>
        {/each}</tbody>
      </table>
    </div>
    <div class="readiness" class:warning={!model.ready} role="status">{model.ready ? "Resolved score is ready for local playback/export." : "Score is not ready for playback/export. Retained conflicts, overflow, missing recipes, overlapping holds or out-of-length cells need attention."}</div>
    <details class="score-evidence">
      <summary>Retained evidence · {diagnostics.length} header / cell / recipe slots{model.readiness.headerConflicts ? " · header conflicts" : ""}</summary>
      <p class="hint">Raw selected values are not necessarily effective: any concurrent clear/remove wins. Choosing a value authors a fresh Save against the current state; it does not replay its old signature. Every slot and source remains inspectable, including overflow and removals.</p>
      <div class="score-toolbar">
        <button type="button" aria-label="Previous evidence page" disabled={currentSlotPage === 0} onclick={() => { slotPage = currentSlotPage - 1; }}>←</button>
        <span class="micro">Evidence page {currentSlotPage + 1} / {diagnosticPages}</span>
        <button type="button" aria-label="Next evidence page" disabled={currentSlotPage + 1 >= diagnosticPages} onclick={() => { slotPage = currentSlotPage + 1; }}>→</button>
      </div>
      <div class="slot-list">{#each visibleSlots as item (item.id)}
        <button type="button" class:on={inspection?.id === item.id} data-score-evidence={item.id} onclick={() => inspect(item.id)}>
          <span>{item.kind} {item.key}</span><span>{item.removed ? "removed · " : ""}{item.alternatives.length} value(s){item.issues.length ? ` · ${item.issues.join(", ")}` : ""}</span>
        </button>
      {/each}</div>
      {#if inspection}
        <section class="inspection" aria-label="Retained score alternatives">
          <h4>{inspection.kind} {inspection.key}</h4>
          <p>{inspection.removed ? "Effective removal wins this slot." : "The first value is the raw Automerge selection."} {inspection.issues.join(" · ")}</p>
          {#if inspection.kind === "cell"}<button type="button" disabled={blocked} onclick={() => send(() => { const [step, row] = inspection!.key.split(":").map(Number); return scoreCellOperation(model!, step, row, { mode: "clear" }); })}>clear this coordinate</button>{/if}
          {#if inspection.kind === "patch"}<button type="button" disabled={blocked} onclick={() => send(() => ({ op: "remove_patch", patch: inspection!.key }))}>remove this recipe</button>{/if}
          {#each alternatives as alternative, index (alternative.source.opId)}
            {@const absolute = currentAlternativePage * SCORE_ALTERNATIVE_PAGE + index}
            <article class="alternative">
              <span class="eyebrow">{absolute === 0 ? "raw selected" : "concurrent alternative"}</span>
              <pre>{JSON.stringify(alternative.value, null, 2)}</pre>
              <dl><dt>Operation</dt><dd>{alternative.source.opId}</dd><dt>Author</dt><dd>{alternative.source.author}</dd><dt>Nonce</dt><dd>{alternative.source.nonce}</dd></dl>
              <button type="button" disabled={blocked} onclick={() => send(() => scoreDiagnosticOperation(model!, inspection!, absolute))}>{alternative.value === null ? "keep removal" : "use this value"}</button>
            </article>
          {/each}
          <div class="score-toolbar"><button type="button" aria-label="Previous alternative page" disabled={currentAlternativePage === 0} onclick={() => { alternativePage = currentAlternativePage - 1; }}>←</button><span class="micro">Values page {currentAlternativePage + 1} / {alternativePages}</span><button type="button" aria-label="Next alternative page" disabled={currentAlternativePage + 1 >= alternativePages} onclick={() => { alternativePage = currentAlternativePage + 1; }}>→</button></div>
        </section>
      {/if}
    </details>
  {:else if !loading}<p class="hint">Select a score to compose music. No fixture or local substitute is used when a native read fails.</p>{/if}
</section>

<style>
  .score-editor { display: flex; flex-direction: column; gap: 9px; min-width: 0; padding: 10px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); }
  .score-toolbar, .score-entry, .score-headers { display: flex; flex-wrap: wrap; align-items: center; gap: 7px; }
  .grow { flex: 1; }
  .eyebrow, .micro, button, input, select, th { font-family: var(--mono); }
  .eyebrow, .micro { font-size: 0.64rem; color: var(--muted); }
  .eyebrow { letter-spacing: 0.08em; text-transform: uppercase; }
  button { min-height: 27px; padding: 3px 8px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-size: 0.65rem; }
  button:not(:disabled) { cursor: pointer; }
  button:disabled { opacity: 0.45; }
  button.on { border-color: var(--accent); background: var(--accent-dim); color: var(--accent-hi); }
  button.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  label { display: inline-flex; align-items: center; gap: 5px; color: var(--text-2); font-size: 0.7rem; }
  label.title, label.recipe { flex: 1; min-width: 170px; }
  input, select { min-width: 0; padding: 4px 6px; border: 1px solid var(--border); border-radius: var(--r); color: var(--text); background: var(--bg-elev); font-size: 0.7rem; }
  input[type="number"] { width: 55px; }
  .title input, .recipe select { flex: 1; width: 90px; }
  p { margin: 0; font-size: 0.72rem; }
  .hint { color: var(--muted); line-height: 1.5; }
  .warning { padding: 7px; border: 1px solid var(--warn, var(--accent)); border-radius: var(--r); color: var(--text); background: var(--bg-elev); }
  .score-scroll { overflow: auto; max-height: 550px; }
  .score-grid { border-collapse: collapse; width: 100%; min-width: 450px; table-layout: fixed; }
  .score-grid th:first-child { width: 34px; }
  .score-grid th { font-size: 0.62rem; font-weight: normal; color: var(--muted); padding: 3px 0; }
  .score-grid td { border: 1px solid var(--border-soft); padding: 1px; }
  .score-grid .beat { border-left: 2px solid var(--border); }
  .score-grid button { width: 100%; height: 19px; min-height: 19px; padding: 0; border: 1px solid transparent; border-radius: 2px; font-size: 0.58rem; }
  .score-grid button.filled { background: var(--accent); color: var(--on-accent); }
  .score-grid button.held { background: var(--accent-dim); color: var(--accent-hi); }
  .score-grid button.problem { border-color: var(--warn, var(--text)); border-style: dashed; }
  .score-grid button.cleared { color: var(--muted); }
  .readiness { padding: 7px; color: var(--muted); font-size: 0.7rem; }
  summary { cursor: pointer; font-size: 0.74rem; color: var(--text-2); }
  .score-evidence { border-top: 1px solid var(--border); padding-top: 8px; }
  .score-evidence > * { margin-bottom: 8px; }
  .slot-list { display: flex; flex-direction: column; gap: 3px; }
  .slot-list button { display: flex; justify-content: space-between; flex-wrap: wrap; gap: 4px; text-align: left; overflow-wrap: anywhere; }
  .inspection { padding: 8px; border: 1px solid var(--border); border-radius: var(--r); min-width: 0; }
  h4 { margin: 0 0 5px; overflow-wrap: anywhere; font-size: 0.75rem; }
  .alternative { margin: 8px 0; padding: 8px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; font-size: 0.66rem; max-height: 240px; overflow: auto; }
  dl { display: grid; grid-template-columns: 60px 1fr; gap: 3px; font-family: var(--mono); font-size: 0.6rem; }
  dt { color: var(--muted); } dd { margin: 0; overflow-wrap: anywhere; }
</style>
