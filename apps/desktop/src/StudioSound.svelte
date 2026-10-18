<script lang="ts">
  // The Flipnote Studio's SOUND inspector tab, plus its evidence mode for the History tab.
  //
  // Edit mode (history = false), top to bottom: a read-only banner, a readiness banner, the
  // selected frame's markers, the "add a sound" form (recipe rack, piano, length, buttons) and the
  // document-wide "all sounds" list. Evidence mode (history = true) lists recipes with their usage,
  // the markers that need cleanup, and the raw retained records.
  //
  // Two invariants the host relies on:
  // - picking a LIBRARY tile never writes to the document. The recipe reaches the document only
  //   when "add to frame"/"save changes" is pressed (`commit`), so browsing recipes cannot
  //   pollute the 64-slot recipe list;
  // - async steps require the same frame/form generation and a mounted component. File reads
  //   also require their captured model; commit permits onpatch's expected projection replacement
  //   so the paired marker can still be added. The host keys this component by document identity:
  //   switching documents destroys it, and an old marker must never land in the new document.
  import { onDestroy, untrack } from "svelte";
  import type { JamPatch } from "./jam-contract.ts";
  import { FLIPNOTE_MAX_PATCHES, FLIPNOTE_MAX_SFX } from "./studio-contract.ts";
  import type { SfxView, SoundPatchView, StudioSoundModel } from "./studio-sound.ts";
  import { EMOJI_SOUND_MAX_BYTES, parseEmojiSoundJson, type EmojiSound } from "./emoji-sound.ts";
  import { parseJamPatchJson } from "./jam-patch.ts";
  import {
    ALL_SOUNDS_PAGE, ALTERNATIVES_PAGE, DEFAULT_LENGTH_MS, EVIDENCE_PAGE, LENGTH_PRESETS, NOTE_MAX, NOTE_MIN, type LibraryItem,
    clampNote, groupShelves, importName, instrumentName, noteName, pageSlice, patchUsage, pianoKeys, rackLabel, registerOf,
    libraryKey, samePatch, secondsLabel, sortSfx, soundAttention, soundLengthMs, waveGlyph,
  } from "./studio-sound-ui.ts";

  let { model, frame, frameNumber = 0, frameIndexOf, who, tint, readonlyReason = "", library = [], history = false,
    onpatch, onsfx, onremove, onremovepatch, onaudition, onnew, onnotice } = $props<{
    model: StudioSoundModel;
    /** Selected frame id, "" when the flipnote has no frames. */
    frame: string;
    /** 1-based number of `frame` for the heading; 0 when none. */
    frameNumber?: number;
    /** 1-based timeline number of any frame id; 0 when that frame is gone. */
    frameIndexOf: (frameId: string) => number;
    /** Author label ("you" for self) for a marker's source author. */
    who: (identity: string) => string;
    /** CSS colour for that author's dot. */
    tint: (identity: string) => string;
    /** Non-empty disables editing; shown once in a banner. */
    readonlyReason?: string;
    /** Recipes NOT yet in the document, by shelf. */
    library?: readonly LibraryItem[];
    /** Evidence mode for the History tab. */
    history?: boolean;
    /** Adds a recipe to the document; resolves its id. */
    onpatch: (patch: JamPatch) => Promise<string>;
    /** Author/edit a marker; the host fills the frame and a fresh id when `id` is absent. */
    onsfx: (input: { id?: string; patch: string; note: number; durationMs: number }) => void;
    onremove: (id: string) => void;
    onremovepatch: (patch: string) => void;
    /** Play one note through the host, which gates Deafen. */
    onaudition: (patch: JamPatch, note: number, durationMs: number) => void;
    /** Open the soundfont popup, seeded with the selected recipe if any. */
    onnew: (startFrom: JamPatch | null) => void;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
  }>();

  // --- form state -------------------------------------------------------------------------------
  // A document recipe is referenced by id; a library/imported recipe is carried by value until
  // `commit` writes it. `null` means nothing picked (also "pick one" for a dangling marker).
  type Selection = { kind: "doc"; id: string } | { kind: "lib"; name: string; patch: JamPatch } | null;
  let selection = $state<Selection>(null);
  let note = $state(60);
  // The length field keeps the author's raw text so a partial entry ("0.") is not wiped by a
  // re-render; `duration` is the validated integer-millisecond view of it ("" = not valid yet).
  let lengthText = $state(String(DEFAULT_LENGTH_MS / 1000));
  let editing = $state(""); // marker id being edited, "" when authoring a new one
  let emoji = $state<EmojiSound | null>(null); // an emoji import constrains pitch and locks length
  // The last imported file, kept as its own shelf so the selection stays visible in the rack. An
  // emoji import remembers its constraints so re-picking the tile restores them.
  let imported = $state<{ name: string; patch: JamPatch; emoji: EmojiSound | null } | null>(null);
  let busy = $state(false); // a file read or `onpatch` is in flight
  let page = $state(0), recipePage = $state(0), cleanupPage = $state(0), evidencePage = $state(0), altPage = $state(0);
  let fileInput = $state<HTMLInputElement | null>(null);
  let disposed = false;
  // Bumped on every frame change; an async step started under an older generation must not write.
  let formGeneration = 0;
  onDestroy(() => { disposed = true; });
  // A frame change abandons whatever was being authored: the edited marker id, the import lock,
  // the picked recipe and the length all belonged to the previous frame's form. The pitch is kept
  // (it is a preference, not a frame fact). `untrack` keeps the reset from re-running on its own
  // writes.
  $effect(() => { void frame; untrack(() => { formGeneration++; editing = ""; emoji = null; imported = null; selection = null; lengthText = String(DEFAULT_LENGTH_MS / 1000); }); });

  // --- derived views ----------------------------------------------------------------------------
  const locked = $derived(!!readonlyReason);
  const duration = $derived<number | "">(lengthText.trim() === "" ? "" : (soundLengthMs(Number(lengthText)) ?? ""));
  const here = $derived<SfxView[]>(model.sfx.filter((sfx: SfxView) => sfx.frame === frame));
  const docTiles = $derived<{ id: string; descriptor: JamPatch }[]>(Object.entries<JamPatch>(model.patches).map(([id, descriptor]) => ({ id, descriptor })));
  const shelves = $derived(groupShelves(library));
  const overPatch = $derived(new Set<string>(model.readiness.patchOverflow));
  const overSfx = $derived(new Set<string>(model.readiness.sfxOverflow));
  const goneFrame = $derived(new Set<string>(model.readiness.missingFrames));
  const goneRecipe = $derived(new Set<string>(model.readiness.missingPatches));
  const attention = $derived(soundAttention(model));
  /** The recipe the form would author with, resolved through the document for a doc selection. */
  const descriptor = $derived<JamPatch | null>(selection === null ? null : selection.kind === "doc" ? (model.patches[selection.id] ?? null) : selection.patch);
  const lo = $derived(emoji ? emoji.lo : NOTE_MIN), hi = $derived(emoji ? emoji.hi : NOTE_MAX);
  const keys = $derived(pianoKeys(registerOf(note)));
  const canHear = $derived(!locked && descriptor !== null && duration !== "");
  const canCommit = $derived(canHear && !busy && !!frame);
  const all = $derived(sortSfx(model.sfx, frameIndexOf));
  const allPage = $derived(pageSlice(all, page, ALL_SOUNDS_PAGE));
  const usage = $derived(patchUsage(model.sfx));
  const liveRecipes = $derived(docTiles.length);
  const recipes = $derived(pageSlice<SoundPatchView>(model.patchSlots, recipePage, EVIDENCE_PAGE));
  const cleanup = $derived<SfxView[]>(model.sfx.filter((sfx: SfxView) => goneFrame.has(sfx.id) || goneRecipe.has(sfx.id) || overSfx.has(sfx.id)));
  const cleanupSlice = $derived(pageSlice(cleanup, cleanupPage, EVIDENCE_PAGE));
  const records = $derived(Object.entries<StudioSoundModel["records"][string]>(model.records).sort(([a], [b]) => a.localeCompare(b)));
  const recordSlice = $derived(pageSlice(records, evidencePage, EVIDENCE_PAGE));
  const editingName = $derived(descriptor ? instrumentName(descriptor, library) : "");

  /**
   * A library selection whose bytes the document already holds. Recipe ids are the hash of the
   * canonical bytes, so equal bytes are the same id: committing such a selection references the
   * document's slot instead of writing the recipe again. This is also how a selection survives
   * its own add: the host re-reads the document, the tile moves from a library shelf to the
   * "in this flipnote" shelf, and the match keeps it highlighted there.
   */
  const docMatch = $derived.by<string | null>(() => {
    const picked = selection; // narrowed once; the closure below cannot see a narrowed `$state`
    if (picked?.kind !== "lib") return null;
    return docTiles.find(slot => samePatch(slot.descriptor, picked.patch))?.id ?? null;
  });
  const isDoc = (id: string) => selection?.kind === "doc" ? selection.id === id : docMatch === id;
  const isLib = (item: { name: string; patch: JamPatch }) => selection?.kind === "lib" && selection.name === item.name && samePatch(selection.patch, item.patch);
  /** The row label: recipe name and note; "?" stands in for a recipe the document no longer has. */
  const label = (sfx: { patch: string; note: number }) => `${model.patches[sfx.patch] ? instrumentName(model.patches[sfx.patch], library) : "?"} · ${noteName(sfx.note)}`;

  // --- picking ----------------------------------------------------------------------------------
  // Picking any tile other than the imported one drops the import's constraints: they describe
  // that file, not the next recipe. The typed length stays, so a 0.5 s choice survives a swap.
  function pickDoc(id: string) { if (locked) return; selection = { kind: "doc", id }; emoji = null; }
  function pickLib(item: { name: string; patch: JamPatch }) { if (locked) return; selection = { kind: "lib", name: item.name, patch: item.patch }; emoji = null; }
  function pickImported() {
    if (locked || !imported) return;
    selection = { kind: "lib", name: imported.name, patch: imported.patch }; emoji = imported.emoji;
    if (imported.emoji) { note = clampNote(note, imported.emoji.lo, imported.emoji.hi); lengthText = String(imported.emoji.len / 1000); }
  }
  function setNote(next: number) { if (locked) return; note = clampNote(next, lo, hi); }
  function setLength(ms: number) { if (locked || emoji) return; lengthText = String(ms / 1000); }

  function edit(sfx: SfxView) {
    if (locked) return;
    emoji = null; editing = sfx.id; note = sfx.note; lengthText = String(sfx.durationMs / 1000);
    // A dangling marker opens with nothing picked: "pick one" is literally that.
    selection = model.patches[sfx.patch] ? { kind: "doc", id: sfx.patch } : null;
  }
  function cancel() { editing = ""; selection = null; emoji = null; lengthText = String(DEFAULT_LENGTH_MS / 1000); }

  /** A conflict alternative becomes the marker's value under the SAME id, so no duplicate is born. */
  function choose(id: string, value: { patch: string; note: number; duration_ms: number }) {
    if (locked) return;
    onsfx({ id, patch: value.patch, note: value.note, durationMs: value.duration_ms });
  }
  function hear() { if (!canHear || descriptor === null || duration === "") return; onaudition(descriptor, note, duration); }
  function hearRow(sfx: SfxView) { const recipe = model.patches[sfx.patch]; if (locked || !recipe) return; onaudition(recipe, sfx.note, sfx.durationMs); }

  // --- committing -------------------------------------------------------------------------------
  function validInput(): number | null {
    const ms = duration;
    if (!Number.isInteger(note) || note < NOTE_MIN || note > NOTE_MAX || ms === "" || !Number.isInteger(ms) || ms < 1 || ms > 600000 ||
      (emoji && (note < emoji.lo || note > emoji.hi || ms !== emoji.len))) { onnotice("Choose a valid pitch and length.", "error"); return null; }
    return ms;
  }
  async function commit() {
    if (!canCommit || selection === null) return;
    const ms = validInput(); if (ms === null) return;
    const id = editing, picked = selection, pitch = note;
    const inDocument = picked.kind === "doc" ? picked.id : docMatch;
    if (inDocument !== null) {
      // The recipe is already in the document: one synchronous write, nothing to fence.
      onsfx({ ...(id ? { id } : {}), patch: inDocument, note: pitch, durationMs: ms }); editing = ""; return;
    }
    if (picked.kind !== "lib") return;
    // A library/imported recipe enters the document only now. The marker is written after the
    // recipe id resolves, and only if nothing moved underneath: a different frame, a newer form
    // generation, a read-only turn or an unmounted component all mean the author is no longer
    // looking at what they pressed "add" for. A re-read of the SAME document is not a move: the
    // model reference changes on every remote edit, and refusing the marker then would spend a
    // recipe slot (the recipe is already written) without the sound the press asked for.
    const sourceFrame = frame, generation = formGeneration; busy = true;
    try {
      const patchId = await onpatch(picked.patch);
      if (disposed) return;
      if (sourceFrame !== frame || generation !== formGeneration || locked) {
        onnotice("The frame or document changed before the sound was added; add it again.", "warn"); return;
      }
      onsfx({ ...(id ? { id } : {}), patch: patchId, note: pitch, durationMs: ms }); editing = "";
      // The selection stays a library one: the document is re-read asynchronously, and until then
      // the id is not in `model.patches`. Once it is, `docMatch` resolves the same selection to
      // the document slot and a second add references it instead of writing the recipe twice.
    } catch (e) { if (!disposed) onnotice(String(e), "error"); }
    finally { busy = false; }
  }

  // --- importing --------------------------------------------------------------------------------
  // `.json` is an emoji sound sidecar (recipe + note range + authored hold); anything else is a
  // bare `.jampatch`. Both share the 4 KiB bound, checked before any bytes are read. Neither path
  // touches the document: the parsed recipe becomes a pending library selection like any tile.
  async function importFile(file: File | undefined) {
    if (!file || busy || locked) return;
    if (file.size > EMOJI_SOUND_MAX_BYTES) { onnotice("sound file exceeds 4KiB", "error"); return; }
    const source = model, sourceFrame = frame, generation = formGeneration; busy = true;
    try {
      const text = await file.text();
      if (disposed || source !== model || sourceFrame !== frame || generation !== formGeneration || locked) return;
      const name = importName(file.name);
      if (/\.jampatch$/i.test(file.name ?? "")) {
        const parsed = parseJamPatchJson(text); if (!parsed.ok) throw new Error(parsed.error);
        imported = { name, patch: parsed.patch, emoji: null };
      } else {
        const parsed = parseEmojiSoundJson(text);
        imported = { name, patch: parsed.patch, emoji: parsed };
      }
      pickImported();
    } catch (e) { if (!disposed) onnotice(String(e), "error"); }
    finally { busy = false; }
  }
  function onFileChange(input: HTMLInputElement) {
    const file = input.files?.[0];
    input.value = ""; // the same file chosen twice must fire change again
    void importFile(file);
  }
</script>

{#snippet noteIcon(colour: string)}
  <svg class="ic" viewBox="0 0 16 16" fill="none" stroke={colour} stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 12.5V4l6-1.5V11" /><circle cx="4.5" cy="12.5" r="1.8" /><circle cx="10.5" cy="11" r="1.8" /></svg>
{/snippet}
{#snippet chip(text: string)}
  <span class="st-chip warn"><i></i><span class="micro">{text}</span></span>
{/snippet}
{#snippet pager(current: number, pages: number, prev: () => void, next: () => void)}
  {#if pages > 1}
    <div class="pager">
      <button type="button" class="st-tile" disabled={current === 0} title="Previous page" aria-label="Previous page" onclick={prev}>‹</button>
      <span class="mono">{current + 1} / {pages}</span>
      <button type="button" class="st-tile" disabled={current + 1 >= pages} title="Next page" aria-label="Next page" onclick={next}>›</button>
    </div>
  {/if}
{/snippet}
{#snippet tile(glyph: string, name: string, on: boolean, title: string, pick: () => void, over: boolean, attrs: Record<string, string>)}
  <button type="button" class="rk" class:on class:gone={over} {title} aria-pressed={on} disabled={locked} onclick={pick} {...attrs}>
    <svg viewBox="0 0 26 12" aria-hidden="true"><path d={glyph} /></svg>
    <span class="nm">{name}</span>
    {#if over}<span class="st-chip warn tiny"><span class="micro">over</span></span>{/if}
  </button>
{/snippet}

<section class="sound" aria-label={history ? "Sound evidence" : "Frame sound editor"}>
{#if !history}
  {#if readonlyReason}<div class="st-banner warn"><b>{readonlyReason}</b></div>{/if}
  {#if !model.ready}
    <div class="st-banner warn">
      {#if attention.markers}<b>{attention.markers} {attention.markers === 1 ? "sound needs" : "sounds need"} attention</b>{/if}
      {#if attention.recipes}<b>{attention.recipes} {attention.recipes === 1 ? "recipe" : "recipes"} over the cap</b>{/if}
      {#if !attention.markers && !attention.recipes}<b>sound evidence needs attention · see history</b>{/if}
    </div>
  {/if}

  <!-- The selected frame's markers. -->
  <div class="st-sec">{frame ? `frame ${frameNumber} · sounds` : "sounds"} <span class="n">· {here.length}</span></div>
  {#each here as sfx (sfx.id)}
    {@const recipe = model.patches[sfx.patch]}
    <div class="snd" class:cur={editing === sfx.id} class:warn={!recipe}>
      {@render noteIcon(editing === sfx.id ? "var(--accent)" : recipe ? "var(--muted)" : "var(--warn)")}
      <div class="col">
        <div class="nm">{label(sfx)}</div>
        {#if recipe}<div class="sub">{who(sfx.source.author)} · {secondsLabel(sfx.durationMs)}</div>
        {:else}<div class="sub">{@render chip("missing recipe")}</div>{/if}
      </div>
      <span class="grow"></span>
      <button type="button" class="pl" title="Hear this sound" aria-label="Hear this sound" disabled={locked || !recipe} onclick={() => hearRow(sfx)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 2.5 12.5 8 3.5 13.5z" /></svg>
      </button>
      {#if recipe}<button type="button" class="x" title="Edit this sound" disabled={locked} onclick={() => edit(sfx)}>edit</button>
      {:else}<button type="button" class="st-link" title="Choose a recipe for this sound" disabled={locked} onclick={() => edit(sfx)}>pick one</button>{/if}
      <button type="button" class="x danger" title="Remove this sound" aria-label="Remove this sound" data-remove-sfx={sfx.id} disabled={locked} onclick={() => onremove(sfx.id)}>✕</button>
    </div>
    {#if sfx.conflicts.length}
      <!-- Concurrent values for one marker id: the selected one shows above; every alternative
           can replace it under the same id so choosing never creates a second marker. -->
      <div class="st-card warn">
        <span>another value by {sfx.conflicts.map((alt) => who(alt.source.author)).join(", ")}</span>
        <div class="st-alt"><i style:background={tint(sfx.source.author)}></i><span>{label(sfx)} · {secondsLabel(sfx.durationMs)}</span><span class="grow"></span>
          <button type="button" class="st-link" disabled={locked} onclick={() => choose(sfx.id, { patch: sfx.patch, note: sfx.note, duration_ms: sfx.durationMs })}>keep this</button></div>
        {#each sfx.conflicts as alt (alt.source.opId)}
          {#if alt.value.op === "set_sfx"}
            {@const v = alt.value}
            <div class="st-alt"><i style:background={tint(alt.source.author)}></i><span>{label(v)} · {secondsLabel(v.duration_ms)}</span><span class="grow"></span>
              <button type="button" class="st-link" disabled={locked} onclick={() => choose(sfx.id, v)}>use this</button></div>
          {/if}
        {/each}
      </div>
    {/if}
  {:else}
    <div class="snd ghost"><div class="sub">{frame ? "no sound on this frame yet" : "no frames yet"}</div></div>
  {/each}

  {#if frame}
    <!-- The add/edit form. -->
    <div class="st-sec nxt">{editing ? `editing · ${editingName || "pick a recipe"}` : "add a sound"}</div>
    <input type="file" accept=".json,.jampatch" class="hidden" aria-label="Import a sound file" bind:this={fileInput} disabled={busy || locked} onchange={(e) => onFileChange(e.currentTarget)} />
    <div class="shelf">in this flipnote · {liveRecipes}/{FLIPNOTE_MAX_PATCHES}</div>
    <div class="rack">
      {#each docTiles as slot (slot.id)}
        {@render tile(waveGlyph(slot.descriptor), rackLabel(instrumentName(slot.descriptor ?? undefined, library)), isDoc(slot.id),
          `${instrumentName(slot.descriptor ?? undefined, library)} · in this flipnote`, () => pickDoc(slot.id), overPatch.has(slot.id), { "data-doc-patch": slot.id })}
      {/each}
      <button type="button" class="rk add" title="Design a new recipe in the soundfont editor" disabled={locked} onclick={() => onnew(descriptor)}>
        <svg viewBox="0 0 26 12" aria-hidden="true"><path d="M13 2v8M9 6h8" /></svg><span class="nm">+ new…</span>
      </button>
      <button type="button" class="rk add" title="Import a .json emoji sound or a .jampatch file" disabled={busy || locked} onclick={() => fileInput?.click()}>
        <svg viewBox="0 0 26 12" aria-hidden="true"><path d="M13 1.5v7" /><path d="M9.5 5.5 13 9l3.5-3.5" /><path d="M3 10.5h20" /></svg><span class="nm">import…</span>
      </button>
    </div>
    {#if imported}
      <div class="shelf">imported</div>
      <div class="rack">
        {@render tile(waveGlyph(imported.patch), imported.name, isLib(imported) && (emoji === imported.emoji), `${imported.name} · imported file`, pickImported, false, { "data-lib-patch": imported.name })}
      </div>
    {/if}
    {#each shelves as group (group.shelf)}
      <div class="shelf">{group.label} · {group.items.length}</div>
      <div class="rack">
        {#each group.items as item (libraryKey(item))}
          {@render tile(waveGlyph(item.patch), rackLabel(item.name), isLib(item), `${item.name} · ${group.label}`, () => pickLib(item), false, { "data-lib-patch": item.name })}
        {/each}
      </div>
    {/each}

    <!-- Pitch: one octave of keys plus octave steps, then semitone steps. -->
    <div class="st-opt pitch">
      <span class="lb">pitch</span>
      <div class="piano" role="group" aria-label="Pitch keys">
        {#each keys.white as key (key.note)}
          <button type="button" class="pk" class:on={note === key.note} title={key.label} aria-label={key.label} aria-pressed={note === key.note}
            disabled={locked || key.note > hi || key.note < lo} onclick={() => setNote(key.note)}></button>
        {/each}
        {#each keys.black as key (key.note)}
          <button type="button" class="pb" class:on={note === key.note} style:left={`${key.left}%`} title={key.label} aria-label={key.label} aria-pressed={note === key.note}
            disabled={locked || key.note > hi || key.note < lo} onclick={() => setNote(key.note)}></button>
        {/each}
      </div>
      <!-- Octave steps clamp at the range ends (G9 is 127), so they stay enabled until there is no room left. -->
      <button type="button" class="st-tile" title="Down an octave" aria-label="Down an octave" disabled={locked || note <= lo} onclick={() => setNote(note - 12)}>−</button>
      <button type="button" class="st-tile" title="Up an octave" aria-label="Up an octave" disabled={locked || note >= hi} onclick={() => setNote(note + 12)}>＋</button>
    </div>
    <div class="st-opt">
      <span class="lb"></span>
      <button type="button" class="st-tile" title="Down a semitone" aria-label="Down a semitone" disabled={locked || note <= lo} onclick={() => setNote(note - 1)}>−</button>
      <span class="mono strong note" aria-live="polite">{noteName(note)}</span>
      <button type="button" class="st-tile" title="Up a semitone" aria-label="Up a semitone" disabled={locked || note >= hi} onclick={() => setNote(note + 1)}>＋</button>
      <span class="micro">midi {note}{#if emoji} · {noteName(emoji.lo)}..{noteName(emoji.hi)}{/if}</span>
    </div>

    <!-- Length: four one-tap holds, the exact field underneath. -->
    <div class="st-opt">
      <span class="lb">length</span>
      {#each LENGTH_PRESETS as preset (preset.ms)}
        <button type="button" class="st-tile len" class:on={duration === preset.ms} title={`${preset.ms / 1000} s`} disabled={locked || !!emoji} onclick={() => setLength(preset.ms)}>{preset.label}</button>
      {/each}
      <span class="mono readout">{duration === "" ? "·" : (duration / 1000).toFixed(3)}</span>
    </div>
    <div class="st-opt">
      <span class="lb"></span>
      <input class="st-input" aria-label="Sound length in seconds" title="Exact length in seconds (0.001 to 600)" type="number" min="0.001" max="600" step="0.001"
        value={lengthText} disabled={locked || !!emoji} oninput={(e) => { lengthText = e.currentTarget.value; }} />
      <span class="micro">s</span>
    </div>

    <div class="acts">
      <button type="button" class="st-btn" title="Play this recipe at the chosen pitch and length" disabled={!canHear} onclick={hear}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 2.5 12.5 8 3.5 13.5z" /></svg>hear it
      </button>
      <button type="button" class="st-btn primary" disabled={!canCommit} onclick={() => void commit()}>{busy ? "adding…" : editing ? "save changes" : `add to frame ${frameNumber}`}</button>
      {#if editing}<button type="button" class="st-btn ghost" onclick={cancel}>cancel</button>{/if}
    </div>
  {/if}

  <!-- Every live marker in the document, 24 to a page. -->
  <div class="st-sec nxt">all sounds <span class="n">· {all.length}{#if all.length > FLIPNOTE_MAX_SFX} of {FLIPNOTE_MAX_SFX}{/if}</span></div>
  {#each allPage.items as sfx (sfx.id)}
    <div class="sfx" class:cur={sfx.frame === frame} class:over={overSfx.has(sfx.id)}>
      <span class="fr">{sfx.frameNumber || "?"}</span>
      <i class="dot" style:background={tint(sfx.source.author)}></i>
      <span class="mono name">{label(sfx)}</span>
      <span class="grow"></span>
      {#if goneFrame.has(sfx.id)}{@render chip("deleted frame")}{/if}
      {#if overSfx.has(sfx.id)}{@render chip("over the cap")}{/if}
      {#if !model.patches[sfx.patch]}{@render chip("missing recipe")}{/if}
      <span class="micro">{who(sfx.source.author)}</span>
      <button type="button" class="x danger" title="Remove this sound" aria-label="Remove this sound" data-remove-sfx={sfx.id} disabled={locked} onclick={() => onremove(sfx.id)}>✕</button>
    </div>
  {:else}
    <div class="snd ghost"><div class="sub">no sounds in this flipnote</div></div>
  {/each}
  {@render pager(allPage.page, allPage.pages, () => { page = allPage.page - 1; }, () => { page = allPage.page + 1; })}

{:else}
  <!-- Evidence mode: what the document retains, for the History tab. -->
  <div class="st-sec">recipes <span class="n">· {liveRecipes} live · {model.sfx.length} markers</span></div>
  {#each recipes.items as slot (slot.id)}
    {@const count = usage.get(slot.id) ?? 0}
    <div class="snd" class:gone={slot.removed} class:over={overPatch.has(slot.id)}>
      <svg class="ic wave" viewBox="0 0 26 12" aria-hidden="true"><path d={waveGlyph(slot.descriptor)} /></svg>
      <div class="col">
        <div class="nm">{slot.removed ? "removed" : instrumentName(slot.descriptor ?? undefined, library)}</div>
        <div class="sub">{slot.removed ? slot.id.slice(0, 8) : count ? `${count} ${count === 1 ? "marker" : "markers"}` : "unused"}</div>
      </div>
      <span class="grow"></span>
      {#if overPatch.has(slot.id)}{@render chip("over the cap")}{/if}
      {#if slot.register.conflicts.length}{@render chip("conflicting versions")}{/if}
      {#if !slot.removed && !count}<button type="button" class="st-link" title="Remove this unused recipe" data-remove-patch={slot.id} disabled={locked} onclick={() => onremovepatch(slot.id)}>remove</button>{/if}
    </div>
  {:else}
    <div class="snd ghost"><div class="sub">no recipes yet</div></div>
  {/each}
  {@render pager(recipes.page, recipes.pages, () => { recipePage = recipes.page - 1; }, () => { recipePage = recipes.page + 1; })}

  <div class="st-sec nxt">markers needing cleanup <span class="n">· {cleanup.length}</span></div>
  {#each cleanupSlice.items as sfx (sfx.id)}
    <div class="sfx" class:over={overSfx.has(sfx.id)}>
      <span class="fr">{frameIndexOf(sfx.frame) || "?"}</span>
      <i class="dot" style:background={tint(sfx.source.author)}></i>
      <span class="mono name">{label(sfx)}</span>
      <span class="grow"></span>
      {#if goneFrame.has(sfx.id)}{@render chip("deleted frame")}{/if}
      {#if goneRecipe.has(sfx.id)}{@render chip("missing recipe")}{/if}
      {#if overSfx.has(sfx.id)}{@render chip("over the cap")}{/if}
      <button type="button" class="x danger" title="Remove this sound" aria-label="Remove this sound" data-remove-sfx={sfx.id} disabled={locked} onclick={() => onremove(sfx.id)}>✕</button>
    </div>
  {:else}
    <div class="snd ghost"><div class="sub">nothing to clean up</div></div>
  {/each}
  {@render pager(cleanupSlice.page, cleanupSlice.pages, () => { cleanupPage = cleanupSlice.page - 1; }, () => { cleanupPage = cleanupSlice.page + 1; })}

  <details class="raw">
    <summary class="micro">raw records · {records.length}</summary>
    <div class="micro wrap">{model.deletedSfx.length} deleted marker ids · {model.deletedExports.length} deleted export ids</div>
    {@render pager(recordSlice.page, recordSlice.pages, () => { evidencePage = recordSlice.page - 1; }, () => { evidencePage = recordSlice.page + 1; })}
    <div class="pager">
      <button type="button" class="st-tile" disabled={altPage === 0} title="Previous alternatives" aria-label="Previous alternatives" onclick={() => altPage--}>‹</button>
      <span class="micro">alternatives {altPage + 1}</span>
      <button type="button" class="st-tile" disabled={!recordSlice.items.some(([, r]) => r.conflicts.length > (altPage + 1) * ALTERNATIVES_PAGE)} title="Next alternatives" aria-label="Next alternatives" onclick={() => altPage++}>›</button>
    </div>
    {#each recordSlice.items as [key, register] (key)}
      <details class="rec">
        <summary class="mono">{key} · {register.conflicts.length} alternatives</summary>
        <pre>{JSON.stringify(register.selected, null, 2)}</pre>
        {#each register.conflicts.slice(altPage * ALTERNATIVES_PAGE, altPage * ALTERNATIVES_PAGE + ALTERNATIVES_PAGE) as value}<pre>{JSON.stringify(value, null, 2)}</pre>{/each}
      </details>
    {/each}
  </details>
{/if}
</section>

<style>
  /* The inspector body is 236px wide (260px minus its padding); every row below is sized to fit
     that without horizontal scroll. The grammar mirrors Studio.svelte's `.st-*` classes: scoped
     styles do not leak between components, so the rules are repeated here on purpose. */
  .sound { display: flex; flex-direction: column; gap: 3px; min-width: 0; font-size: 0.78rem; color: var(--text); }
  .hidden { display: none; }
  .grow { flex: 1; }
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .micro.wrap { white-space: normal; line-height: 1.5; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .st-sec { display: flex; align-items: center; gap: 6px; padding: 6px 0 4px; font-family: var(--mono); font-size: 0.62rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); }
  .st-sec.nxt { border-top: 1px solid var(--border-soft); margin-top: 6px; }
  .st-sec .n { color: var(--faint); font-weight: 400; }
  .st-banner { margin: 0.2rem 0; padding: 6px 10px; border-radius: var(--r); font-size: 0.76rem; display: flex; flex-direction: column; gap: 2px; }
  .st-banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); color: var(--text-2); }
  .st-banner b { color: var(--text); font-weight: 600; }
  .st-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 1px 7px; white-space: nowrap; }
  .st-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .st-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); }
  .st-chip.warn .micro { color: var(--warn); font-size: 0.52rem; }
  .st-chip.warn i { background: var(--warn); }
  .st-chip.tiny { position: absolute; top: -6px; right: -4px; padding: 0 4px; }
  .st-card { display: flex; flex-direction: column; gap: 3px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); padding: 7px 9px; font-size: 0.76rem; color: var(--text); }
  .st-card.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .st-alt { display: flex; align-items: center; gap: 6px; font-size: 0.72rem; color: var(--text-2); min-width: 0; }
  .st-alt > span:not(.grow) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .st-alt i { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .st-opt { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; row-gap: 4px; }
  .st-opt .lb { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); width: 44px; flex: none; }
  .st-opt.pitch { margin-top: 6px; }
  .st-opt .note { min-width: 28px; text-align: center; }
  .st-opt .readout { color: var(--faint); margin-left: auto; }
  /* The global `button` rule paints accent-on-accent, so every button class re-pins its paint. */
  .st-tile { display: inline-flex; align-items: center; justify-content: center; width: 22px; height: 24px; padding: 0; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); font: inherit; font-family: var(--mono); font-size: 0.6rem; font-weight: 400; text-transform: uppercase; }
  .st-tile:hover { background: var(--bg-elev); color: var(--text); border-color: var(--border); }
  .st-tile.len { width: 30px; font-size: 0.56rem; }
  .st-tile.on, .st-tile.on:hover { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tile:disabled { opacity: 0.45; }
  .st-input { flex: 1; min-width: 0; height: 24px; box-sizing: border-box; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.72rem; padding: 2px 6px; }
  .st-input:disabled { opacity: 0.45; }
  .st-btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px; height: 26px; padding: 0 8px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font: inherit; font-family: var(--mono); font-size: 0.66rem; font-weight: 400; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; }
  .st-btn:hover { background: var(--bg-elev); color: var(--text); }
  .st-btn svg { width: 9px; height: 9px; fill: currentColor; }
  .st-btn.primary, .st-btn.primary:hover { background: var(--accent); border-color: var(--accent); color: var(--on-accent); flex: 1; }
  .st-btn.ghost, .st-btn.ghost:hover { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn:disabled { opacity: 0.45; }
  .st-link { background: transparent; border: none; padding: 0; font: inherit; font-family: var(--mono); font-size: 0.6rem; font-weight: 400; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); text-decoration: underline; white-space: nowrap; }
  .st-link:hover { background: transparent; color: var(--text); }
  .st-link:disabled { opacity: 0.45; text-decoration: none; }
  .acts { display: flex; gap: 6px; margin-top: 4px; flex-wrap: wrap; }
  .pager { display: flex; align-items: center; gap: 6px; justify-content: center; padding: 2px 0; }

  /* Marker rows on the selected frame. */
  .snd { display: flex; align-items: center; gap: 7px; padding: 5px 7px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); min-width: 0; }
  .snd.cur { border-color: var(--accent); background: var(--accent-dim); }
  .snd.warn { border-color: var(--warn-brd); background: var(--warn-dim); }
  .snd.gone { opacity: 0.55; }
  .snd.gone .nm { text-decoration: line-through; }
  /* Over-cap: the backend counts the 4096/64 smallest ids; the rest are hatched, not hidden. */
  .snd.over, .sfx.over { opacity: 0.6; background: repeating-linear-gradient(135deg, var(--bg-elev) 0 6px, var(--panel) 6px 12px); }
  .snd.ghost { border-style: dashed; background: transparent; }
  .snd .ic { width: 12px; height: 12px; flex: none; }
  .snd .ic.wave { width: 26px; height: 12px; fill: none; stroke: var(--muted); stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .snd .col { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .snd .nm { font-family: var(--mono); font-size: 0.66rem; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .snd .sub { font-family: var(--mono); font-size: 0.56rem; letter-spacing: 0.06em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .snd .pl { width: 14px; height: 14px; padding: 0; border: none; background: transparent; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; flex: none; }
  .snd .pl:hover { background: transparent; color: var(--text); }
  .snd .pl svg { width: 10px; height: 10px; fill: currentColor; }
  .x { background: transparent; border: none; padding: 0; font: inherit; font-family: var(--mono); font-size: 0.58rem; font-weight: 400; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .x:hover { background: transparent; color: var(--text); }
  .x.danger, .x.danger:hover { color: var(--danger); }
  .x:disabled { opacity: 0.45; }

  /* The recipe rack: shelves of three-column tiles. */
  .shelf { font-family: var(--mono); font-size: 0.56rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); padding: 4px 0 2px; display: flex; align-items: center; gap: 6px; white-space: nowrap; }
  .shelf::after { content: ""; flex: 1; height: 1px; background: var(--border-soft); }
  .rack { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; }
  .rk { display: flex; flex-direction: column; align-items: center; gap: 2px; padding: 5px 4px 4px; border: 1px solid var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); min-width: 0; position: relative; font: inherit; }
  .rk:hover { background: var(--panel); border-color: var(--border); color: var(--text); }
  .rk svg { width: 26px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .rk .nm { font-family: var(--mono); font-size: 0.52rem; letter-spacing: 0.08em; text-transform: uppercase; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rk.on, .rk.on:hover { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .rk.add { border-style: dashed; color: var(--faint); }
  .rk.gone { opacity: 0.5; }
  .rk.gone .nm { text-decoration: line-through; }
  .rk:disabled { opacity: 0.45; }

  /* The one-octave piano. Ivory and ebony are the only hex colours in this component: a key is
     the one surface whose colour must not follow the theme, or a dark theme's white key would
     vanish into the panel and a light theme's black key would read as a white one. */
  .piano { display: flex; position: relative; flex: 1; min-width: 90px; height: 34px; border: 1px solid var(--border); border-radius: 3px; overflow: hidden; background: var(--bg-elev); }
  .pk { flex: 1; min-width: 0; padding: 0; border: none; border-right: 1px solid var(--border-soft); border-radius: 0; background: #e9e4f2; }
  .pk:last-of-type { border-right: none; }
  .pk:hover { background: #d9d2ea; }
  .pb { position: absolute; top: 0; width: 7.2%; height: 58%; padding: 0; background: #2a2735; border: 1px solid var(--bg-0); border-top: none; border-radius: 0 0 2px 2px; }
  .pb:hover { background: #3b3749; }
  .pk.on, .pk.on:hover { background: var(--accent); }
  .pb.on, .pb.on:hover { background: var(--accent-hi); }
  .pk:disabled, .pb:disabled { opacity: 0.35; }

  /* The document-wide list. */
  .sfx { display: flex; align-items: center; gap: 6px; padding: 3px 4px; border-radius: var(--r); min-width: 0; }
  .sfx.cur { background: var(--accent-dim); }
  .sfx .fr { font-family: var(--mono); font-size: 0.6rem; color: var(--faint); width: 18px; text-align: right; flex: none; }
  .sfx .dot { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .sfx .name { overflow: hidden; text-overflow: ellipsis; min-width: 0; }

  /* Raw evidence. */
  .raw { margin-top: 6px; }
  .raw > summary, .rec > summary { cursor: pointer; }
  .rec { margin: 4px 0; }
  .rec summary.mono { overflow: hidden; text-overflow: ellipsis; }
  pre { margin: 2px 0; font-family: var(--mono); font-size: 0.58rem; color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
