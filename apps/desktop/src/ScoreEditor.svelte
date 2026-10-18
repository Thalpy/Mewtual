<script lang="ts">
  // The Studio's score composer: a chromatic step grid (C4..C6 at sixteenths), a recipe rack,
  // the tempo/meter/length transport and an attention list for everything the backend flagged.
  //
  // Division of labour: this component renders the native ScoreModel and emits typed ScoreOps
  // through `onop`; it never keeps an optimistic copy of the document. Placement guards come
  // from score-editor.ts and native admission stays the authority. Everything local (page,
  // tool, chosen recipe, hold, selection) is view state and is dropped with the component,
  // which the host keys by object.
  //
  // Reading the grid: a filled block is a note onset in its recipe's colour; the block runs on
  // for the note's authored hold (a continuation never sounds on its own). A diamond with two
  // letters is a pad hit. Hatching means "kept but silent": over the 4096-cell cap or past the
  // authored length. A hollow block names a recipe the score no longer holds. A gold corner is a
  // retained concurrent value; a red edge an overlapping hold. None of these are hidden or
  // resolved here: the attention list on the score tab lets a person choose.
  import { onDestroy, untrack, type Snippet } from "svelte";
  import { JAM_MET_BPM_MIN, JAM_MET_BPM_MAX, JAM_MET_BPB_MIN, JAM_MET_BPB_MAX, type JamPatch } from "./jam-contract.ts";
  import { SCORE_MAX_STEPS, SCORE_MAX_PATCHES, SCORE_ROWS, canonicalJson, type ScoreOp } from "./studio-contract.ts";
  import type { ScoreModel } from "./studio-native.ts";
  import { groupShelves, instrumentName, libraryKey, rackLabel, samePatch, waveGlyph, type LibraryItem } from "./studio-sound-ui.ts";
  import {
    PAD_CODES, PAD_NAMES, SCORE_ALTERNATIVE_PAGE, SCORE_DIAGNOSTIC_PAGE, SCORE_ISSUE_LABELS, SCORE_PAGE_STEPS, SCORE_ROW_BASE_NOTE,
    SCORE_STEPS_PER_BEAT, SCORE_WIDE_CARD_PX, SCORE_WIDE_PAGE_STEPS, scoreAttention, scoreBarBeat, scoreCellOperation,
    scoreDiagnosticOperation, scoreDurationMs, scoreHeaderOperation, scoreHolds, scoreLastSoundingStep, scoreOccupiedSteps,
    scoreOnsetAt, scorePatchOrder, scorePatchUsage, scorePitch, scorePlayheadStep, scoreRowIsBlack, scoreSlotDiagnostic,
    scoreTimeLabel, scoreExtent, type ScoreDiagnostic, type ScoreHold,
  } from "./score-editor.ts";

  type Props = {
    model: ScoreModel | null;
    /** Non-empty disables every write; shown once as a banner. */
    readonlyReason: string;
    loading: boolean;
    error: string;
    /** Names for recipes; the saved descriptor stays the playback source. */
    patchLibrary: readonly { name: string; patch: JamPatch }[];
    /** Recipes a person can add, by shelf (presets, this device, the share). */
    library?: readonly LibraryItem[];
    /** Author label ("you" for self) and dot colour for retained values. */
    who?: (identity: string) => string;
    tint?: (identity: string) => string;
    /** Queues one typed operation; `false` means the host refused it (nothing was queued). */
    onop: (op: ScoreOp) => boolean | void;
    /** Adds a recipe to the document; resolves its id once queued. */
    onaddpatch: (patch: JamPatch) => Promise<string>;
    onplay?: () => void;
    onstop?: () => void;
    playing?: boolean;
    /** Milliseconds into document playback, or null when nothing is playing. */
    positionMs?: number | null;
    loop?: boolean;
    onloop?: (loop: boolean) => void;
    /** Play one note through the host, which gates Deafen; absent hides auditioning. */
    onaudition?: (patch: JamPatch, note: number, durationMs: number) => void;
    /** Open the soundfont popup, seeded with the chosen recipe if any. */
    onnew?: (startFrom: JamPatch | null) => void;
    onexport?: () => void;
    onshare?: () => void;
    mediaBusy?: boolean;
    /** The host's history rail, rendered under the inspector's history tab. */
    history?: Snippet;
    onnotice: (text: string, kind: "info" | "success" | "error") => void;
  };
  let {
    model, readonlyReason, loading, error, patchLibrary, library = [], who = (id: string) => id.slice(0, 8), tint = () => "var(--muted)",
    onop, onaddpatch, onplay, onstop, playing = false, positionMs = null, loop = false, onloop, onaudition, onnew, onexport, onshare,
    mediaBusy = false, history, onnotice,
  }: Props = $props();

  // --- view state --------------------------------------------------------------------------------
  let page = $state(0);
  let tab = $state<"patches" | "score" | "history">("patches");
  let tool = $state<"place" | "erase" | "audition">("place");
  let mode = $state<"note" | "drum">("note");
  let patch = $state(""); // chosen recipe id; "" means the first active one
  let hold = $state(1);
  let pad = $state(0);
  let selected = $state<string | null>(null); // a "step:row" key, live cell or not
  let expanded = $state(""); // attention item whose alternatives are open
  let attentionPage = $state(0), altPage = $state(0);
  let adding = $state(""); // library key whose add is in flight
  let armedRemove = $state("");
  let cardWidth = $state(0);
  let cardEl = $state<HTMLElement | null>(null);
  let drag = $state<{ row: number; start: number; end: number } | null>(null);
  let rootEl = $state<HTMLElement | null>(null);
  // The page width follows the card's width. Measured with a ResizeObserver where one exists;
  // without one (the unit-test DOM) the card reads as zero wide and pages 16 steps.
  $effect(() => {
    const el = cardEl;
    if (!el) return;
    cardWidth = el.clientWidth;
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => { cardWidth = el.clientWidth; });
    observer.observe(el);
    return () => observer.disconnect();
  });
  // A drag that placed a hold must swallow the click the browser fires after it; the flag is
  // cleared on a timer because that click never arrives when the pointer was released elsewhere.
  let dragged = false;
  let alive = true;
  onDestroy(() => { alive = false; });

  // --- derived views -----------------------------------------------------------------------------
  const blocked = $derived(loading || !model || !!readonlyReason);
  const size = $derived(cardWidth >= SCORE_WIDE_CARD_PX ? SCORE_WIDE_PAGE_STEPS : SCORE_PAGE_STEPS);
  const extent = $derived(model ? scoreExtent(model) : SCORE_PAGE_STEPS);
  const pages = $derived(Math.max(1, Math.ceil(extent / size)));
  const currentPage = $derived(Math.min(page, pages - 1));
  const columns = $derived(Array.from({ length: Math.max(0, Math.min(size, extent - currentPage * size)) }, (_, i) => currentPage * size + i));
  const rows = Array.from({ length: SCORE_ROWS }, (_, i) => SCORE_ROWS - 1 - i);
  const holds = $derived<Map<string, ScoreHold>>(model ? scoreHolds(model, columns) : new Map());
  const order = $derived(model ? scorePatchOrder(model) : []);
  const usage = $derived<Map<string, number>>(model ? scorePatchUsage(model) : new Map());
  // Once explicitly chosen, an unavailable recipe must not silently fall back to another sound.
  // This also holds note entry while a newly queued recipe is not yet in the projection the
  // host shows (the working projection under autosave, the native readback otherwise).
  const selectedPatch = $derived(patch ? (order.includes(patch) ? patch : "") : order[0] ?? "");
  const patchPending = $derived(!!patch && !order.includes(patch));
  const attention = $derived(model ? scoreAttention(model) : []);
  const attentionPages = $derived(Math.max(1, Math.ceil(attention.length / SCORE_DIAGNOSTIC_PAGE)));
  const currentAttentionPage = $derived(Math.min(attentionPage, attentionPages - 1));
  const attentionSlice = $derived(attention.slice(currentAttentionPage * SCORE_DIAGNOSTIC_PAGE, (currentAttentionPage + 1) * SCORE_DIAGNOSTIC_PAGE));
  // Recipe names and library membership are computed once per model/library change, never per
  // cell: naming serialises the recipe against every library entry, and a page has 800 cells.
  const names = $derived.by<Map<string, string>>(() => {
    const result = new Map<string, string>();
    for (const id of order) result.set(id, instrumentName(model!.patches[id], patchLibrary));
    return result;
  });
  const docIdOf = $derived.by<Map<string, string>>(() => {
    const result = new Map<string, string>();
    if (!model) return result;
    const byBytes = new Map<string, string>(order.map((id) => [canonicalJson(model.patches[id]), id]));
    for (const item of library) { const hit = byBytes.get(canonicalJson(item.patch)); if (hit) result.set(libraryKey(item), hit); }
    return result;
  });
  const selectedCell = $derived(model && selected ? model.cells[selected] ?? null : null);
  const selectedDiag = $derived(model && selected ? scoreSlotDiagnostic(model, "cell", selected) : null);
  const occupied = $derived<Set<number>>(model ? scoreOccupiedSteps(model) : new Set());
  const stepsPerBar = $derived(Math.max(1, model?.bpb ?? 4) * SCORE_STEPS_PER_BEAT);
  const playhead = $derived(model && playing && positionMs !== null ? scorePlayheadStep(positionMs, model.bpm, model.steps, loop) : -1);
  const shelves = $derived(groupShelves(library));
  const issueCount = $derived(attention.length);
  const bars = $derived(model ? Math.ceil(model.steps / stepsPerBar) : 0);
  const totalMs = $derived(model ? scoreDurationMs(model.steps, model.bpm) : 0);
  const cursor = $derived(playhead >= 0 ? playhead : selected ? Number(selected.split(":")[0]) : columns[0] ?? 0);
  const cursorMs = $derived(model ? (playhead >= 0 && positionMs !== null ? (loop && totalMs > 0 ? positionMs % totalMs : Math.min(positionMs, totalMs)) : scoreDurationMs(cursor, model.bpm)) : 0);

  // Follow the playhead across pages while the document plays.
  $effect(() => {
    const ph = playhead, s = size;
    if (ph < 0) return;
    untrack(() => { const target = Math.floor(ph / s); if (target !== currentPage) page = target; });
  });
  // A resize that changes the page width keeps the same first step in view.
  let sizeShown = SCORE_PAGE_STEPS;
  $effect(() => {
    const s = size;
    untrack(() => { if (s !== sizeShown) { page = Math.floor((currentPage * sizeShown) / s); sizeShown = s; } });
  });

  // --- helpers -----------------------------------------------------------------------------------
  const message = (cause: unknown) => cause instanceof Error ? cause.message : String(cause);
  const swatch = (id: string) => { const i = order.indexOf(id); return i < 0 ? "var(--faint)" : `var(--sc-p${i % 8})`; };
  const nameOf = (id: string) => names.get(id) ?? "?";
  const stepLabel = (step: number) => `step ${step + 1}`;
  function valueLabel(value: unknown): string {
    if (value === null || value === undefined) return "cleared";
    if (typeof value === "object" && "k" in (value as object)) {
      const cell = value as { k: "n"; p: string; len?: number } | { k: "d"; pad: number };
      return cell.k === "d" ? PAD_NAMES[cell.pad] ?? `pad ${cell.pad}` : `${nameOf(cell.p)} · ${cell.len ?? 1}`;
    }
    if (typeof value === "object") return instrumentName(value as JamPatch, patchLibrary);
    return String(value);
  }
  function itemLabel(item: ScoreDiagnostic): string {
    if (item.kind === "header") return item.key;
    if (item.kind === "patch") return nameOf(item.key);
    const [step, row] = item.key.split(":").map(Number);
    return `${scorePitch(row)} · ${stepLabel(step)}`;
  }
  function cellLabel(step: number, row: number): string {
    const key = `${step}:${row}`, cell = model?.cells[key], held = holds.get(key);
    const base = `${scorePitch(row)}, ${stepLabel(step)}`;
    if (cell) return `${base}: ${cell.k === "d" ? PAD_NAMES[cell.pad] : `${nameOf(cell.p)}, ${cell.len ?? 1} step${(cell.len ?? 1) === 1 ? "" : "s"}`}`;
    if (held) return `${base}: ${nameOf(held.p)} continues`;
    return `${base}: empty`;
  }
  function cellTitle(step: number, row: number, label: string): string {
    const key = `${step}:${row}`, issues = model?.readiness.cells[key], slot = model?.cellSlots[key];
    const flags = issues ? (Object.entries(issues) as [keyof typeof SCORE_ISSUE_LABELS, boolean][]).filter(([, v]) => v).map(([k]) => SCORE_ISSUE_LABELS[k]) : [];
    const parts = [label];
    if (slot?.register.conflicts.length) parts.push(`another value by ${slot.register.conflicts.map((c) => who(c.source.author)).join(", ")}`);
    if (slot && !model?.cells[key] && slot.removed && (slot.register.selected.value !== null || slot.register.conflicts.some((c) => c.value !== null))) parts.push("cleared");
    if (slot?.register.selected && model?.cells[key]) parts.push(`by ${who(slot.register.selected.source.author)}`);
    return [...parts, ...flags].join(" · ");
  }
  function cellClass(step: number, row: number): string {
    const key = `${step}:${row}`, cell = model?.cells[key], held = holds.get(key), issues = model?.readiness.cells[key], slot = model?.cellSlots[key];
    const classes: string[] = [];
    if ((step + 1) % stepsPerBar === 0) classes.push("bar"); else if ((step + 1) % SCORE_STEPS_PER_BEAT === 0) classes.push("beat");
    if (cell?.k === "n") classes.push("n", (cell.len ?? 1) === 1 ? "end" : "");
    if (cell?.k === "d") classes.push("pad");
    if (held) classes.push("held", held.last ? "end" : "");
    if (issues?.count) classes.push("over");
    if (issues?.extent || step >= (model?.steps ?? 0)) classes.push("past");
    // Readiness is keyed by the onset; a hold whose recipe is gone is hollow along its whole run.
    if (issues?.recipe || (held && model?.readiness.cells[held.onset]?.recipe)) classes.push("norecipe");
    if (issues?.overlap) classes.push("overlap");
    if (issues?.conflict || (slot?.register.conflicts.length ?? 0) > 0) classes.push("conf");
    // A clear that beat a value (its own setter or a concurrent one) leaves a mark the attention
    // list can explain; a plain clear with nothing beaten is just an empty cell.
    if (slot && !cell && !held && slot.removed && (slot.register.selected.value !== null || slot.register.conflicts.some((c) => c.value !== null))) classes.push("gone");
    if (selected === key || (selected !== null && held?.onset === selected)) classes.push("sel");
    if (drag && drag.row === row && step >= drag.start && step <= drag.end) classes.push("drag");
    if (step === playhead) classes.push("ph");
    return classes.join(" ");
  }
  function cellStyle(step: number, row: number): string {
    const key = `${step}:${row}`, cell = model?.cells[key], held = holds.get(key);
    const id = cell?.k === "n" ? cell.p : held?.p;
    return id ? `--pc: ${swatch(id)}` : "";
  }
  function rulerLabel(step: number): string {
    if (step % stepsPerBar === 0) return String(step / stepsPerBar + 1);
    return step % SCORE_STEPS_PER_BEAT === 0 ? "·" : "";
  }

  // --- writes ------------------------------------------------------------------------------------
  function send(make: () => ScoreOp): boolean {
    if (blocked) return false;
    try { return onop(make()) !== false; } catch (cause) { onnotice(message(cause), "error"); return false; }
  }
  function audition(id: string, row: number, len: number) {
    const descriptor = model?.patches[id];
    if (!onaudition || !descriptor || !model) return;
    onaudition(descriptor, SCORE_ROW_BASE_NOTE + row, Math.max(1, scoreDurationMs(len, model.bpm)));
  }
  function place(step: number, row: number, len: number) {
    if (!model) return;
    const entry = mode === "drum" ? { mode: "drum" as const, pad } : { mode: "note" as const, patch: selectedPatch, len };
    if (!send(() => scoreCellOperation(model!, step, row, entry))) return;
    selected = `${step}:${row}`;
    if (entry.mode === "note") audition(entry.patch, row, len);
  }
  function erase(step: number, row: number) {
    if (!model) return;
    const onset = scoreOnsetAt(model, step, row);
    if (!onset) return;
    const [s, r] = onset.key.split(":").map(Number);
    if (send(() => scoreCellOperation(model!, s, r, { mode: "clear" })) && selected === onset.key) selected = null;
  }
  function clearKey(key: string) {
    const [s, r] = key.split(":").map(Number);
    send(() => scoreCellOperation(model!, s, r, { mode: "clear" }));
  }
  function clickCell(step: number, row: number) {
    if (dragged) { dragged = false; return; }
    if (!model) return;
    const key = `${step}:${row}`, onset = scoreOnsetAt(model, step, row);
    if (tool === "erase") { erase(step, row); return; }
    if (tool === "audition") {
      selected = onset?.key ?? key;
      if (onset?.cell.k === "n") audition(onset.cell.p, row, onset.cell.len ?? 1);
      // An empty cell sounds for the hold a placement there would get, never past the end.
      else if (!onset && selectedPatch && step < model.steps) audition(selectedPatch, row, Math.max(1, Math.min(hold, model.steps - step)));
      return;
    }
    if (onset) { selected = onset.key; return; }
    place(step, row, mode === "note" ? hold : 1);
  }
  function contextCell(event: MouseEvent, step: number, row: number) {
    event.preventDefault();
    if (!blocked) erase(step, row);
  }
  function pointerDownCell(event: PointerEvent, step: number, row: number) {
    if (event.button !== 0 || blocked || !model || tool !== "place" || mode !== "note" || !selectedPatch) return;
    if (scoreOnsetAt(model, step, row) || step >= model.steps) return;
    drag = { row, start: step, end: step };
  }
  function dragMove(event: PointerEvent) {
    if (!drag) return;
    const el = document.elementFromPoint(event.clientX, event.clientY)?.closest("[data-score-cell]") as HTMLElement | null;
    const key = el?.dataset.scoreCell;
    if (!key) return;
    const [s, r] = key.split(":").map(Number);
    if (r !== drag.row) return; // a hold runs along one pitch
    const end = Math.max(drag.start, Math.min(s, (model?.steps ?? 1) - 1));
    if (end !== drag.end) drag = { ...drag, end };
  }
  function dragEnd() {
    if (!drag) return;
    const { row, start, end } = drag;
    drag = null;
    if (end > start) {
      dragged = true;
      setTimeout(() => { dragged = false; }, 0);
      place(start, row, end - start + 1);
    }
  }
  function header(field: "title" | "bpm" | "bpb" | "steps", event: Event) {
    send(() => scoreHeaderOperation(field, (event.currentTarget as HTMLInputElement).value));
  }
  function setSteps(next: number) {
    send(() => scoreHeaderOperation("steps", String(next)));
  }
  function setHold(next: number) {
    hold = Math.max(1, Math.min(SCORE_MAX_STEPS, Math.round(next) || 1));
  }
  function resizeSelected(delta: number) {
    if (!model || !selected || selectedCell?.k !== "n") return;
    const [step, row] = selected.split(":").map(Number), cell = selectedCell;
    const len = Math.max(1, (cell.len ?? 1) + delta);
    send(() => scoreCellOperation(model!, step, row, { mode: "note", patch: cell.p, len }));
  }
  function useAlternative(item: ScoreDiagnostic, index: number) {
    send(() => scoreDiagnosticOperation(model!, item, index));
  }
  function removePatch(id: string) {
    const used = usage.get(id) ?? 0;
    if (used > 0 && armedRemove !== id) { armedRemove = id; return; }
    armedRemove = "";
    send(() => ({ op: "remove_patch", patch: id }));
  }
  function pickPatch(id: string) { patch = id; mode = "note"; }
  async function addFromLibrary(item: { name: string; patch: JamPatch }) {
    if (blocked || adding || !model) return;
    const already = docIdOf.get(libraryKey(item)) ?? order.find((id) => samePatch(model!.patches[id], item.patch));
    if (already) { pickPatch(already); return; }
    if (order.length >= SCORE_MAX_PATCHES) { onnotice(`this score already holds ${SCORE_MAX_PATCHES} recipes`, "error"); return; }
    const key = libraryKey(item);
    adding = key;
    try {
      const id = await onaddpatch(item.patch);
      // Target identity is the host's fence (`addStudioPatch` refuses when the object, kind,
      // view or generation moved) and the host keys this component by object, so a resolved id
      // belongs to this document. The model reference itself is NOT a usable fence: queuing the
      // recipe replaces the working projection synchronously. An id the projection does not
      // (yet) hold selects nothing, so adopting it early never enables placement with it.
      if (alive) { patch = id; mode = "note"; }
    } catch (cause) {
      if (alive) onnotice(message(cause), "error");
    } finally { if (alive) adding = ""; }
  }
  function focusItem(item: ScoreDiagnostic) {
    expanded = expanded === item.id ? "" : item.id;
    altPage = 0;
    if (item.kind === "cell") {
      selected = item.key;
      page = Math.floor(Number(item.key.split(":")[0]) / size);
    } else if (item.kind === "patch" && model?.patches[item.key]) pickPatch(item.key);
  }
  function goTo(step: number) { page = Math.floor(step / size); }
  function moveSelection(dStep: number, dRow: number) {
    const [s, r] = (selected ?? `${columns[0] ?? 0}:12`).split(":").map(Number);
    const step = Math.max(0, Math.min(extent - 1, s + dStep)), row = Math.max(0, Math.min(SCORE_ROWS - 1, r + dRow));
    selected = `${step}:${row}`;
    goTo(step);
  }
  function onKey(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    const active = document.activeElement;
    if (!rootEl || !rootEl.contains(active) || !target) return;
    if (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    // Tool and page keys work anywhere in the composer. Keys that move the selection or clear a
    // cell work only while focus is on the grid card: the inspector hosts the history rail and
    // the rack, where Backspace or an arrow must never turn into an edit of the score.
    const map: Record<string, () => void> = {
      b: () => { tool = "place"; }, e: () => { tool = "erase"; }, a: () => { tool = "audition"; },
      k: () => { mode = "note"; }, d: () => { mode = "drum"; },
      "[": () => { page = Math.max(0, currentPage - 1); }, "]": () => { page = Math.min(pages - 1, currentPage + 1); },
    };
    const onGrid: Record<string, () => void> = {
      Escape: () => { selected = null; },
      ArrowLeft: () => moveSelection(-1, 0), ArrowRight: () => moveSelection(1, 0), ArrowUp: () => moveSelection(0, 1), ArrowDown: () => moveSelection(0, -1),
      Delete: () => { if (selected) { const [s, r] = selected.split(":").map(Number); erase(s, r); } },
      Backspace: () => { if (selected) { const [s, r] = selected.split(":").map(Number); erase(s, r); } },
    };
    const action = map[event.key] ?? (cardEl?.contains(active) ? onGrid[event.key] : undefined);
    if (action) { event.preventDefault(); action(); }
  }
</script>

<svelte:window onkeydown={onKey} onpointermove={drag ? dragMove : undefined} onpointerup={drag ? dragEnd : undefined} onpointercancel={drag ? dragEnd : undefined} />

{#snippet alternatives(item: ScoreDiagnostic)}
  {@const altPages = Math.max(1, Math.ceil(item.alternatives.length / SCORE_ALTERNATIVE_PAGE))}
  {@const current = Math.min(altPage, altPages - 1)}
  {@const effective = item.removed ? null : item.alternatives[0]?.value}
  <div class="sc-alts">
    {#each item.alternatives.slice(current * SCORE_ALTERNATIVE_PAGE, (current + 1) * SCORE_ALTERNATIVE_PAGE) as alt, i (alt.source.opId)}
      {@const absolute = current * SCORE_ALTERNATIVE_PAGE + i}
      <div class="sc-alt" class:cur={alt.value === effective || (alt.value === null && effective === null)}>
        <i class="who" style="background: {tint(alt.source.author)}"></i>
        <span class="t">{who(alt.source.author)} · {valueLabel(alt.value)}</span>
        <span class="grow"></span>
        <button type="button" class="st-link" data-score-alt={absolute} disabled={blocked} title={alt.value === null ? "author this clear again" : "author this value again as your own edit"} onclick={() => useAlternative(item, absolute)}>{alt.value === null ? "clear" : "use this"}</button>
      </div>
    {/each}
    {#if altPages > 1}
      <div class="sc-pager">
        <button type="button" class="st-tile" aria-label="Previous alternative page" disabled={current === 0} onclick={() => { altPage = current - 1; }}>‹</button>
        <span class="micro">{current + 1} / {altPages}</span>
        <button type="button" class="st-tile" aria-label="Next alternative page" disabled={current + 1 >= altPages} onclick={() => { altPage = current + 1; }}>›</button>
      </div>
    {/if}
  </div>
{/snippet}

<section class="sc" aria-label="Score composer" bind:this={rootEl}>
  <!-- header: the document's facts and the export actions -->
  <div class="sc-head">
    <input class="st-title" aria-label="Score title" value={model?.title ?? ""} placeholder="untitled" disabled={blocked} onchange={(e) => header("title", e)} />
    {#if model?.headers.title?.conflicts.length}<span class="chip warn" title="another title was set at the same time by {model.headers.title.conflicts.map((c) => who(c.source.author)).join(', ')}"><i></i><span class="micro">title</span></span>{/if}
    {#if model}
      <span class="chip"><span class="micro">{model.bpm} bpm</span></span>
      <span class="chip"><span class="micro">{model.bpb} / bar</span></span>
      <span class="chip"><span class="micro">{model.steps} steps · {bars} {bars === 1 ? "bar" : "bars"} · {scoreTimeLabel(totalMs)}</span></span>
      {#if model.ready && !issueCount}
        <span class="chip ok" title="plays and exports as written"><i></i><span class="micro">ready</span></span>
      {:else if model.ready}
        <button type="button" class="chip as-btn" title="plays and exports as written; retained values are waiting for a decision" onclick={() => { tab = "score"; }}><i></i><span class="micro">{issueCount} to review</span></button>
      {:else}
        <button type="button" class="chip warn as-btn" title="open the list of cells, recipes and headers that need a decision" onclick={() => { tab = "score"; }}><i></i><span class="micro">{issueCount} to fix</span></button>
      {/if}
    {/if}
    <span class="grow"></span>
    {#if onexport}<button type="button" class="st-btn" disabled={mediaBusy || playing || !model?.ready} title={model?.ready ? "save this score as a .jamtake file" : "fix the flagged items first"} onclick={onexport}>download .jamtake</button>{/if}
    {#if onshare}<button type="button" class="st-btn primary" disabled={mediaBusy || playing || !model?.ready} title={model?.ready ? "put a .jamtake of this score in the server share" : "fix the flagged items first"} onclick={onshare}>share</button>{/if}
  </div>

  {#if error}<div class="st-banner danger" role="alert">{error}</div>{/if}
  {#if readonlyReason}<div class="st-banner warn">{readonlyReason}</div>{/if}
  {#if loading}<p class="micro" role="status">reading…</p>{/if}

  {#if model}
    <div class="sc-row">
      <!-- tool rail -->
      <div class="sc-tools">
        <button type="button" class="sc-tool" class:on={tool === "place"} aria-pressed={tool === "place"} aria-label="Place" title="place [b] · click sets a note, drag sets its length" onclick={() => { tool = "place"; }}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 13l1-4 7-7 3 3-7 7-4 1z"></path><path d="M9.5 3.5l3 3"></path></svg><span class="k">b</span>
        </button>
        <button type="button" class="sc-tool" class:on={tool === "erase"} aria-pressed={tool === "erase"} aria-label="Erase" title="erase [e] · right-click erases in any tool" onclick={() => { tool = "erase"; }}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9 3l4 4-6 6H4.5L2.5 11z"></path><path d="M6 8l3.5 3.5M3 14h10"></path></svg><span class="k">e</span>
        </button>
        {#if onaudition}
          <button type="button" class="sc-tool" class:on={tool === "audition"} aria-pressed={tool === "audition"} aria-label="Audition" title="audition [a] · click a cell to hear it" onclick={() => { tool = "audition"; }}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 6v4h2.5L9 13V3L5.5 6z"></path><path d="M11 6a3 3 0 010 4"></path></svg><span class="k">a</span>
          </button>
        {/if}
        <span class="sc-sep"></span>
        <button type="button" class="sc-tool" class:on={mode === "note"} aria-pressed={mode === "note"} aria-label="Keys" data-score-mode="note" title="keys [k] · place notes with the chosen recipe" onclick={() => { mode = "note"; }}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="2" y="3" width="12" height="10" rx="1"></rect><path d="M5 3v6M8 3v6M11 3v6"></path></svg><span class="k">k</span>
        </button>
        <button type="button" class="sc-tool" class:on={mode === "drum"} aria-pressed={mode === "drum"} aria-label="Pads" data-score-mode="drum" title="pads [d] · place hits of the chosen pad" onclick={() => { mode = "drum"; }}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><rect x="2.5" y="2.5" width="4" height="4" rx="1"></rect><rect x="9.5" y="2.5" width="4" height="4" rx="1"></rect><rect x="2.5" y="9.5" width="4" height="4" rx="1"></rect><rect x="9.5" y="9.5" width="4" height="4" rx="1"></rect></svg><span class="k">d</span>
        </button>
      </div>

      <!-- the grid -->
      <div class="sc-card" bind:this={cardEl}>
        <div class="sc-grid" class:locked={blocked} role="group" aria-label="Chromatic score grid C4 to C6" style="--cols: {columns.length}">
          <span class="sc-corner"></span>
          {#each columns as step (step)}
            <span class="sc-rs" class:bar={step % stepsPerBar === 0} class:beat={step % SCORE_STEPS_PER_BEAT === 0} class:ph={step === playhead} class:past={step >= model.steps}>{rulerLabel(step)}</span>
          {/each}
          {#each rows as row (row)}
            <span class="sc-lab" class:blk={scoreRowIsBlack(row)} class:oct={row % 12 === 0}>{scorePitch(row)}</span>
            {#each columns as step (step)}
              {@const key = `${step}:${row}`}
              {@const cell = model.cells[key]}
              {@const held = holds.get(key)}
              {@const label = cellLabel(step, row)}
              <button type="button" class="sc-cell {cellClass(step, row)}" class:blk={scoreRowIsBlack(row)} data-score-cell={key} data-k={cell?.k === "d" ? PAD_CODES[cell.pad] : undefined}
                style={cellStyle(step, row)} aria-label={label} title={cellTitle(step, row, label)}
                disabled={blocked || (!cell && !held && (step >= model.steps || (mode === "note" && !selectedPatch)))}
                onclick={() => clickCell(step, row)} oncontextmenu={(e) => contextCell(e, step, row)} onpointerdown={(e) => pointerDownCell(e, step, row)}></button>
            {/each}
          {/each}
          {#if playhead >= 0 && columns.includes(playhead)}
            <span class="sc-ph" aria-hidden="true" style="--i: {playhead - columns[0]}"></span>
          {/if}
        </div>
        <div class="sc-foot">
          <button type="button" class="st-tile" aria-label="Previous score page" disabled={currentPage === 0} onclick={() => { page = currentPage - 1; }}>‹</button>
          <span class="mono">{columns.length ? `${columns[0] + 1}–${columns[columns.length - 1] + 1}` : "·"} <span class="dim">/ {model.steps}</span></span>
          <button type="button" class="st-tile" aria-label="Next score page" disabled={currentPage + 1 >= pages} onclick={() => { page = currentPage + 1; }}>›</button>
          <span class="grow"></span>
          <span class="micro">{selected ? `${scorePitch(Number(selected.split(":")[1]))} · ${stepLabel(Number(selected.split(":")[0]))}` : ""}</span>
          <span class="micro hi">{tool}{mode === "drum" ? ` · ${PAD_NAMES[pad]}` : selectedPatch ? ` · ${nameOf(selectedPatch)}` : patchPending ? " · saving recipe…" : ""}</span>
        </div>
      </div>

      <!-- inspector -->
      <div class="sc-inspector">
        <div class="sc-itabs">
          <button type="button" class="st-itab" class:on={tab === "patches"} data-score-tab="patches" onclick={() => { tab = "patches"; }}>patches</button>
          <button type="button" class="st-itab" class:on={tab === "score"} data-score-tab="score" onclick={() => { tab = "score"; }}>score{#if issueCount}<i class="dot"></i>{/if}</button>
          <button type="button" class="st-itab" class:on={tab === "history"} data-score-tab="history" onclick={() => { tab = "history"; }}>history</button>
        </div>
        <div class="sc-ibody">
          {#if tab === "patches"}
            <div class="st-sec">placing</div>
            <div class="st-opt">
              <button type="button" class="st-tile txt" class:on={mode === "note"} aria-pressed={mode === "note"} onclick={() => { mode = "note"; }}>keys</button>
              <button type="button" class="st-tile txt" class:on={mode === "drum"} aria-pressed={mode === "drum"} onclick={() => { mode = "drum"; }}>pads</button>
            </div>
            {#if mode === "note"}
              <div class="st-opt">
                <span class="lb">hold</span>
                <button type="button" class="st-tile" aria-label="Shorter hold" disabled={hold <= 1} onclick={() => setHold(hold - 1)}>−</button>
                <input class="st-input num" type="number" min="1" max={SCORE_MAX_STEPS} step="1" aria-label="Note hold in sixteenth steps" title="how many sixteenth steps a placed note holds; dragging across the grid sets it too" value={hold} oninput={(e) => setHold(Number(e.currentTarget.value))} disabled={blocked} />
                <button type="button" class="st-tile" aria-label="Longer hold" disabled={hold >= SCORE_MAX_STEPS} onclick={() => setHold(hold + 1)}>＋</button>
                <span class="micro">{hold === 1 ? "sixteenth" : hold === 2 ? "eighth" : hold === 4 ? "quarter" : hold === 8 ? "half" : hold === 16 ? "whole" : `${hold} / 16`}</span>
              </div>
            {/if}
            {#if mode === "drum"}
              <div class="sc-pads">
                {#each PAD_NAMES as name, i (name)}
                  <button type="button" class="sc-padt" class:on={pad === i} aria-pressed={pad === i} data-score-pad={i} title={name} onclick={() => { pad = i; }}>
                    <span class="code">{PAD_CODES[i]}</span><span class="nm">{name}</span>
                  </button>
                {/each}
              </div>
            {/if}

            <div class="st-sec nxt">in this score <span class="n">· {order.length} / {SCORE_MAX_PATCHES}</span></div>
            {#each order as id (id)}
              {@const issues = model.readiness.patches[id]}
              {@const used = usage.get(id) ?? 0}
              <div class="sc-prow" class:on={selectedPatch === id}>
                <button type="button" class="pick" data-doc-patch={id} aria-pressed={selectedPatch === id} title="{nameOf(id)} · {used} {used === 1 ? 'note' : 'notes'}" onclick={() => pickPatch(id)}>
                  <i class="sw" style="background: {swatch(id)}"></i>
                  <svg viewBox="0 0 26 12" aria-hidden="true"><path d={waveGlyph(model.patches[id])}></path></svg>
                  <span class="nm">{nameOf(id)}</span>
                  <span class="grow"></span>
                  {#if issues?.count}<span class="chip warn"><span class="micro">over</span></span>{/if}
                  {#if issues?.conflict}<span class="chip warn"><span class="micro">conflict</span></span>{/if}
                  <span class="micro">{used}</span>
                </button>
                <button type="button" class="x" class:armed={armedRemove === id} data-remove-patch={id} aria-label="Remove {nameOf(id)}" disabled={blocked}
                  title={armedRemove === id ? `${used} ${used === 1 ? "note keeps" : "notes keep"} this recipe's id and ${used === 1 ? "is" : "are"} flagged until cleared or re-placed` : used ? `remove · ${used} ${used === 1 ? "note uses" : "notes use"} it` : "remove"}
                  onclick={() => removePatch(id)}>{armedRemove === id ? "remove anyway" : "×"}</button>
              </div>
            {:else}
              <p class="micro wrap">no recipes yet · add one below</p>
            {/each}
            {#if patchPending}<div class="st-card accent" data-pending-patch={patch}><span class="micro">saving recipe…</span></div>{/if}

            {#each shelves as group (group.shelf)}
              <div class="sc-shelf">{group.label} · {group.items.length}</div>
              <div class="sc-rack">
                {#each group.items as item (libraryKey(item))}
                  {@const inDoc = docIdOf.get(libraryKey(item)) ?? null}
                  <button type="button" class="rk" class:indoc={!!inDoc} class:on={!!inDoc && selectedPatch === inDoc} class:busy={adding === libraryKey(item)} data-lib-patch={item.name}
                    aria-busy={adding === libraryKey(item)} disabled={blocked || !!adding} title={inDoc ? `${item.name} · already in this score` : `${item.name} · ${group.label} · click to add`} onclick={() => addFromLibrary(item)}>
                    <svg viewBox="0 0 26 12" aria-hidden="true"><path d={waveGlyph(item.patch)}></path></svg>
                    <span class="nm">{rackLabel(item.name)}</span>
                  </button>
                {/each}
              </div>
            {/each}
            {#if onnew}
              <div class="sc-rack one">
                <button type="button" class="rk add" title="design a new recipe in the soundfont editor" disabled={blocked} onclick={() => onnew?.(selectedPatch ? model!.patches[selectedPatch] : null)}>
                  <svg viewBox="0 0 26 12" aria-hidden="true"><path d="M13 2v8M9 6h8"></path></svg><span class="nm">+ new…</span>
                </button>
              </div>
            {/if}

            {#if selected}
              {@const [sStep, sRow] = selected.split(":").map(Number)}
              <div class="st-sec nxt">{scorePitch(sRow)} <span class="n">· {stepLabel(sStep)}</span></div>
              {#if selectedCell?.k === "n"}
                <div class="st-opt">
                  <i class="sw" style="background: {swatch(selectedCell.p)}"></i>
                  <span class="t">{nameOf(selectedCell.p)}</span>
                  <span class="grow"></span>
                  <button type="button" class="st-tile" aria-label="Shorten this note" disabled={blocked || (selectedCell.len ?? 1) <= 1} onclick={() => resizeSelected(-1)}>−</button>
                  <span class="mono strong">{selectedCell.len ?? 1}</span>
                  <button type="button" class="st-tile" aria-label="Lengthen this note" disabled={blocked} onclick={() => resizeSelected(1)}>＋</button>
                  <button type="button" class="st-tile" aria-label="Clear this note" title="clear" disabled={blocked} onclick={() => clearKey(selected!)}>×</button>
                </div>
              {:else if selectedCell?.k === "d"}
                <div class="st-opt">
                  <span class="sc-padt mini"><span class="code">{PAD_CODES[selectedCell.pad]}</span></span>
                  <span class="t">{PAD_NAMES[selectedCell.pad]}</span>
                  <span class="grow"></span>
                  <button type="button" class="st-tile" aria-label="Clear this pad hit" title="clear" disabled={blocked} onclick={() => clearKey(selected!)}>×</button>
                </div>
              {:else if selectedDiag?.removed}
                <div class="st-opt"><span class="micro">cleared</span></div>
              {/if}
              {#if model.cellSlots[selected]?.register.selected && selectedCell}
                <p class="micro wrap">by <i class="who" style="background: {tint(model.cellSlots[selected].register.selected.source.author)}"></i>{who(model.cellSlots[selected].register.selected.source.author)}</p>
              {/if}
              {#if selectedDiag && (selectedDiag.alternatives.length > 1 || selectedDiag.issues.length)}
                {#if selectedDiag.issues.length}<div class="st-opt">{#each selectedDiag.issues as issue (issue)}<span class="chip warn"><i></i><span class="micro">{SCORE_ISSUE_LABELS[issue as keyof typeof SCORE_ISSUE_LABELS] ?? issue}</span></span>{/each}</div>{/if}
                {#if selectedDiag.alternatives.length > 1}{@render alternatives(selectedDiag)}{/if}
              {/if}
            {/if}
          {:else if tab === "score"}
            {#if model.ready && !attention.length}
              <div class="st-sec">ready</div>
              <p class="micro wrap">plays and exports as written</p>
            {:else}
              <div class="st-sec">{attention.length} {model.ready ? "to review" : "to fix"}</div>
            {/if}
            {#each attentionSlice as item (item.id)}
              <button type="button" class="sc-att" class:on={expanded === item.id} data-score-evidence={item.id} aria-expanded={expanded === item.id} onclick={() => focusItem(item)}>
                <span class="kind">{item.kind === "cell" ? "◼" : item.kind === "patch" ? "♪" : "⌂"}</span>
                <span class="lb">{itemLabel(item)}</span>
                <span class="grow"></span>
                {#each item.issues as issue (issue)}<span class="chip warn"><span class="micro">{SCORE_ISSUE_LABELS[issue as keyof typeof SCORE_ISSUE_LABELS] ?? issue}</span></span>{/each}
                {#if item.removed}<span class="chip"><span class="micro">cleared</span></span>{/if}
                {#if item.alternatives.length > 1}<span class="chip warn"><span class="micro">{item.alternatives.length} values</span></span>{/if}
              </button>
              {#if expanded === item.id}
                <div class="sc-attbody">
                  {#if item.alternatives.length > 1 || item.removed}{@render alternatives(item)}{/if}
                  <div class="acts">
                    {#if item.kind === "cell"}<button type="button" class="st-btn ghost" disabled={blocked} onclick={() => clearKey(item.key)}>clear</button>{/if}
                    {#if item.kind === "patch" && model.patches[item.key]}<button type="button" class="st-btn ghost" disabled={blocked} onclick={() => send(() => ({ op: "remove_patch", patch: item.key }))}>remove recipe</button>{/if}
                  </div>
                </div>
              {/if}
            {/each}
            {#if attentionPages > 1}
              <div class="sc-pager">
                <button type="button" class="st-tile" aria-label="Previous evidence page" disabled={currentAttentionPage === 0} onclick={() => { attentionPage = currentAttentionPage - 1; }}>‹</button>
                <span class="micro">{currentAttentionPage + 1} / {attentionPages}</span>
                <button type="button" class="st-tile" aria-label="Next evidence page" disabled={currentAttentionPage + 1 >= attentionPages} onclick={() => { attentionPage = currentAttentionPage + 1; }}>›</button>
              </div>
            {/if}
          {:else}
            {@render history?.()}
          {/if}
        </div>
      </div>
    </div>

    <!-- transport -->
    <div class="sc-transport">
      <button type="button" class="sc-tool sm" aria-label="First page" disabled={currentPage === 0} onclick={() => { page = 0; }}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M8 3 2 8l6 5zM14 3 8 8l6 5z"></path></svg></button>
      <button type="button" class="sc-tool sm" aria-label="Previous page" disabled={currentPage === 0} onclick={() => { page = currentPage - 1; }}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg></button>
      {#if playing}
        <button type="button" class="sc-tool play on" title="stop [space]" aria-label="stop" onclick={onstop} disabled={!onstop}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 3.5h9v9h-9z"></path></svg></button>
      {:else}
        <button type="button" class="sc-tool play" title={model.ready ? "play [space]" : "fix the flagged items to play"} aria-label="play" onclick={onplay} disabled={loading || !model.ready || !onplay}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg></button>
      {/if}
      <button type="button" class="sc-tool sm" aria-label="Next page" disabled={currentPage + 1 >= pages} onclick={() => { page = currentPage + 1; }}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg></button>
      <button type="button" class="sc-tool sm" aria-label="Last page" disabled={currentPage + 1 >= pages} onclick={() => { page = pages - 1; }}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M2 3l6 5-6 5zM8 3l6 5-6 5z"></path></svg></button>
      <button type="button" class="sc-tool sm" class:on={loop} aria-pressed={loop} aria-label="Loop playback" title={loop ? "looping the whole score" : "play once"} disabled={playing || !onloop} onclick={() => onloop?.(!loop)}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8a5 5 0 015-5h4M13 8a5 5 0 01-5 5H4"></path><path d="M10.5 1.5 12 3l-1.5 1.5M5.5 11.5 4 13l1.5 1.5"></path></svg></button>
      <span class="mono strong">{cursor + 1} <span class="dim">/ {model.steps}</span></span>
      <span class="micro">{scoreTimeLabel(cursorMs)} <span class="dim">of {scoreTimeLabel(totalMs)}</span> · bar {scoreBarBeat(cursor, model.bpb).bar} · beat {scoreBarBeat(cursor, model.bpb).beat}</span>
      <span class="grow"></span>
      <label class="sc-field" title="tempo · {JAM_MET_BPM_MIN} to {JAM_MET_BPM_MAX}{model.headers.bpm?.conflicts.length ? ' · another value by ' + model.headers.bpm.conflicts.map((c) => who(c.source.author)).join(', ') : ''}" class:conf={model.headers.bpm?.conflicts.length}>
        <input type="number" aria-label="Score BPM" min={JAM_MET_BPM_MIN} max={JAM_MET_BPM_MAX} step="1" value={model.bpm} disabled={blocked} onchange={(e) => header("bpm", e)} /><span class="micro">bpm</span>
      </label>
      <label class="sc-field" title="beats per bar · {JAM_MET_BPB_MIN} to {JAM_MET_BPB_MAX}{model.headers.bpb?.conflicts.length ? ' · another value by ' + model.headers.bpb.conflicts.map((c) => who(c.source.author)).join(', ') : ''}" class:conf={model.headers.bpb?.conflicts.length}>
        <input type="number" aria-label="Score beats per bar" min={JAM_MET_BPB_MIN} max={JAM_MET_BPB_MAX} step="1" value={model.bpb} disabled={blocked} onchange={(e) => header("bpb", e)} /><span class="micro">/ bar</span>
      </label>
      <label class="sc-field" title="length in sixteenth steps · 1 to {SCORE_MAX_STEPS}{model.headers.steps?.conflicts.length ? ' · another value by ' + model.headers.steps.conflicts.map((c) => who(c.source.author)).join(', ') : ''}" class:conf={model.headers.steps?.conflicts.length}>
        <input type="number" aria-label="Score steps" min="1" max={SCORE_MAX_STEPS} step="1" value={model.steps} disabled={blocked} onchange={(e) => header("steps", e)} /><span class="micro">steps</span>
      </label>
      <button type="button" class="st-btn ghost" title="add one bar of {stepsPerBar} steps" disabled={blocked || model.steps + stepsPerBar > SCORE_MAX_STEPS} onclick={() => setSteps(model!.steps + stepsPerBar)}>+ bar</button>
      <!-- Trim only ever shortens: with cells past the end the length is a decision, not a fit. -->
      <button type="button" class="st-btn ghost" title="end the score after its last sounding step" disabled={blocked || scoreLastSoundingStep(model) < 0 || scoreLastSoundingStep(model) + 1 >= model.steps} onclick={() => setSteps(Math.max(1, scoreLastSoundingStep(model!) + 1))}>trim</button>
    </div>
    <div class="sc-ovw" role="group" aria-label="Score overview">
      {#each Array.from({ length: extent }, (_, s) => s) as s (s)}
        <button type="button" class="sc-ov" class:has={occupied.has(s)} class:win={columns.includes(s)} class:ph={s === playhead} class:past={s >= model.steps} class:bar={(s + 1) % stepsPerBar === 0} aria-label="Go to {stepLabel(s)}" onclick={() => goTo(s)}></button>
      {/each}
    </div>
  {:else if !loading && !error}
    <p class="micro wrap">pick a score on the left</p>
  {/if}
</section>

<style>
  /* Recipe swatches and the pad colour are data colours, not theme colours: eight recipes must
     stay apart under every preset and livery, and a pad must never be confused with a recipe.
     Like the piano's ivory and ebony, they are the only literal colours in this component. */
  .sc { --sc-p0: #6fb3e6; --sc-p1: #d08ad8; --sc-p2: #5fc9c0; --sc-p3: #f3b562; --sc-p4: #b7a6ff; --sc-p5: #e6dfc8; --sc-p6: #8ccf6f; --sc-p7: #ef8fc4; --sc-pad: #f0a07a;
    display: flex; flex-direction: column; flex: 1; min-height: 0; min-width: 0; gap: 6px; }
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .micro.wrap { white-space: normal; line-height: 1.5; margin: 3px 0 0; }
  .micro.hi { color: var(--accent-hi); }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .dim { color: var(--faint); }
  .grow { flex: 1; }
  .t { font-size: 0.78rem; color: var(--text); }
  svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; flex: none; }
  svg.solid { fill: currentColor; stroke: none; }
  button { font: inherit; color: inherit; }
  button:not(:disabled) { cursor: pointer; }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }

  /* Header: the same bar the flipnote editor wears, flush with the surface strip above. */
  .sc-head { display: flex; align-items: center; gap: 0.6rem; padding: 0.4rem 0.6rem; background: var(--panel); border-bottom: 1px solid var(--border); margin: -0.3rem -0.5rem 0.2rem; flex-wrap: wrap; }
  .st-title { font-size: 0.95rem; font-weight: 600; color: var(--text); background: transparent; border: 1px solid transparent; padding: 2px 4px; width: 9rem; font-family: inherit; }
  .st-title:hover, .st-title:focus { border-color: var(--border); background: var(--bg-elev); }
  .st-title:disabled { color: var(--text-2); }
  .chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; background: transparent; }
  .chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); display: inline-block; }
  .chip .micro { color: var(--muted); }
  .chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); } .chip.ok .micro { color: var(--ok); } .chip.ok i { background: var(--ok); }
  .chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); } .chip.warn .micro { color: var(--warn); } .chip.warn i { background: var(--warn); }
  .chip.as-btn { padding: 2px 9px; }
  .st-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; }
  .st-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn:disabled { opacity: 0.45; }
  .st-link { background: none; border: none; padding: 0; color: var(--accent-hi); font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.06em; text-transform: uppercase; }
  .st-link:disabled { opacity: 0.45; }
  .st-banner { margin: 0.2rem 0; padding: 6px 10px; border-radius: var(--r); font-size: 0.78rem; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; color: var(--text-2); }
  .st-banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); }
  .st-banner.danger { background: var(--danger-dim); border: 1px solid var(--danger-brd); }
  .st-card { display: flex; flex-direction: column; gap: 3px; padding: 6px 9px; border-radius: var(--r); border: 1px solid var(--border); background: var(--bg-elev); font-size: 0.78rem; color: var(--text-2); }
  .st-card.accent { border-color: color-mix(in oklab, var(--accent) 40%, var(--panel)); background: var(--accent-dim); }
  .acts { display: flex; gap: 6px; margin-top: 4px; flex-wrap: wrap; }

  /* Editor row: tools, the grid card, the inspector. */
  .sc-row { flex: 1; min-height: 0; display: flex; gap: 10px; padding: 6px 0; align-items: stretch; }
  .sc-tools { display: flex; flex-direction: column; gap: 4px; flex: none; }
  .sc-tool { width: 32px; height: 32px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: flex; align-items: center; justify-content: center; position: relative; padding: 0; }
  .sc-tool.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .sc-tool:disabled { opacity: 0.4; }
  .sc-tool .k { position: absolute; right: 2px; bottom: 0; font-family: var(--mono); font-size: 0.5rem; color: var(--faint); }
  .sc-tool.on .k { color: var(--accent-hi); }
  .sc-tool.sm { width: 26px; height: 26px; } .sc-tool.sm svg { width: 11px; height: 11px; }
  .sc-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin: 0 4px; }
  .sc-tool.play svg { width: 14px; height: 14px; }
  .sc-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  .sc-sep { height: 1px; background: var(--border-soft); margin: 4px 2px; }

  /* The card hugs its 25 rows and scrolls inside when the window is shorter than they are. */
  .sc-card { flex: 1; min-width: 0; align-self: flex-start; max-height: 100%; display: flex; flex-direction: column; gap: 6px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); padding: 10px; overflow: auto; scrollbar-gutter: stable; }
  .sc-grid { position: relative; display: grid; grid-template-columns: 34px repeat(var(--cols), minmax(0, 1fr)); grid-template-rows: 16px repeat(25, 16px); border: 1px solid var(--border); background: var(--bg-0); user-select: none; }
  .sc-grid.locked .sc-cell { cursor: default; }
  .sc-corner { border-bottom: 1px solid var(--border); border-right: 1px solid var(--border); background: var(--panel); }
  .sc-rs { font-family: var(--mono); font-size: 0.56rem; color: var(--faint); line-height: 16px; padding-left: 3px; border-bottom: 1px solid var(--border); overflow: hidden; white-space: nowrap; }
  .sc-rs.beat { color: var(--muted); }
  .sc-rs.bar { color: var(--text-2); border-left: 1px solid var(--border); }
  .sc-rs.past { background: repeating-linear-gradient(45deg, var(--border-soft) 0 2px, transparent 2px 6px); }
  .sc-rs.ph { color: var(--accent-hi); }
  .sc-lab { font-family: var(--mono); font-size: 0.56rem; color: var(--faint); text-align: right; padding-right: 5px; line-height: 16px; border-right: 1px solid var(--border); background: var(--panel); white-space: nowrap; }
  .sc-lab.oct { color: var(--text-2); }
  .sc-lab.blk { background: color-mix(in oklab, var(--panel) 55%, var(--bg-0)); }
  .sc-cell { position: relative; min-width: 0; padding: 0; border: none; border-right: 1px solid var(--border-soft); border-bottom: 1px solid var(--border-soft); border-radius: 0; background: var(--panel); touch-action: none; }
  .sc-cell.blk { background: color-mix(in oklab, var(--panel) 55%, var(--bg-0)); }
  .sc-cell.beat { border-right-color: var(--border); }
  .sc-cell.bar { border-right-color: color-mix(in oklab, var(--text-2) 35%, var(--border)); }
  .sc-cell:disabled { cursor: default; }
  .sc-cell:not(:disabled):hover { box-shadow: inset 0 0 0 1px var(--accent); }
  /* A note: the onset starts the block, continuations carry it, the last step rounds it off. */
  .sc-cell.n::before, .sc-cell.held::before { content: ""; position: absolute; top: 2px; bottom: 2px; left: 0; right: 0; background: var(--pc); }
  .sc-cell.n::before { left: 2px; border-radius: 2px 0 0 2px; }
  .sc-cell.held::before { left: -1px; }
  .sc-cell.end::before { right: 2px; border-radius: 0 2px 2px 0; }
  .sc-cell.n.end::before { border-radius: 2px; }
  /* A pad hit: a diamond with its two-letter code. */
  .sc-cell.pad::before { content: ""; position: absolute; left: 50%; top: 50%; width: 11px; height: 11px; margin: -5.5px 0 0 -5.5px; background: var(--sc-pad); clip-path: polygon(50% 0, 100% 50%, 50% 100%, 0 50%); }
  .sc-cell.pad::after { content: attr(data-k); position: absolute; inset: 0; font-family: var(--mono); font-size: 0.42rem; font-weight: 700; line-height: 15px; text-align: center; color: var(--bg-0); letter-spacing: 0; }
  /* Kept but silent: over the cap or past the authored length. */
  .sc-cell.over::before, .sc-cell.past.n::before, .sc-cell.past.held::before { background: repeating-linear-gradient(45deg, var(--faint) 0 1px, transparent 1px 4px); outline: 1px dashed var(--faint); outline-offset: -1px; }
  .sc-cell.past:not(.n):not(.held):not(.pad) { background: repeating-linear-gradient(45deg, var(--border-soft) 0 2px, transparent 2px 6px) var(--bg-0); }
  .sc-cell.over.pad::before { background: var(--faint); }
  /* Names a recipe the score no longer holds. */
  .sc-cell.norecipe::before { background: transparent; outline: 1px dashed var(--muted); outline-offset: -1px; }
  .sc-cell.overlap { box-shadow: inset 0 0 0 1px var(--danger); }
  .sc-cell.conf { background-image: linear-gradient(225deg, var(--warn) 0 5px, transparent 5.5px); }
  .sc-cell.gone::after { content: "×"; position: absolute; inset: 0; font-family: var(--mono); font-size: 0.6rem; line-height: 15px; text-align: center; color: var(--faint); }
  .sc-cell.drag { box-shadow: inset 0 0 0 1px var(--accent); background-color: var(--accent-dim); }
  .sc-cell.sel { box-shadow: inset 0 0 0 2px var(--accent); z-index: 1; }
  .sc-cell.ph { background-color: color-mix(in oklab, var(--accent) 12%, var(--panel)); }
  .sc-ph { position: absolute; top: 0; bottom: 0; width: 1px; left: calc(34px + (100% - 34px) * var(--i) / var(--cols)); background: var(--accent); pointer-events: none; }
  .sc-foot { display: flex; align-items: center; gap: 8px; }
  .st-tile { display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 26px; padding: 0; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); font-family: var(--mono); font-size: 0.66rem; text-transform: uppercase; letter-spacing: 0.04em; }
  .st-tile.txt { width: auto; padding: 0 8px; }
  .st-tile.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tile:disabled { opacity: 0.45; }

  /* Inspector */
  .sc-inspector { flex: 0 0 260px; min-width: 240px; display: flex; flex-direction: column; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); overflow: hidden; }
  .sc-itabs { display: flex; border-bottom: 1px solid var(--border); }
  .st-itab { flex: 1; padding: 7px 0; border: none; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; gap: 5px; }
  .st-itab.on { color: var(--text); border-bottom-color: var(--accent); }
  .st-itab .dot { width: 5px; height: 5px; border-radius: 50%; background: var(--warn); display: inline-block; }
  .sc-ibody { display: flex; flex-direction: column; gap: 4px; padding: 4px 12px 10px; overflow-y: auto; min-height: 0; }
  .st-sec { padding: 6px 0 4px; font-family: var(--mono); font-size: 0.62rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); }
  .st-sec.nxt { border-top: 1px solid var(--border-soft); margin-top: 6px; }
  .st-sec .n { color: var(--faint); font-weight: 400; }
  .st-opt { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; row-gap: 4px; }
  .st-opt .lb { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); }
  .st-input { background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.72rem; padding: 3px 6px; }
  .st-input.num { width: 3.2rem; height: 26px; text-align: center; }
  .sw { width: 12px; height: 12px; border-radius: 3px; flex: none; display: inline-block; }
  .who { width: 8px; height: 8px; border-radius: 50%; flex: none; display: inline-block; margin-right: 4px; vertical-align: middle; }
  .sc-pads { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 4px; margin-top: 4px; }
  .sc-padt { height: 34px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 0; min-width: 0; }
  .sc-padt .code { font-family: var(--mono); font-size: 0.66rem; font-weight: 700; color: var(--sc-pad); line-height: 1; }
  .sc-padt .nm { font-family: var(--mono); font-size: 0.5rem; color: var(--faint); text-transform: uppercase; letter-spacing: 0.04em; line-height: 1.2; margin-top: 2px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sc-padt.on { border-color: var(--sc-pad); background: color-mix(in oklab, var(--sc-pad) 14%, var(--panel)); }
  .sc-padt.mini { height: 20px; width: 26px; display: inline-flex; }
  .sc-prow { display: flex; align-items: center; gap: 2px; }
  .sc-prow .pick { flex: 1; min-width: 0; display: flex; align-items: center; gap: 7px; padding: 4px 6px; border-radius: var(--r); border: 1px solid transparent; background: transparent; text-align: left; }
  .sc-prow .pick svg { width: 22px; height: 10px; stroke-width: 1.4; color: var(--muted); }
  .sc-prow .pick .nm { font-size: 0.78rem; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sc-prow .pick:hover { background: var(--bg-elev); }
  .sc-prow.on .pick { background: var(--accent-dim); border-color: color-mix(in oklab, var(--accent) 40%, var(--panel)); }
  .sc-prow .x { width: 22px; height: 22px; flex: none; padding: 0; border: 1px solid transparent; border-radius: var(--r); background: transparent; color: var(--faint); font-family: var(--mono); font-size: 0.7rem; }
  .sc-prow .x:hover { color: var(--danger); border-color: var(--danger-brd); }
  .sc-prow .x.armed { width: auto; padding: 0 7px; font-size: 0.58rem; letter-spacing: 0.06em; text-transform: uppercase; color: var(--danger); border-color: var(--danger-brd); background: var(--danger-dim); }
  .sc-prow .x:disabled { opacity: 0.45; }
  .sc-shelf { font-family: var(--mono); font-size: 0.56rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); padding: 6px 0 2px; display: flex; align-items: center; gap: 6px; white-space: nowrap; }
  .sc-shelf::after { content: ""; flex: 1; height: 1px; background: var(--border-soft); }
  .sc-rack { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; }
  .sc-rack.one { grid-template-columns: repeat(3, minmax(0, 1fr)); margin-top: 4px; }
  .rk { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 5px 4px 4px; border: 1px solid var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); min-width: 0; position: relative; }
  .rk:hover { background: var(--bg-elev); color: var(--text); }
  .rk svg { width: 26px; height: 12px; stroke-width: 1.4; }
  .rk .nm { font-family: var(--mono); font-size: 0.52rem; letter-spacing: 0.08em; text-transform: uppercase; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rk.indoc { opacity: 0.55; }
  .rk.on, .rk.on:hover { opacity: 1; background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .rk.add { border-style: dashed; color: var(--faint); }
  .rk.busy { border-color: var(--accent); }
  .rk:disabled { opacity: 0.45; }
  .sc-alts { display: flex; flex-direction: column; gap: 3px; margin-top: 4px; }
  .sc-alt { display: flex; align-items: center; gap: 6px; padding: 4px 7px; border-radius: var(--r); border: 1px solid var(--border-soft); background: var(--bg-elev); font-size: 0.76rem; color: var(--text-2); }
  .sc-alt .who { margin: 0; }
  .sc-alt.cur { border-color: var(--border); color: var(--text); }
  .sc-alt .t { font-size: 0.76rem; color: inherit; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sc-pager { display: flex; align-items: center; gap: 8px; justify-content: center; padding: 4px 0; }
  .sc-att { display: flex; align-items: center; gap: 6px; width: 100%; padding: 5px 7px; border: 1px solid transparent; border-radius: var(--r); background: transparent; text-align: left; font-size: 0.78rem; color: var(--text-2); min-width: 0; }
  .sc-att:hover { background: var(--bg-elev); }
  .sc-att.on { background: var(--bg-elev); border-color: var(--border); color: var(--text); }
  .sc-att .kind { color: var(--faint); font-size: 0.7rem; width: 12px; text-align: center; flex: none; }
  .sc-att .lb { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sc-att .chip { padding: 0 6px; } .sc-att .chip .micro { font-size: 0.52rem; }
  .sc-attbody { padding: 2px 7px 6px; border-left: 2px solid var(--border); margin: 0 0 4px 6px; }

  /* Transport */
  .sc-transport { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; padding-top: 2px; }
  .sc-field { display: inline-flex; align-items: center; gap: 5px; height: 26px; padding: 0 7px 0 2px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); }
  .sc-field.conf { border-color: var(--warn-brd); background: var(--warn-dim); }
  .sc-field input { width: 3.4rem; height: 22px; background: transparent; border: none; color: var(--text); font-family: var(--mono); font-size: 0.72rem; font-weight: 600; text-align: right; padding: 0 2px; }
  .sc-field input:disabled { color: var(--text-2); }
  .sc-ovw { display: flex; height: 18px; border: 1px solid var(--border); background: var(--bg-0); border-radius: var(--r); overflow: hidden; }
  .sc-ov { flex: 1; min-width: 0; padding: 0; border: none; border-right: 1px solid var(--border-soft); border-radius: 0; background: transparent; position: relative; }
  .sc-ov.bar { border-right-color: var(--border); }
  .sc-ov.has::after { content: ""; position: absolute; left: 20%; right: 20%; top: 6px; height: 5px; border-radius: 1px; background: var(--muted); }
  .sc-ov.win { background: var(--accent-dim); }
  .sc-ov.win.has::after { background: var(--text-2); }
  .sc-ov.past { background: repeating-linear-gradient(45deg, var(--border-soft) 0 2px, transparent 2px 6px); }
  .sc-ov.ph { box-shadow: inset 1px 0 0 var(--accent); }
  .sc-ov:hover { background: var(--bg-elev); }
</style>
