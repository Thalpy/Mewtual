<script lang="ts">
  import type { JamPatch } from "./jam-contract.ts";
  import type { StudioSoundModel, SfxView } from "./studio-sound.ts";
  import { EMOJI_SOUND_MAX_BYTES, parseEmojiSoundJson, type EmojiSound } from "./emoji-sound.ts";
  let { model, frame, readonlyReason = "", patchLibrary = [], onpatch, onsfx, onremove, onremovepatch, onnotice } = $props<{
    model: StudioSoundModel; frame: string; readonlyReason?: string; patchLibrary?: readonly { name: string; patch: JamPatch }[];
    onpatch: (patch: JamPatch) => Promise<string>;
    onsfx: (input: { id?: string; patch: string; note: number; durationMs: number }) => void;
    onremove: (id: string) => void; onremovepatch: (patch: string) => void;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
  }>();
  let patch = $state(""), note = $state(60), duration = $state<number | "">(""), editing = $state("");
  let busy = $state(false), emoji = $state<EmojiSound | null>(null), page = $state(0), evidencePage = $state(0), altPage = $state(0);
  const here = $derived<SfxView[]>(model.sfx.filter((sfx: SfxView) => sfx.frame === frame));
  const shown = $derived(here.slice(page * 8, page * 8 + 8));
  const records = $derived(Object.entries<StudioSoundModel["records"][string]>(model.records).sort(([a], [b]) => a.localeCompare(b)));
  function edit(sfx: (typeof here)[number]) { editing = sfx.id; patch = sfx.patch; note = sfx.note; duration = sfx.durationMs; }
  async function addPatch(descriptor: JamPatch) {
    if (busy || readonlyReason) return;
    const source = model; busy = true;
    try { const id = await onpatch(descriptor); if (source === model) patch = id; }
    catch (e) { onnotice(String(e), "error"); } finally { busy = false; }
  }
  async function importEmoji(file: File | undefined) {
    if (!file || busy) return;
    if (file.size > EMOJI_SOUND_MAX_BYTES) { onnotice("emoji sound exceeds 4KiB", "error"); return; }
    const source = model, sourceFrame = frame; busy = true;
    try {
      const parsed = parseEmojiSoundJson(await file.text());
      if (source !== model || sourceFrame !== frame || readonlyReason) return;
      emoji = parsed; note = parsed.lo; duration = parsed.len;
      const id = await onpatch(parsed.patch); if (source === model) patch = id;
    } catch (e) { onnotice(String(e), "error"); } finally { busy = false; }
  }
  function save() {
    if (!patch || !model.patches[patch] || !frame || readonlyReason || duration === "") return;
    if (!Number.isInteger(note) || note < 0 || note > 127 || !Number.isInteger(duration) || duration < 1 || duration > 600000 ||
      (emoji && (note < emoji.lo || note > emoji.hi || duration !== emoji.len))) { onnotice("choose a valid note and the source's authored duration", "error"); return; }
    onsfx({ ...(editing ? { id: editing } : {}), patch, note, durationMs: duration }); editing = "";
  }
</script>

<section class="sound" aria-label="Frame sound editor">
  <h4>Frame sounds</h4>
  <p>Each marker keeps its own note and authored hold, followed by the recipe's release.</p>
  {#if !model.ready}<p class="warn">Playback/export blocked: resolve retained conflicts, missing references or overflow. No sound is silently dropped.</p>{/if}
  {#if readonlyReason}<p class="warn">{readonlyReason}</p>{/if}
  {#each shown as sfx (sfx.id)}
    <div class="marker"><span>{sfx.id.slice(0, 8)} · note {sfx.note} · {sfx.durationMs}ms{#if sfx.conflicts.length} · {sfx.conflicts.length} conflict(s){/if}</span>
      <button disabled={!!readonlyReason} onclick={() => edit(sfx)}>edit</button><button disabled={!!readonlyReason} onclick={() => onremove(sfx.id)}>remove</button></div>
  {:else}<p>No sound on this frame.</p>{/each}
  {#if here.length > 8}<div><button disabled={page === 0} onclick={() => page--}>previous</button> {page + 1}/{Math.ceil(here.length / 8)} <button disabled={(page + 1) * 8 >= here.length} onclick={() => page++}>next</button></div>{/if}
  <label>Active recipe <select bind:value={patch} disabled={!!readonlyReason}><option value="">choose a held recipe</option>{#each Object.keys(model.patches).sort() as id}<option value={id}>{id.slice(0, 12)}</option>{/each}</select></label>
  <label>Jam library <select disabled={busy || !!readonlyReason} value="" onchange={(e) => { const item = patchLibrary[Number(e.currentTarget.value)]; if (item) void addPatch(item.patch); }}><option value="">add a creator recipe…</option>{#each patchLibrary as item, i}<option value={i}>{item.name}</option>{/each}</select></label>
  <label>Local emoji-sound file <input type="file" accept=".json,application/json" disabled={busy || !!readonlyReason} onchange={(e) => void importEmoji(e.currentTarget.files?.[0])} /></label>
  <p>Local import only: no shared emoji catalog or verified image-author binding is claimed.</p>
  {#if emoji}<p>Imported source: notes {emoji.lo}–{emoji.hi}; authored hold {emoji.len}ms. Image {emoji.img.slice(0, 12)}.</p><button onclick={() => { emoji = null; }}>use manual creator duration</button>{/if}
  <label>Note <input type="number" min={emoji?.lo ?? 0} max={emoji?.hi ?? 127} bind:value={note} disabled={!!readonlyReason} /></label>
  <label>Authored hold (ms) <input type="number" min="1" max="600000" placeholder="from the sound creator" bind:value={duration} disabled={!!emoji || !!readonlyReason} /></label>
  <button disabled={busy || !!readonlyReason || !frame || !model.patches[patch] || duration === ""} onclick={save}>{editing ? "save marker edit" : "add sound marker"}</button>
  {#if editing}<button onclick={() => { editing = ""; }}>cancel marker edit</button>{/if}
  {#if patch && !model.patches[patch]}<p class="warn">Recipe Save must land before a marker can use it. Check the pending Save card.</p>{/if}
  {#if patch && model.patches[patch]}<button disabled={!!readonlyReason} onclick={() => onremovepatch(patch)}>remove active recipe</button>{/if}
  <details><summary>All retained sound evidence ({records.length})</summary>
    <p>{model.deletedSfx.length} deleted marker IDs · {model.deletedExports.length} deleted export IDs. Readiness: {JSON.stringify(model.readiness)}</p>
    <div><button disabled={evidencePage === 0} onclick={() => evidencePage--}>previous records</button> {evidencePage + 1}/{Math.max(1, Math.ceil(records.length / 8))} <button disabled={(evidencePage + 1) * 8 >= records.length} onclick={() => evidencePage++}>next records</button></div>
    <div><button disabled={altPage === 0} onclick={() => altPage--}>previous alternatives</button> alternatives page {altPage + 1} <button disabled={!records.slice(evidencePage * 8, evidencePage * 8 + 8).some(([, r]) => r.conflicts.length > (altPage + 1) * 3)} onclick={() => altPage++}>next alternatives</button></div>
    {#each records.slice(evidencePage * 8, evidencePage * 8 + 8) as [key, register]}<details><summary>{key} · {register.conflicts.length} alternatives</summary><pre>{JSON.stringify(register.selected, null, 2)}</pre>{#each register.conflicts.slice(altPage * 3, altPage * 3 + 3) as value}<pre>{JSON.stringify(value, null, 2)}</pre>{/each}</details>{/each}
  </details>
</section>
<style>
  .sound { display: flex; flex-direction: column; gap: 8px; font-size: .74rem; min-width: 0; }
  h4,p { margin: 0; } p { color: var(--muted); } label { display: flex; flex-direction: column; gap: 3px; }
  select,input,button { font: inherit; max-width: 100%; } .marker { display: flex; gap: 4px; flex-wrap: wrap; border-bottom: 1px solid var(--border); padding: 5px 0; }
  .warn { color: var(--warn); } pre { font-size: .62rem; white-space: pre-wrap; overflow-wrap: anywhere; }
</style>
