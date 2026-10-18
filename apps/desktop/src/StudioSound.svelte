<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import type { JamPatch } from "./jam-contract.ts";
  import type { StudioSoundModel, SoundPatchView, SfxView } from "./studio-sound.ts";
  import { EMOJI_SOUND_MAX_BYTES, parseEmojiSoundJson, type EmojiSound } from "./emoji-sound.ts";
  import { instrumentName, noteName, soundLengthMs } from "./studio-sound-ui.ts";
  let { model, frame, readonlyReason = "", patchLibrary = [], history = false, onhistory, onpatch, onsfx, onremove, onremovepatch, onnotice } = $props<{
    model: StudioSoundModel; frame: string; readonlyReason?: string; patchLibrary?: readonly { name: string; patch: JamPatch }[];
    history?: boolean; onhistory?: () => void;
    onpatch: (patch: JamPatch) => Promise<string>;
    onsfx: (input: { id?: string; patch: string; note: number; durationMs: number }) => void;
    onremove: (id: string) => void; onremovepatch: (patch: string) => void;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
  }>();
  // A visible default is a new marker's authored length. Editing/import never substitutes it.
  let patch = $state(""), note = $state(60), duration = $state<number | "">(250), editing = $state("");
  let busy = $state(false), emoji = $state<EmojiSound | null>(null), page = $state(0), cleanupPage = $state(0), instrumentPage = $state(0), evidencePage = $state(0), altPage = $state(0);
  let disposed = false;
  let formGeneration = 0;
  onDestroy(() => { disposed = true; });
  $effect(() => { void frame; untrack(() => { formGeneration++; editing = ""; emoji = null; duration = 250; page = 0; }); });
  const here = $derived<SfxView[]>(model.sfx.filter((sfx: SfxView) => sfx.frame === frame));
  const shown = $derived(here.slice(page * 8, page * 8 + 8));
  const records = $derived(Object.entries<StudioSoundModel["records"][string]>(model.records).sort(([a], [b]) => a.localeCompare(b)));
  function edit(sfx: (typeof here)[number]) { emoji = null; editing = sfx.id; patch = sfx.patch; note = sfx.note; duration = sfx.durationMs; }
  async function addPatch(descriptor: JamPatch) {
    if (busy || readonlyReason) return;
    const source = model, sourceFrame = frame, generation = formGeneration; busy = true;
    try { const id = await onpatch(descriptor); if (!disposed && source === model && sourceFrame === frame && generation === formGeneration) patch = id; }
    catch (e) { onnotice(String(e), "error"); } finally { busy = false; }
  }
  async function importEmoji(file: File | undefined) {
    if (!file || busy) return;
    if (file.size > EMOJI_SOUND_MAX_BYTES) { onnotice("emoji sound exceeds 4KiB", "error"); return; }
    const source = model, sourceFrame = frame, generation = formGeneration; busy = true;
    try {
      const parsed = parseEmojiSoundJson(await file.text());
      if (disposed || source !== model || sourceFrame !== frame || generation !== formGeneration || readonlyReason) return;
      emoji = parsed; note = parsed.lo; duration = parsed.len;
      const id = await onpatch(parsed.patch); if (!disposed && source === model && sourceFrame === frame && generation === formGeneration) patch = id;
    } catch (e) { onnotice(String(e), "error"); } finally { busy = false; }
  }
  function save() {
    if (!patch || !model.patches[patch] || !frame || readonlyReason || duration === "") return;
    if (!Number.isInteger(note) || note < 0 || note > 127 || !Number.isInteger(duration) || duration < 1 || duration > 600000 ||
      (emoji && (note < emoji.lo || note > emoji.hi || duration !== emoji.len))) { onnotice("Choose a valid pitch and length.", "error"); return; }
    onsfx({ ...(editing ? { id: editing } : {}), patch, note, durationMs: duration }); editing = "";
  }
</script>

<section class="sound" aria-label="Frame sound editor">
  {#if !history}
  <h4>Frame sounds</h4>
  {#if !model.ready}<p class="warn">Some sounds need attention before playback. {#if onhistory}<button onclick={onhistory}>View in History</button>{/if}</p>{/if}
  {#if readonlyReason}<p class="warn">{readonlyReason}</p>{/if}
  {#each shown as sfx (sfx.id)}
    <div class="marker"><span>{instrumentName(model.patches[sfx.patch], patchLibrary)} · {noteName(sfx.note)} · {sfx.durationMs / 1000}s{#if sfx.conflicts.length} · needs review{/if}</span>
      <button disabled={!!readonlyReason} onclick={() => edit(sfx)}>Edit</button><button disabled={!!readonlyReason} onclick={() => onremove(sfx.id)}>Remove</button></div>
  {:else}<p>No sound on this frame.</p>{/each}
  {#if here.length > 8}<div><button disabled={page === 0} onclick={() => page--}>previous</button> {page + 1}/{Math.ceil(here.length / 8)} <button disabled={(page + 1) * 8 >= here.length} onclick={() => page++}>next</button></div>{/if}
  <label>Sound <select aria-label="Sound" value={patch} disabled={busy || !!readonlyReason} onchange={(e) => {
    const value = e.currentTarget.value;
    if (value.startsWith("library:")) { const item = patchLibrary[Number(value.slice(8))]; if (item) { emoji = null; void addPatch(item.patch); } }
    else { patch = value; emoji = null; }
  }}><option value="">Choose a sound…</option>
    {#each Object.keys(model.patches).sort() as id}<option value={id}>{instrumentName(model.patches[id], patchLibrary)}</option>{/each}
    {#if patchLibrary.length}<optgroup label="Sound library">{#each patchLibrary as item, i}<option value={`library:${i}`}>{item.name}</option>{/each}</optgroup>{/if}
  </select></label>
  <div class="sound-fields">
    <label>Pitch <select aria-label="Pitch" bind:value={note} disabled={!!readonlyReason}>
      {#each Array.from({length: 128}, (_, i) => i).filter(i => !emoji || (i >= emoji.lo && i <= emoji.hi)) as n}<option value={n}>{noteName(n)}</option>{/each}
    </select></label>
    <label>Length (seconds) <input aria-label="Sound length in seconds" type="number" min="0.001" max="600" step="0.001" value={duration === "" ? "" : duration / 1000} oninput={(e) => { duration = soundLengthMs(e.currentTarget.valueAsNumber) ?? ""; }} disabled={!!emoji || !!readonlyReason} /></label>
  </div>
  <button class="add-sound" disabled={busy || !!readonlyReason || !frame || !model.patches[patch] || duration === ""} onclick={save}>{editing ? "Save changes" : "Add sound to frame"}</button>
  {#if editing}<button onclick={() => { editing = ""; }}>Cancel edit</button>{/if}
  {#if busy || (patch && !model.patches[patch])}<p>Saving sound…</p>{/if}
  <details><summary>Import a sound</summary>
    <label>Sound file (.json) <input type="file" accept=".json,application/json" disabled={busy || !!readonlyReason} onchange={(e) => void importEmoji(e.currentTarget.files?.[0])} /></label>
    {#if emoji}<p>Imported sound · {emoji.len / 1000}s</p><button onclick={() => { emoji = null; }}>Edit length</button>{/if}
  </details>
  {:else}
  <details><summary>Instruments ({model.patchSlots.filter((slot: SoundPatchView) => !slot.removed).length})</summary>
    <div><button disabled={instrumentPage === 0} onclick={() => instrumentPage--}>Previous instruments</button> {instrumentPage + 1}/{Math.max(1, Math.ceil(model.patchSlots.length / 8))} <button disabled={(instrumentPage + 1) * 8 >= model.patchSlots.length} onclick={() => instrumentPage++}>Next instruments</button></div>
    {#each model.patchSlots.slice(instrumentPage * 8, instrumentPage * 8 + 8) as slot (slot.id)}
      <div class="marker"><span>{instrumentName(slot.descriptor ?? undefined, patchLibrary)}{#if slot.removed} · removed{:else if model.readiness.patchOverflow.includes(slot.id)} · over the instrument limit{/if}{#if slot.register.conflicts.length} · conflicting versions{/if}{#if !slot.removed && !model.sfx.some((sfx: SfxView) => sfx.patch === slot.id)} · unused{/if}</span>
        {#if !slot.removed}<button disabled={!!readonlyReason} data-remove-patch={slot.id} onclick={() => onremovepatch(slot.id)}>Remove instrument</button>{/if}
      </div>
    {/each}
  </details>
  <details><summary>All active sound markers · cleanup ({model.sfx.length})</summary>
    <p>Includes orphaned and overflow markers. Removing a marker never requires its old frame to remain selectable.</p>
    <div><button disabled={cleanupPage === 0} onclick={() => cleanupPage--}>previous markers</button> {cleanupPage + 1}/{Math.max(1, Math.ceil(model.sfx.length / 8))} <button disabled={(cleanupPage + 1) * 8 >= model.sfx.length} onclick={() => cleanupPage++}>next markers</button></div>
    {#each model.sfx.slice(cleanupPage * 8, cleanupPage * 8 + 8) as marker (marker.id)}<div class="marker"><span>{marker.id} · frame {marker.frame}{#if model.readiness.missingFrames.includes(marker.id)} · missing frame{/if}{#if model.readiness.missingPatches.includes(marker.id)} · missing recipe{/if}{#if model.readiness.sfxOverflow.includes(marker.id)} · overflow{/if}</span><button disabled={!!readonlyReason} data-remove-sfx={marker.id} onclick={() => onremove(marker.id)}>remove marker</button></div>{/each}
  </details>
  <details><summary>All retained sound evidence ({records.length})</summary>
    <p>{model.deletedSfx.length} deleted marker IDs · {model.deletedExports.length} deleted export IDs. Readiness: {JSON.stringify(model.readiness)}</p>
    <div><button disabled={evidencePage === 0} onclick={() => evidencePage--}>previous records</button> {evidencePage + 1}/{Math.max(1, Math.ceil(records.length / 8))} <button disabled={(evidencePage + 1) * 8 >= records.length} onclick={() => evidencePage++}>next records</button></div>
    <div><button disabled={altPage === 0} onclick={() => altPage--}>previous alternatives</button> alternatives page {altPage + 1} <button disabled={!records.slice(evidencePage * 8, evidencePage * 8 + 8).some(([, r]) => r.conflicts.length > (altPage + 1) * 3)} onclick={() => altPage++}>next alternatives</button></div>
    {#each records.slice(evidencePage * 8, evidencePage * 8 + 8) as [key, register]}<details><summary>{key} · {register.conflicts.length} alternatives</summary><pre>{JSON.stringify(register.selected, null, 2)}</pre>{#each register.conflicts.slice(altPage * 3, altPage * 3 + 3) as value}<pre>{JSON.stringify(value, null, 2)}</pre>{/each}</details>{/each}
  </details>
  {/if}
</section>
<style>
  .sound { display: flex; flex-direction: column; gap: 8px; font-size: .74rem; min-width: 0; }
  h4,p { margin: 0; } p { color: var(--muted); } label { display: flex; flex-direction: column; gap: 3px; }
  select,input,button { font: inherit; max-width: 100%; } .marker { display: flex; gap: 4px; flex-wrap: wrap; border-bottom: 1px solid var(--border); padding: 5px 0; }
  .warn { color: var(--warn); } pre { font-size: .62rem; white-space: pre-wrap; overflow-wrap: anywhere; }
  .sound-fields { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .add-sound { background: var(--accent); color: var(--on-accent, var(--bg-0)); padding: 8px; }
  select,input { padding: 7px; width: 100%; box-sizing: border-box; }
  .marker span { flex: 1; min-width: 90px; }
  details { margin-top: 10px; } summary { cursor: pointer; color: var(--muted); }
</style>
