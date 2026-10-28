<!--
  StudioSoundfont: the Flipnote Studio's soundfont editor, as a modal dialog.

  A "soundfont" is a `jam-patch:v1` recipe (`JamPatch`). This is the call drawer's inline patch
  editor lifted into a popup so a Studio user can start from a preset, a library entry, a blank
  sheet or a file, shape it by ear, and then USE it in the flipnote, SAVE it to this device, or
  SHARE it into the server's encrypted share, without ever seeing a JSON file.

  Ownership: this component owns nothing durable. It holds a working copy of a recipe and hands
  validated clones to the host through callbacks; the host owns the library, the share, the
  flipnote and the synth. The host is also what decides whether a note may sound at all (Deafen,
  a stopped playback, and so on): `onaudition` is a request, not a guarantee.
-->
<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import {
    JAM_PATCH_NAME_MAX_CHARS,
    JAM_PATCH_VERSION,
    JAM_SAVED_PATCHES_MAX,
    PATCH_OSC_MAX,
    PATCH_PARAM,
    type JamPatch,
  } from "./jam-contract.ts";
  import {
    EMPTY_STAGE_STASH,
    envOff,
    filterOff,
    sendsOff,
    setFilterMode,
    toggleStage,
    type JamEditorStep,
    type JamStage,
    type JamStageStash,
  } from "./jam-editor.ts";
  import { validateJamPatch } from "./jam-patch.ts";
  import { dismissOnBackdrop } from "./overlay-dismiss.ts";
  import { noteName } from "./studio-sound-ui.ts";
  import {
    blankSoundfontPatch,
    clonePatch,
    cutoffLabel,
    HOME_ROW_SEMITONES,
    importName,
    importSoundfontText,
    knobArc,
    knobDragValue,
    knobStep,
    lfoRateLabel,
    oscSummary,
    PIANO_BLACK_KEYS,
    PIANO_BLACK_WIDTH_PCT,
    PIANO_WHITE_KEYS,
    recipeKey,
    registerBase,
    registerLabel,
    secondsLabel,
    SOUNDFONT_IMPORT_MAX_BYTES,
    SOUNDFONT_REGISTER_DEFAULT,
    SOUNDFONT_REGISTER_MAX,
    SOUNDFONT_REGISTER_MIN,
    WAVE_TILES,
    waveTile,
  } from "./studio-soundfont.ts";

  /** One "start from" tile: a name and an already-validated recipe. */
  type StartTile = { name: string; patch: JamPatch };

  // The host contract. `name` and `initial` are read ONCE, at mount: the dialog is one editing
  // session, so the host mounts a fresh instance each time it opens and never needs to steer a
  // live one. The callbacks reject with an Error whose message is shown through `onnotice`.
  let {
    name = "",
    initial,
    startFrom = [],
    libraryCount = 0,
    libraryMax = JAM_SAVED_PATCHES_MAX,
    targetName = "",
    recipeCount = 0,
    recipeMax = 64,
    shareName = "",
    bpm = 120,
    onaudition,
    onuse,
    onsave,
    onshare,
    onnotice,
    onclose,
  } = $props<{
    /** Initial name, at most JAM_PATCH_NAME_MAX_CHARS (longer is cut, not refused). */
    name?: string;
    /** An already-validated recipe to start from. Cloned on mount; the prop is never mutated. */
    initial: JamPatch;
    /** The "start from" shelf: presets, the library, the current recipe. Each click clones. */
    startFrom?: readonly StartTile[];
    libraryCount?: number;
    libraryMax?: number;
    /** Where USE puts the recipe, e.g. the flipnote's title. */
    targetName?: string;
    recipeCount?: number;
    recipeMax?: number;
    /** The server whose share SHARE seals into. Empty hides the button. */
    shareName?: string;
    /** Play one note of this recipe, held for `durationMs` (the chosen preview length). The host
     *  gates it (Deafen etc.) and owns the synth. */
    onaudition: (patch: JamPatch, note: number, durationMs: number) => void;
    /** The tempo the "beat" and "bar" preview lengths follow; the open score's, else 120. */
    bpm?: number;
    /** Add to the flipnote. Success closes the dialog. */
    onuse: (name: string, patch: JamPatch) => Promise<void>;
    /** Keep on this device. Success keeps the dialog open. */
    onsave: (name: string, patch: JamPatch) => Promise<void>;
    /** Seal into the server share. Success keeps the dialog open. */
    onshare: (name: string, patch: JamPatch) => Promise<void>;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
    onclose: () => void;
  }>();

  // --- Working state ----------------------------------------------------------------------------
  // `untrack` makes the once-only read explicit: these are the session's starting values, not
  // mirrors of the props.
  let draftName = $state(untrack(() => name.slice(0, JAM_PATCH_NAME_MAX_CHARS)));
  // The working recipe is `$state.raw`, and that is an invariant rather than a tuning choice:
  // every edit REPLACES the whole descriptor with a fresh clone (as the call drawer does, where a
  // new object is what mints a new id). Mutating a field in place would change nothing on screen,
  // which is exactly the loud failure wanted if a future edit path forgets the rule. It also means
  // no proxy ever reaches the host: what `onuse` receives is the validator's own fresh object.
  let patch = $state.raw<JamPatch>(untrack(() => clonePatch(initial)));
  // What each bypassed stage held, so OFF then OFF again returns the sound rather than a default.
  // Editor state only: never part of the recipe, and reset whenever a whole recipe is swapped in,
  // because restoring one recipe's filter into another is the defect jam-editor.ts was written for.
  let stash = $state.raw<JamStageStash>(EMPTY_STAGE_STASH);
  // Exactly one layer unfolds at a time; the first is open on arrival. A collapsed layer is a
  // one-line summary with its level bar, so the stack reads like a small mixer, and the controls
  // only ever occupy one card's worth of height however many layers there are.
  let openLayer = $state<number | null>(0);
  let register = $state(SOUNDFONT_REGISTER_DEFAULT);
  // Which footer action is in flight. The three actions are mutually exclusive; cancel never is.
  let busy = $state<"" | "use" | "save" | "share">("");
  let nameInput: HTMLInputElement | undefined;
  let fileInput: HTMLInputElement | undefined;

  // The canonical bytes the session started from, and the working copy's current validation.
  // "Edited" is a comparison of canonical forms, the same bytes an id is hashed over, so the chip
  // lights exactly when a save would mint a new id and never on a no-op such as a knob returned to
  // where it was. A host that hands over an invalid `initial` (a contract breach) gets a dialog
  // whose footer stays disabled rather than one that quietly plays something else.
  const baseline = untrack(() => recipeKey(initial));
  const checked = $derived(validateJamPatch(clonePatch(patch)));
  const valid = $derived(checked.ok);
  const currentKey = $derived(checked.ok ? checked.canonical : null);
  const edited = $derived(currentKey === null || currentKey !== baseline);
  const isEnvOff = $derived(envOff(patch));
  const isFilterOff = $derived(filterOff(patch));
  const isSendsOff = $derived(sendsOff(patch));
  const base = $derived(registerBase(register));
  // The shelf, with each tile's canonical key so the one matching the working copy can light.
  // The parameter is annotated because a destructuring default of `[]` widens `startFrom` to a
  // union of array types, over which `.map` cannot infer its callback's parameter.
  const shelf = $derived(startFrom.map((tile: StartTile) => ({ name: tile.name, patch: tile.patch, key: recipeKey(tile.patch), glyph: waveTile(tile.patch.o[0]?.w ?? 0) })));
  const blankKey = recipeKey(blankSoundfontPatch());

  // --- Lifecycle and the generation fence -------------------------------------------------------
  // Every awaited callback captures the generation it started under and ignores its own result
  // if the number has moved on. The number moves when the dialog closes (cancel, Escape, the
  // scrim, a successful USE) and when a new action starts, so a slow `onuse` that resolves after
  // the user gave up cannot close a dialog that is already closed, or the one that replaced it,
  // and a rejection from an abandoned attempt cannot surface as an error on a later one.
  let generation = 0;
  let disposed = false;
  onDestroy(() => {
    disposed = true;
    generation += 1;
  });
  onMount(() => {
    nameInput?.focus();
  });
  function close() {
    generation += 1;
    onclose();
  }

  // --- Edits: every one a whole-descriptor replacement --------------------------------------------
  function editNum(path: "e" | "f" | "l" | "x", key: string, raw: number) {
    const value = Math.round(raw);
    if (!Number.isFinite(value)) return;
    const next = clonePatch(patch);
    (next[path] as unknown as Record<string, number>)[key] = value;
    patch = next;
  }
  function editOsc(index: number, key: "w" | "t" | "c" | "l", raw: number) {
    if (!patch.o[index]) return;
    const value = Math.round(raw);
    if (!Number.isFinite(value)) return;
    const next = clonePatch(patch);
    next.o[index][key] = value;
    patch = next;
  }
  function addLayer() {
    if (patch.o.length >= PATCH_OSC_MAX) return;
    const next = clonePatch(patch);
    next.o.push({ w: 3, t: 0, c: 0, l: 60 }); // the call drawer's new-layer default: a quiet saw
    patch = next;
    openLayer = next.o.length - 1; // the new layer opens, ready to shape
  }
  function removeLayer(index: number) {
    if (patch.o.length <= 1) return; // a recipe is one to three layers; the last cannot go
    const next = clonePatch(patch);
    next.o.splice(index, 1);
    patch = next;
    if (openLayer !== null && openLayer >= next.o.length) openLayer = next.o.length - 1;
  }
  // Stage bypass: the transitions live in jam-editor.ts as pure functions; this only binds them.
  function applyStep(step: JamEditorStep) {
    stash = step.stash;
    patch = step.patch;
  }
  function onToggleStage(stage: JamStage) {
    applyStep(toggleStage(patch, stash, stage));
  }
  function onFilterMode(mode: number) {
    applyStep(setFilterMode(patch, stash, mode));
  }

  /**
   * Swap in a whole recipe (a shelf tile, the blank sheet, an import). The stash is dropped with
   * the recipe it belonged to, and an empty name adopts the tile's, since a user who has not named
   * anything yet most likely wants the name of what they just picked.
   */
  function adopt(tileName: string, source: JamPatch) {
    const result = validateJamPatch(clonePatch(source));
    if (!result.ok) {
      onnotice(`That soundfont cannot be loaded: ${result.error}`, "error");
      return;
    }
    patch = result.patch;
    stash = EMPTY_STAGE_STASH;
    openLayer = 0;
    if (!draftName.trim()) draftName = tileName.slice(0, JAM_PATCH_NAME_MAX_CHARS);
  }

  // --- Import -----------------------------------------------------------------------------------
  async function importFile(file: File | undefined) {
    if (!file) return;
    // Refuse by the declared size before reading: the decoder re-measures the bytes it gets.
    if (file.size > SOUNDFONT_IMPORT_MAX_BYTES) {
      onnotice(`${file.name} is larger than a soundfont can be (${SOUNDFONT_IMPORT_MAX_BYTES} bytes)`, "error");
      return;
    }
    const started = generation;
    let text: string;
    try {
      text = await file.text();
    } catch {
      onnotice(`${file.name} could not be read`, "error");
      return;
    }
    // A read that lands after the dialog closed must not resurrect its state.
    if (disposed || started !== generation) return;
    const result = importSoundfontText(text);
    if (!result.ok) {
      onnotice(`${file.name}: ${result.error}`, "error");
      return;
    }
    adopt(importName(file.name), result.patch);
    if (result.notice) onnotice(result.notice, "warn");
  }
  function onFilePicked(e: Event & { currentTarget: HTMLInputElement }) {
    const file = e.currentTarget.files?.[0];
    // Clear the picker so choosing the same file again still fires `change`.
    e.currentTarget.value = "";
    void importFile(file);
  }

  // --- Audition ---------------------------------------------------------------------------------
  // Only ever called from an explicit gesture: a key press, a piano key, never a knob change or
  // mount. The host plays the note; the dialog does not own an AudioContext.
  // How long a played key is held. "beat" and "bar" follow the host's tempo so a preview is the
  // length a note of that value will actually have in the score; the envelope graph is drawn on
  // this time scale, with the key-up marked, so a slow attack cut short is visible before it is
  // heard.
  const PREVIEWS = ["short", "beat", "bar", "long"] as const;
  type Preview = (typeof PREVIEWS)[number];
  let preview = $state<Preview>("short");
  const previewMs = $derived(previewLength(preview, bpm));
  function previewLength(kind: Preview, tempo: number): number {
    const beat = 60_000 / (Number.isFinite(tempo) && tempo > 0 ? tempo : 120);
    return kind === "short" ? 400 : kind === "beat" ? Math.round(beat) : kind === "bar" ? Math.round(beat * 4) : 2000;
  }
  function audition(semitone: number) {
    const result = validateJamPatch(clonePatch(patch));
    if (!result.ok) {
      onnotice(`This soundfont cannot be played: ${result.error}`, "error");
      return;
    }
    onaudition(result.patch, base + semitone, previewMs);
  }
  /**
   * The loudness curve on a real time axis: attack, decay, the shelf until the key-up at
   * `holdMs`, then the release. A key let go during the attack or decay releases from the level
   * it had reached, which is what the engine does and what a short preview of a slow sound hides.
   */
  function envelopePath(e: JamPatch["e"], holdMs: number): { points: [number, number][]; keyUp: number } {
    const level = (t: number) => t < e.a ? (e.a > 0 ? t / e.a : 1) : t < e.a + e.d ? 1 - (1 - e.s / 100) * ((t - e.a) / Math.max(1, e.d)) : e.s / 100;
    const total = Math.max(1, holdMs + e.r);
    const x = (t: number) => (t / total) * 100, y = (l: number) => 26 - l * 20;
    const points: [number, number][] = [[0, 26]];
    if (holdMs > e.a) points.push([x(e.a), y(1)]);
    if (holdMs > e.a + e.d) points.push([x(e.a + e.d), y(e.s / 100)]);
    points.push([x(holdMs), y(level(holdMs))], [100, 26]);
    return { points, keyUp: x(holdMs) };
  }
  function shiftRegister(direction: number) {
    register = Math.min(SOUNDFONT_REGISTER_MAX, Math.max(SOUNDFONT_REGISTER_MIN, register + direction));
  }

  /**
   * Keys typed anywhere in the dialog.
   *
   * The handler sits on the dialog's own root, not on the window, and stops propagation first:
   * the Studio underneath listens on the window for its hotkeys whenever focus is inside its root,
   * which this dialog is. Without the stop, "a" would play a note here AND fire whatever the Studio
   * bound to "a" beneath the scrim. Escape closes. The home row plays only on a fresh press (auto
   * repeat would retrigger a held note dozens of times a second) with no modifier held, and never
   * while a text field has focus, where the letters are the name being typed.
   */
  function onKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
    const tag = (e.target as Element | null)?.tagName ?? "";
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    const semitone = HOME_ROW_SEMITONES[e.key.toLowerCase()];
    if (semitone === undefined) return;
    e.preventDefault();
    audition(semitone);
  }

  // --- Knobs ------------------------------------------------------------------------------------
  // Grab and drag up/right to raise (Shift = fine), wheel steps, arrows step when focused. One
  // drag at a time. `hint` is what the knob does to the SOUND, shown on hover, because the
  // three-letter label under a small dial can only ever be an abbreviation. `bipolar` says the
  // rest position is the centre, so the lit arc grows from twelve o'clock. Optional PROPERTIES
  // are fine; only optional PARAMETERS break this app at load, so none appear in this script.
  type KnobBinding = { label: string; value: number; min: number; max: number; disp: string; hint: string; bipolar?: boolean; set: (v: number) => void };
  let drag: { bind: KnobBinding; startX: number; startY: number; startVal: number } | null = null;
  function knobDown(e: PointerEvent, bind: KnobBinding) {
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      // A DOM without pointer capture (the test runner's): the drag still follows while the
      // pointer stays over the knob, and releases wherever it ends.
    }
    drag = { bind, startX: e.clientX, startY: e.clientY, startVal: bind.value };
  }
  function knobMove(e: PointerEvent) {
    const d = drag;
    if (!d) return;
    e.preventDefault();
    const px = d.startY - e.clientY + (e.clientX - d.startX);
    d.bind.set(knobDragValue(d.startVal, d.bind.min, d.bind.max, px, e.shiftKey));
  }
  function knobUp() {
    drag = null;
  }
  function knobKey(e: KeyboardEvent, bind: KnobBinding) {
    const direction = e.key === "ArrowUp" || e.key === "ArrowRight" ? 1 : e.key === "ArrowDown" || e.key === "ArrowLeft" ? -1 : 0;
    if (!direction) return;
    e.preventDefault();
    bind.set(knobStep(bind.value, bind.min, bind.max, direction, e.shiftKey));
  }

  // --- Footer actions ---------------------------------------------------------------------------
  /**
   * Validate a clone, hand it to the host, and apply the outcome only if this attempt is still the
   * current one. The clone is the validator's own reconstruction, so edits made in the dialog while
   * the host is still working (or after it finished) cannot reach what the host received.
   */
  async function run(kind: "use" | "save" | "share") {
    if (busy) return;
    const verb = kind === "use" ? "used" : kind === "save" ? "saved" : "shared";
    const result = validateJamPatch(clonePatch(patch));
    if (!result.ok) {
      onnotice(`This soundfont cannot be ${verb}: ${result.error}`, "error");
      return;
    }
    const trimmed = draftName.trim().slice(0, JAM_PATCH_NAME_MAX_CHARS);
    const started = ++generation;
    busy = kind;
    try {
      await (kind === "use" ? onuse : kind === "save" ? onsave : onshare)(trimmed, result.patch);
      if (disposed || started !== generation) return;
      if (kind === "use") {
        close();
        return;
      }
      const label = trimmed || "soundfont";
      onnotice(kind === "save" ? `${label} is in your library` : `${label} is in the ${shareName} share as a .jampatch`, "info");
    } catch (e) {
      if (disposed || started !== generation) return;
      onnotice(e instanceof Error ? e.message : String(e), "error");
    } finally {
      // Re-arm even when the attempt was superseded: the dialog is either gone or is a live one
      // whose footer must not stay disabled on account of an attempt it has already forgotten.
      if (!disposed) busy = "";
    }
  }
</script>

<!--
  One patch knob.

  The lit arc runs from the knob's rest position to the tick. The tooltip leads with what the knob
  does to the sound; the hint's opening term is also the accessible name, so a screen reader says
  "attack" rather than "atk". `aria-description` would be the right home for the rest of the
  sentence, but the slider role does not support it and the DOM typings do not know it, so it
  stays in `title`.
-->
{#snippet knob(bind: KnobBinding)}
  {@const arc = knobArc(bind.value, bind.min, bind.max, !!bind.bipolar)}
  <div
    class="sf-knob"
    role="slider"
    tabindex="0"
    aria-label={bind.hint.split(":")[0]}
    aria-valuemin={bind.min}
    aria-valuemax={bind.max}
    aria-valuenow={bind.value}
    aria-valuetext={bind.disp}
    title={`${bind.hint}\nDrag up or right (Shift for fine), scroll, or use the arrow keys.`}
    onpointerdown={(e) => knobDown(e, bind)}
    onpointermove={knobMove}
    onpointerup={knobUp}
    onpointercancel={knobUp}
    onwheel={(e) => { e.preventDefault(); bind.set(knobStep(bind.value, bind.min, bind.max, e.deltaY < 0 ? 1 : -1, e.shiftKey)); }}
    onkeydown={(e) => knobKey(e, bind)}
  >
    <svg class="sf-knob-face" viewBox="0 0 32 32" aria-hidden="true">
      <circle cx="16" cy="16" r="13" class="sf-knob-ring" />
      <path d={arc.d} class="sf-knob-arc" />
      <g style={`transform: rotate(${arc.deg}deg); transform-origin: 16px 16px`}>
        <line x1="16" y1="16" x2="16" y2="5.5" class="sf-knob-tick" />
      </g>
    </svg>
    <span class="sf-knob-lbl">{bind.label}</span>
    <span class="sf-knob-val">{bind.disp}</span>
  </div>
{/snippet}

<!--
  A scope per stage: not a meter, a sketch of the setting the knobs beside it describe, redrawn
  from the working recipe on every edit. All four draw into the same `0 0 100 28` box with
  `preserveAspectRatio="none"`, so one trace fills a tile beside the knobs or a strip above them;
  the CSS pins stroke widths to screen pixels so the stretch never fattens a line. They are
  proportional to the patch bounds, not measurements of a running graph: the shape is honest, the
  axes are not calibrated. Ported from the call drawer unchanged.
-->

<!-- Loudness over time, on the preview's own time axis: attack up, decay down to the sustain
     shelf, the shelf until the key is let go (the dashed rule, at the chosen preview length),
     then the release. Unlike the other scopes this one is calibrated, so a slow attack that a
     short preview cuts off is visible as a peak that never arrives. -->
{#snippet scopeEnv(e: JamPatch["e"])}
  {@const shape = envelopePath(e, previewMs)}
  {@const line = shape.points.map(([x, y]) => `${x.toFixed(2)} ${y.toFixed(2)}`).join(" L")}
  <svg class="sf-scope" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true" data-sf-keyup={shape.keyUp.toFixed(1)}>
    <path class="grid" d="M0 26H100" />
    <path class="fill" d={`M${line} Z`} />
    <path class="trace" d={`M${line}`} />
    <path class="mark" d={`M${shape.keyUp.toFixed(2)} 4 V26`} />
  </svg>
{/snippet}

<!-- The filter's response over a log frequency axis: a shoulder or a hump at the cutoff, taller
     and narrower with resonance. The dashed run along the floor is how far each note sweeps the
     cutoff, so a negative envelope amount visibly reaches the other way. -->
{#snippet scopeFilter(f: JamPatch["f"])}
  {@const decades = Math.log2(PATCH_PARAM.f.c.max / PATCH_PARAM.f.c.min)}
  {@const xc = (Math.log2(Math.max(PATCH_PARAM.f.c.min, f.c) / PATCH_PARAM.f.c.min) / decades) * 100}
  {@const peak = 9 - (f.q / PATCH_PARAM.f.q.max) * 18}
  {@const hump = 12 - (f.q / PATCH_PARAM.f.q.max) * 10}
  {@const w = 24 - (f.q / PATCH_PARAM.f.q.max) * 14}
  {@const xe = Math.min(100, Math.max(0, xc + (f.e / PATCH_PARAM.f.e.max) * 61))}
  {@const d = f.m === 1 ? `M100 9 L${xc + 6} 9 Q${xc} ${peak} ${xc - 3} 15 L${xc - 15} 26` : f.m === 2 ? `M${xc - w} 26 Q${xc} ${2 * hump - 26} ${xc + w} 26` : `M0 9 L${xc - 6} 9 Q${xc} ${peak} ${xc + 3} 15 L${xc + 15} 26`}
  {@const closing = f.m === 1 ? "L100 26" : f.m === 2 ? "" : "L0 26"}
  <svg class="sf-scope" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
    <path class="grid" d="M0 26H100" />
    <path class="fill" d={`${d} ${closing} Z`} />
    <path class="trace" d={d} />
    {#if f.e !== 0}
      <path class="sweep" d={`M${xc} 27 H${xe}`} />
    {/if}
  </svg>
{/snippet}

<!-- The wobble itself: more cycles across the box as the rate rises, taller as the depth rises.
     Off, or a depth of zero, draws flat, because that is exactly what the engine does with it. -->
{#snippet scopeLfo(l: JamPatch["l"])}
  {@const on = l.t !== 0 && l.d > 0}
  {@const cycles = 1 + (l.r / PATCH_PARAM.l.r.max) * 6}
  {@const amp = on ? 2 + (l.d / PATCH_PARAM.l.d.max) * 10 : 0}
  {@const d = Array.from({ length: 41 }, (_u, i) => `${i ? "L" : "M"}${(i * 2.5).toFixed(1)} ${(14 - amp * Math.sin((i / 40) * cycles * 2 * Math.PI)).toFixed(2)}`).join(" ")}
  <svg class="sf-scope" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
    <path class="grid" d="M0 14H100" />
    <path class={on ? "trace" : "mark"} d={d} />
  </svg>
{/snippet}

<!-- What the room's effects do to one note after it is struck: the dry hit, a chorus twin beside
     it, the reverb's short tail behind it, and the echo's repeats spaced down the box. -->
{#snippet scopeSends(x: JamPatch["x"])}
  {@const c = x.c / PATCH_PARAM.x.c.max}
  {@const dl = x.d / PATCH_PARAM.x.d.max}
  {@const rv = x.r / PATCH_PARAM.x.r.max}
  <svg class="sf-scope" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
    <path class="grid" d="M0 26H100" />
    {#if rv > 0}
      <path class="tail" d={`M7 26 L7 ${26 - 14 * rv} L${7 + 22 * (0.4 + 0.6 * rv)} 26 Z`} />
    {/if}
    <path class="bar" d="M6 26 V6" />
    {#if c > 0}
      <path class="bar soft" d={`M9.5 26 V${26 - 20 * c}`} />
    {/if}
    {#each [0.7, 0.35, 0.18] as k, n (n)}
      {#if dl * k * 20 >= 0.5}
        <path class="bar soft" d={`M${6 + 28 * (n + 1)} 26 V${26 - 20 * dl * k}`} />
      {/if}
    {/each}
  </svg>
{/snippet}

<!-- The scrim is dismissed only by a press that both starts and ends on it (see overlay-dismiss),
     so a knob drag or a text selection that wanders off the card never throws the work away. -->
<div class="sf-overlay" role="presentation" use:dismissOnBackdrop={close} onkeydown={onKey}>
  <div class="sf-card" role="dialog" aria-modal="true" aria-label="Soundfont editor">
    <header class="sf-head">
      <svg class="sf-note" viewBox="0 0 16 16" aria-hidden="true"><path d="M6 12.5V4l6-1.5V11" /><circle cx="4.5" cy="12.5" r="1.8" /><circle cx="10.5" cy="11" r="1.8" /></svg>
      <span class="sf-hd">soundfont</span>
      <input
        class="sf-name"
        type="text"
        maxlength={JAM_PATCH_NAME_MAX_CHARS}
        placeholder="name"
        aria-label="Soundfont name"
        title={`Up to ${JAM_PATCH_NAME_MAX_CHARS} characters. A label for the tile, not an identity: the recipe's id comes from its contents.`}
        bind:value={draftName}
        bind:this={nameInput}
      />
      <span class="sf-grow"></span>
      <span class="sf-chip" title="The recipe format this editor writes"><span class="micro">{JAM_PATCH_VERSION}</span></span>
      {#if edited}
        <span class="sf-chip warn" title="This recipe no longer matches what the editor opened with. Saving or using it mints a new id; the original stays as it was."><i></i><span class="micro">edited · new id on save</span></span>
      {/if}
      <button type="button" class="sf-btn ghost icon" aria-label="Close" title="Close without using this soundfont" onclick={close}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
      </button>
    </header>

    <!-- Start from: a shelf of whole recipes. Picking one replaces the working copy outright. -->
    <div class="sf-row shelf">
      <span class="micro sf-lbl">start from</span>
      {#each shelf as tile, i (i)}
        <button type="button" class="sf-rk" class:on={tile.key !== null && tile.key === currentKey} title={`Start from ${tile.name}`} onclick={() => adopt(tile.name, tile.patch)}>
          <svg viewBox="0 0 26 12" aria-hidden="true"><path d={tile.glyph.d} /></svg>
          <span class="nm">{tile.name}</span>
        </button>
      {/each}
      <button type="button" class="sf-rk" class:on={blankKey !== null && blankKey === currentKey} title="Start from a plain sine with nothing else switched on" onclick={() => adopt("", blankSoundfontPatch())}>
        <svg viewBox="0 0 26 12" aria-hidden="true"><path d={waveTile(0).d} /></svg>
        <span class="nm">blank</span>
      </button>
      <button type="button" class="sf-rk add" title="Load a .jampatch file, or an emoji sound's .json (its recipe only)" onclick={() => fileInput?.click()}>
        <svg viewBox="0 0 26 12" aria-hidden="true"><path d="M13 1.5v7" /><path d="M9.5 5.5 13 9l3.5-3.5" /><path d="M3 10.5h20" /></svg>
        <span class="nm">import…</span>
      </button>
      <input class="sf-file" type="file" accept=".jampatch,.json" aria-label="Import a soundfont file" tabindex="-1" bind:this={fileInput} onchange={onFilePicked} />
    </div>

    <!-- Hear it: two octaves from C{register}. Keys are reachable by pointer; the home row is the
         keyboard path, so the keys stay out of the tab order rather than adding 24 stops. -->
    <div class="sf-row hear">
      <span class="micro sf-lbl">hear it</span>
      <div class="sf-piano" role="group" aria-label="Audition keyboard">
        <div class="whites">
          {#each PIANO_WHITE_KEYS as semitone (semitone)}
            <button type="button" class="pk" tabindex="-1" aria-label={`Play ${noteName(base + semitone)}`} title={noteName(base + semitone)} onpointerdown={() => audition(semitone)}></button>
          {/each}
        </div>
        {#each PIANO_BLACK_KEYS as key (key.semitone)}
          <button type="button" class="pb" style={`left:${key.left}%;width:${PIANO_BLACK_WIDTH_PCT}%`} tabindex="-1" aria-label={`Play ${noteName(base + key.semitone)}`} title={noteName(base + key.semitone)} onpointerdown={() => audition(key.semitone)}></button>
        {/each}
      </div>
      <button type="button" class="sf-mb" aria-label="Register down" title="One octave down" disabled={register <= SOUNDFONT_REGISTER_MIN} onclick={() => shiftRegister(-1)}>−</button>
      <span class="mono strong sf-reg" title="The two octaves the keyboard and the home row cover">{registerLabel(register)}</span>
      <button type="button" class="sf-mb" aria-label="Register up" title="One octave up" disabled={register >= SOUNDFONT_REGISTER_MAX} onclick={() => shiftRegister(1)}>＋</button>
      <span class="micro sf-lbl sf-hold">length</span>
      <div class="sf-modes" role="group" aria-label="Preview length">
        {#each PREVIEWS as kind (kind)}
          <button type="button" class="sf-mb" class:on={preview === kind} aria-pressed={preview === kind} data-sf-preview={kind}
            title={kind === "short" ? "a quick tap: 0.4 s" : kind === "beat" ? `one beat at ${bpm} bpm` : kind === "bar" ? `one bar of four beats at ${bpm} bpm` : "a long hold: 2 s"} onclick={() => { preview = kind; }}>{kind}</button>
        {/each}
      </div>
      <span class="mono sf-reg" data-sf-preview-ms={previewMs} title="How long a played key is held; the envelope's dashed rule is where it lets go">{(previewMs / 1000).toFixed(previewMs % 1000 ? 2 : 1)}s</span>
    </div>

    <div class="sf-body">
      <!-- The oscillators make the raw tone; the stages on the right are what each note is put
           through, in order. -->
      <section class="sf-col" aria-label="Oscillator stack">
        <div class="sf-hd">osc stack <span class="sub">· the raw tone</span></div>
        {#each patch.o as osc, i (i)}
          {@const open = openLayer === i}
          {@const tile = waveTile(osc.w)}
          <div class="sf-layer" class:open>
            <button type="button" class="sf-layer-head" aria-expanded={open} title={open ? "Fold this layer" : "Open this layer's controls"} onclick={() => (openLayer = open ? null : i)}>
              <svg class="sf-wv" viewBox="0 0 26 12" aria-hidden="true"><path d={tile.d} /></svg>
              <span class="sf-sum">{oscSummary(osc)}</span>
              <span class="sf-lvlbar"><i style={`width:${osc.l}%`}></i></span>
              <span class="sf-sum lvl">{osc.l}</span>
            </button>
            {#if open}
              <div class="sf-layer-body">
                <div class="sf-waves" role="group" aria-label={`Layer ${i + 1} wave`}>
                  {#each WAVE_TILES as t (t.wave)}
                    <button type="button" class="sf-mb wave" class:on={osc.w === t.index} aria-pressed={osc.w === t.index} title={`Layer ${i + 1}: ${t.wave}`} onclick={() => editOsc(i, "w", t.index)}>{t.label}</button>
                  {/each}
                  {#if patch.o.length > 1}
                    <button type="button" class="sf-del" title="Remove this layer" aria-label="Remove this layer" onclick={() => removeLayer(i)}>
                      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" /></svg>
                    </button>
                  {/if}
                </div>
                <div class="sf-knobs">
                  {@render knob({ label: "st", value: osc.t, ...PATCH_PARAM.o.t, disp: String(osc.t), bipolar: true, hint: "Transpose: shifts this layer by whole semitones. +12 is an octave up, -12 an octave down; +7 against another layer gives a fifth.", set: (v) => editOsc(i, "t", v) })}
                  {@render knob({ label: "ct", value: osc.c, ...PATCH_PARAM.o.c, disp: String(osc.c), bipolar: true, hint: "Detune: nudges this layer slightly sharp or flat, in hundredths of a semitone. A few cents against another layer makes the sound thicker and slowly beating; 0 is dead in tune.", set: (v) => editOsc(i, "c", v) })}
                  {@render knob({ label: "lvl", value: osc.l, ...PATCH_PARAM.o.l, disp: String(osc.l), hint: "Level: how loud this layer is in the mix against the others. 0 silences it without removing it.", set: (v) => editOsc(i, "l", v) })}
                </div>
              </div>
            {/if}
          </div>
        {/each}
        {#if patch.o.length < PATCH_OSC_MAX}
          <button type="button" class="sf-layer-add" title={`Add an oscillator layer (${PATCH_OSC_MAX} max)`} onclick={addLayer}>＋ layer</button>
        {:else}
          <span class="micro sf-layer-full">{patch.o.length} of {PATCH_OSC_MAX} layers</span>
        {/if}
      </section>

      <section class="sf-col" aria-label="Shape">
        <div class="sf-hd">shape <span class="sub">· every note runs through these in order</span></div>
        <div class="sf-stages">
          <div class="sf-stage">
            <div class="sf-stage-hd">
              <span class="sf-stage-nm">envelope</span>
              <span class="sf-stage-sub">loudness over time</span>
              <div class="sf-modes" role="group" aria-label="Envelope">
                <button type="button" class="sf-mb" class:on={isEnvOff} aria-pressed={isEnvOff} title="No shaping: the note is at full volume the instant it is pressed and gone the instant it is let go, like an organ key. Turning any of these four knobs brings the envelope back." onclick={() => onToggleStage("e")}>OFF</button>
              </div>
            </div>
            <div class="sf-stage-body">
              {@render scopeEnv(patch.e)}
              <div class="sf-knobs">
                {@render knob({ label: "atk", value: patch.e.a, ...PATCH_PARAM.e.a, disp: secondsLabel(patch.e.a), hint: "Attack: how long a note takes to reach full volume. 0 is an instant pluck; a few seconds is a slow swell.", set: (v) => editNum("e", "a", v) })}
                {@render knob({ label: "dec", value: patch.e.d, ...PATCH_PARAM.e.d, disp: secondsLabel(patch.e.d), hint: "Decay: after the peak, how long the note takes to fall to the sustain level. Short is a snappy bite; long is a slow settle.", set: (v) => editNum("e", "d", v) })}
                {@render knob({ label: "sus", value: patch.e.s, ...PATCH_PARAM.e.s, disp: String(patch.e.s), hint: "Sustain: the volume a held note settles at, as a share of the peak. 100 holds at full; 0 fades out even while the key is down.", set: (v) => editNum("e", "s", v) })}
                {@render knob({ label: "rel", value: patch.e.r, ...PATCH_PARAM.e.r, disp: secondsLabel(patch.e.r), hint: "Release: how long the note takes to fade after the key is let go. 0 stops dead; high leaves a tail hanging.", set: (v) => editNum("e", "r", v) })}
              </div>
            </div>
          </div>
          <div class="sf-stage">
            <div class="sf-stage-hd">
              <span class="sf-stage-nm">filter</span>
              <span class="sf-stage-sub">tone</span>
              <div class="sf-modes" role="group" aria-label="Filter">
                <button type="button" class="sf-mb" class:on={isFilterOff} aria-pressed={isFilterOff} title="Wide open: the cutoff sits at the top of its range, well above anything you are playing, so the filter stays out of the way. It is not a bypass, because a patch has no way to say so: every note still runs through one gentle lowpass at that corner, and an output running at a low sample rate brings the corner down with it. Turning cut, res or env gives the filter a shape again." onclick={() => onToggleStage("f")}>WIDE</button>
                <button type="button" class="sf-mb" class:on={!isFilterOff && patch.f.m === 0} aria-pressed={!isFilterOff && patch.f.m === 0} title="Lowpass: keeps the lows and rolls off everything brighter than the cutoff. Warm and rounded; the classic synth tone." onclick={() => onFilterMode(0)}>LP</button>
                <button type="button" class="sf-mb" class:on={!isFilterOff && patch.f.m === 1} aria-pressed={!isFilterOff && patch.f.m === 1} title="Highpass: keeps the highs and rolls off everything below the cutoff. Thin and airy; it cuts through a busy room." onclick={() => onFilterMode(1)}>HP</button>
                <button type="button" class="sf-mb" class:on={!isFilterOff && patch.f.m === 2} aria-pressed={!isFilterOff && patch.f.m === 2} title="Bandpass: keeps only a band around the cutoff and rolls off both sides. Nasal and hollow, like a small speaker." onclick={() => onFilterMode(2)}>BP</button>
              </div>
            </div>
            <div class="sf-stage-body">
              {@render scopeFilter(patch.f)}
              <div class="sf-knobs">
                {@render knob({ label: "cut", value: patch.f.c, ...PATCH_PARAM.f.c, disp: cutoffLabel(patch.f.c), hint: "Cutoff: where the filter starts to bite, in Hz. On lowpass, lower is darker; on highpass, higher is thinner; on bandpass it is the centre of the band.", set: (v) => editNum("f", "c", v) })}
                {@render knob({ label: "res", value: patch.f.q, ...PATCH_PARAM.f.q, disp: String(patch.f.q), hint: "Resonance: a peak right at the cutoff. 0 is smooth; high makes the cutoff ring and whistle, and any sweep of it turns squelchy.", set: (v) => editNum("f", "q", v) })}
                {@render knob({ label: "env", value: patch.f.e, ...PATCH_PARAM.f.e, disp: String(patch.f.e), bipolar: true, hint: "Envelope amount: how far each note sweeps the cutoff. Positive opens the filter on the attack and closes it through the decay (the classic wow); negative dips it instead; 0 holds it still. Full is six octaves.", set: (v) => editNum("f", "e", v) })}
              </div>
            </div>
          </div>
          <div class="sf-stage">
            <div class="sf-stage-hd">
              <span class="sf-stage-nm">lfo</span>
              <span class="sf-stage-sub">movement</span>
              <div class="sf-modes" role="group" aria-label="What the LFO moves">
                <button type="button" class="sf-mb" class:on={patch.l.t === 0} aria-pressed={patch.l.t === 0} title="No wobble: the sound holds steady once the envelope has settled." onclick={() => editNum("l", "t", 0)}>OFF</button>
                <button type="button" class="sf-mb" class:on={patch.l.t === 1} aria-pressed={patch.l.t === 1} title="The wobble sweeps the filter cutoff up and down: slow for a wah, fast for a growl. Depth sets how far, up to four octaves." onclick={() => editNum("l", "t", 1)}>CUT</button>
                <button type="button" class="sf-mb" class:on={patch.l.t === 2} aria-pressed={patch.l.t === 2} title="The wobble bends the pitch up and down for a vibrato. Even at full depth it stays within a quarter of a semitone." onclick={() => editNum("l", "t", 2)}>PIT</button>
              </div>
            </div>
            <div class="sf-stage-body">
              {@render scopeLfo(patch.l)}
              <div class="sf-knobs">
                {@render knob({ label: "rate", value: patch.l.r, ...PATCH_PARAM.l.r, disp: lfoRateLabel(patch.l.r), hint: "Rate: how fast the wobble cycles, in Hz. Under 1 is a slow drift, 5 to 7 is a vibrato, 12 is a buzz.", set: (v) => editNum("l", "r", v) })}
                {@render knob({ label: "dep", value: patch.l.d, ...PATCH_PARAM.l.d, disp: String(patch.l.d), hint: "Depth: how far the wobble reaches. 0 is none even with a target picked; 100 is the full sweep.", set: (v) => editNum("l", "d", v) })}
              </div>
            </div>
          </div>
          <div class="sf-stage">
            <div class="sf-stage-hd">
              <span class="sf-stage-nm">room sends</span>
              <span class="sf-stage-sub">space</span>
              <div class="sf-modes" role="group" aria-label="Room sends">
                <button type="button" class="sf-mb" class:on={isSendsOff} aria-pressed={isSendsOff} title="Dry: none of this sound reaches the room's chorus, echo or reverb. Turning any send back up brings it in." onclick={() => onToggleStage("x")}>OFF</button>
              </div>
            </div>
            <div class="sf-stage-body">
              {@render scopeSends(patch.x)}
              <div class="sf-knobs">
                {@render knob({ label: "cho", value: patch.x.c, ...PATCH_PARAM.x.c, disp: String(patch.x.c), hint: "Chorus send: how much of this sound goes to the room's shared chorus, which doubles it with a slowly drifting copy. Thicker and wider; 0 is dry.", set: (v) => editNum("x", "c", v) })}
                {@render knob({ label: "del", value: patch.x.d, ...PATCH_PARAM.x.d, disp: String(patch.x.d), hint: "Delay send: how much goes to the room's shared echo, a repeat about a quarter of a second later that trails off. 0 is dry.", set: (v) => editNum("x", "d", v) })}
                {@render knob({ label: "rev", value: patch.x.r, ...PATCH_PARAM.x.r, disp: String(patch.x.r), hint: "Reverb send: how much goes to the room's shared reverb, a small space that softens the edges. 0 is dry.", set: (v) => editNum("x", "r", v) })}
              </div>
            </div>
          </div>
        </div>
      </section>
    </div>

    <!-- Three destinations with three lifetimes: this device, the server's share, this flipnote. -->
    <footer class="sf-foot">
      <button type="button" class="sf-btn" disabled={!valid || !!busy} title="Keep this soundfont on this device, as a tile in your library" onclick={() => void run("save")}>
        {busy === "save" ? "saving…" : "save to my library"} <span class="dim">· {libraryCount} of {libraryMax}</span>
      </button>
      {#if shareName}
        <button type="button" class="sf-btn" disabled={!valid || !!busy} title={`Seal this soundfont into ${shareName}'s encrypted share, so anyone there can load it`} onclick={() => void run("share")}>
          {busy === "share" ? "sharing…" : `share to ${shareName}`} <span class="dim">· .jampatch</span>
        </button>
      {/if}
      <span class="sf-grow"></span>
      <button type="button" class="sf-btn ghost" title="Close without using this soundfont" onclick={close}>cancel</button>
      <button type="button" class="sf-btn primary" disabled={!valid || !!busy} title="Add this soundfont to the flipnote and select it" onclick={() => void run("use")}>
        {busy === "use" ? "adding…" : targetName ? `use in ${targetName}` : "use"} <span class="dim">· {recipeCount} of {recipeMax}</span>
      </button>
    </footer>
  </div>
</div>

<style>
  /* Scoped to this component. Every colour is a design token; the two hex literals are the piano's
     ivory and ebony keys, which must stay the colours of a piano whatever the theme's palette does.
     Buttons set their own background, border, colour and font explicitly because the app's global
     `button` rule paints every button accent and bold; a scoped class outranks it. */
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .sf-grow { flex: 1; }

  .sf-overlay { position: fixed; inset: 0; z-index: 60; display: flex; align-items: center; justify-content: center; padding: 16px; background: color-mix(in oklab, var(--bg-0) 55%, transparent); }
  .sf-card { container: sf-card / inline-size; display: flex; flex-direction: column; width: min(760px, 100%); max-height: 90vh; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 18px 44px color-mix(in oklab, var(--bg-0) 60%, transparent); overflow: hidden; color: var(--text); font-family: var(--ui); font-size: 0.84rem; }

  /* Head */
  .sf-head { display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border); flex: none; }
  .sf-note { width: 14px; height: 14px; flex: none; fill: none; stroke: var(--accent); stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .sf-hd { font-family: var(--mono); font-size: 0.6rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); display: flex; align-items: center; gap: 6px; white-space: nowrap; }
  .sf-hd .sub { font-weight: 400; letter-spacing: 0.02em; text-transform: none; color: var(--faint); min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .sf-name { width: 9rem; flex: none; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.74rem; padding: 4px 8px; }
  .sf-name:focus { outline: 1px solid var(--accent); outline-offset: 0; }
  .sf-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; }
  .sf-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .sf-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); }
  .sf-chip.warn .micro { color: var(--warn); }
  .sf-chip.warn i { background: var(--warn); }

  /* Buttons: the Studio's grammar (mono, uppercase, 26px tall). */
  .sf-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; font-weight: 400; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; cursor: pointer; }
  .sf-btn:hover { border-color: var(--accent); color: var(--text); background: var(--bg-elev); }
  .sf-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .sf-btn.primary:hover { background: var(--accent-hi); border-color: var(--accent-hi); color: var(--on-accent); }
  .sf-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .sf-btn.ghost:hover { background: transparent; border-color: var(--border); color: var(--text-2); }
  .sf-btn.icon { width: 26px; padding: 0; justify-content: center; }
  .sf-btn.icon svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; }
  .sf-btn:disabled { opacity: 0.45; cursor: not-allowed; }
  .sf-btn .dim { color: var(--faint); }
  .sf-btn.primary .dim { color: var(--on-accent); opacity: 0.7; }
  .sf-btn:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }

  /* Rows under the head: the shelf and the keyboard. */
  .sf-row { display: flex; align-items: center; gap: 8px; padding: 8px 14px; flex: none; min-width: 0; }
  .sf-row.shelf { border-bottom: 1px solid var(--border-soft); flex-wrap: wrap; row-gap: 6px; }
  .sf-row.hear { border-bottom: 1px solid var(--border); }
  .sf-lbl { width: 64px; flex: none; }
  .sf-lbl.sf-hold { width: auto; margin-left: 10px; }
  .sf-rk { display: inline-flex; flex-direction: column; align-items: center; gap: 1px; padding: 3px 7px; border: 1px solid var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); cursor: pointer; }
  .sf-rk:hover { background: var(--bg-elev); border-color: var(--accent); color: var(--text); }
  .sf-rk svg { width: 26px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .sf-rk .nm { font-family: var(--mono); font-size: 0.52rem; letter-spacing: 0.08em; text-transform: uppercase; max-width: 7rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sf-rk.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .sf-rk.on:hover { background: var(--accent-dim); color: var(--accent-hi); }
  .sf-rk.add { border-style: dashed; color: var(--faint); }
  .sf-rk:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }
  .sf-file { display: none; }

  /* The piano: ivory and ebony are the one place a literal colour is right. */
  .sf-piano { position: relative; flex: 1; min-width: 120px; height: 40px; border: 1px solid var(--border); border-radius: 3px; overflow: hidden; background: var(--bg-elev); }
  .sf-piano .whites { display: flex; height: 100%; }
  .pk { flex: 1; min-width: 0; height: 100%; padding: 0; border: none; border-right: 1px solid var(--border-soft); border-radius: 0; background: #e9e4f2; cursor: pointer; }
  .pk:last-child { border-right: none; }
  .pk:hover { background: #e9e4f2; }
  .pk:active { background: var(--accent); }
  .pb { position: absolute; top: 0; height: 58%; padding: 0; background: #2a2735; border: 1px solid var(--bg-0); border-top: none; border-radius: 0 0 2px 2px; cursor: pointer; }
  .pb:hover { background: #2a2735; }
  .pb:active { background: var(--accent-hi); }
  .sf-reg { min-width: 4.2em; text-align: center; }

  /* Mode buttons, wave tiles and the register stepper share one small chip. */
  .sf-mb { padding: 1px 7px; border: 1px solid var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); font-family: var(--mono); font-size: 0.56rem; font-weight: 400; letter-spacing: 0.08em; text-transform: uppercase; line-height: 1.5; cursor: pointer; white-space: nowrap; }
  .sf-mb:hover { background: var(--bg-elev); border-color: var(--accent); color: var(--text); }
  .sf-mb.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .sf-mb.on:hover { background: var(--accent-dim); color: var(--accent-hi); }
  .sf-mb:disabled { opacity: 0.45; cursor: not-allowed; }
  .sf-mb:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }
  .sf-mb.wave { flex: 1 1 0; min-width: 0; text-align: center; padding: 2px 0; }

  /* Body: osc stack left, the four stages right; one column when the card is narrow. */
  .sf-body { flex: 1; min-height: 0; overflow: auto; display: grid; grid-template-columns: 250px minmax(0, 1fr); gap: 10px; padding: 10px 14px; align-content: start; scrollbar-gutter: stable; }
  .sf-col { display: flex; flex-direction: column; gap: 6px; min-width: 0; }

  /* Layers: collapsed rows read like mixer channels; one unfolds at a time. */
  .sf-layer { border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); }
  .sf-layer.open { border-color: var(--accent); }
  .sf-layer-head { display: flex; align-items: center; gap: 8px; width: 100%; padding: 5px 9px; background: transparent; border: none; border-radius: var(--r); color: var(--text-2); font-weight: 400; text-align: left; cursor: pointer; }
  .sf-layer-head:hover { background: transparent; color: var(--text); }
  .sf-layer.open .sf-layer-head { color: var(--accent-hi); }
  .sf-layer.open .sf-layer-head:hover { color: var(--accent-hi); }
  .sf-layer-head:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .sf-wv { width: 26px; height: 12px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.4; stroke-linecap: round; stroke-linejoin: round; }
  .sf-sum { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.04em; white-space: nowrap; }
  .sf-sum.lvl { color: var(--muted); min-width: 2em; text-align: right; }
  .sf-lvlbar { flex: 1; min-width: 24px; height: 5px; border-radius: 999px; background: var(--bg-0); overflow: hidden; }
  .sf-lvlbar i { display: block; height: 100%; background: var(--accent); opacity: 0.85; }
  .sf-layer-body { display: flex; flex-direction: column; gap: 6px; padding: 2px 9px 8px; }
  .sf-waves { display: flex; gap: 3px; align-items: center; }
  .sf-del { width: 18px; height: 18px; flex: none; padding: 0; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--border); border-radius: 50%; background: transparent; color: var(--muted); cursor: pointer; }
  .sf-del svg { width: 9px; height: 9px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; }
  .sf-del:hover { background: transparent; color: var(--danger); border-color: var(--danger-brd); }
  .sf-layer-add { border: 1px dashed var(--border); border-radius: var(--r); background: transparent; color: var(--faint); font-family: var(--mono); font-size: 0.6rem; font-weight: 400; letter-spacing: 0.1em; padding: 4px 0; text-align: center; cursor: pointer; }
  .sf-layer-add:hover { background: transparent; color: var(--accent-hi); border-color: var(--accent); }
  .sf-layer-full { text-align: center; border: 1px dashed var(--border-soft); border-radius: var(--r); padding: 4px 0; }

  /* Knobs */
  .sf-knobs { display: flex; flex-wrap: wrap; gap: 4px 10px; }
  .sf-knob { display: flex; flex-direction: column; align-items: center; gap: 1px; width: 44px; cursor: ns-resize; touch-action: none; user-select: none; border-radius: var(--r); }
  .sf-knob:focus-visible { outline: 1px solid var(--accent); outline-offset: 1px; }
  .sf-knob-face { width: 30px; height: 30px; }
  .sf-knob-ring { fill: var(--bg-elev); stroke: var(--border); stroke-width: 1.5; }
  .sf-knob:hover .sf-knob-ring, .sf-knob:focus-visible .sf-knob-ring { stroke: var(--accent); }
  .sf-knob-arc { fill: none; stroke: var(--accent); stroke-width: 2; opacity: 0.85; }
  .sf-knob-tick { stroke: var(--accent-hi); stroke-width: 2; stroke-linecap: round; }
  .sf-knob-lbl { font-family: var(--mono); font-size: 0.54rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); }
  .sf-knob-val { font-family: var(--mono); font-size: 0.58rem; color: var(--text-2); }

  /* The four stages wear the layer card's frame, so the editor is one family of cards. */
  .sf-stages { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; }
  .sf-stage { container: sf-stage / inline-size; display: flex; flex-direction: column; gap: 5px; min-width: 0; padding: 5px 9px 7px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); }
  .sf-stage-hd { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 6px; min-width: 0; }
  .sf-stage-nm { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--text-2); white-space: nowrap; }
  .sf-stage-sub { flex: 1; min-width: 0; font-family: var(--mono); font-size: 0.56rem; color: var(--faint); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sf-modes { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 3px; flex: none; margin-left: auto; }
  .sf-stage-body { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 8px; }
  .sf-stage .sf-knobs { gap: 4px 4px; }

  /* Scopes: the box is stretched to whatever width it gets; strokes stay in screen pixels. */
  .sf-scope { flex: 1 1 72px; min-width: 72px; max-width: 150px; height: 32px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-0); }
  .sf-scope path { vector-effect: non-scaling-stroke; stroke-linecap: round; stroke-linejoin: round; }
  .sf-scope .grid { fill: none; stroke: var(--border); stroke-width: 1; }
  .sf-scope .trace { fill: none; stroke: var(--accent-hi); stroke-width: 1.5; }
  .sf-scope .fill { fill: var(--accent-dim); stroke: none; }
  .sf-scope .tail { fill: var(--accent-dim); stroke: none; }
  .sf-scope .mark { fill: none; stroke: var(--faint); stroke-width: 1; stroke-dasharray: 2 2; }
  .sf-scope .sweep { fill: none; stroke: var(--accent); stroke-width: 2; stroke-dasharray: 3 2; }
  .sf-scope .bar { fill: none; stroke: var(--accent-hi); stroke-width: 2.5; }
  .sf-scope .bar.soft { stroke: var(--accent); opacity: 0.6; }
  /* Without room for a scope beside a row of knobs, the scope becomes a strip above them. */
  @container sf-stage (max-width: 299px) {
    .sf-scope { flex-basis: 100%; max-width: none; height: 26px; }
  }

  /* Foot */
  .sf-foot { display: flex; align-items: center; gap: 8px; padding: 10px 14px; border-top: 1px solid var(--border); background: var(--bg-elev); flex: none; flex-wrap: wrap; }

  /* A narrow card stacks the stack and the stages, and the stages go one under another. */
  @container sf-card (max-width: 560px) {
    .sf-body { grid-template-columns: minmax(0, 1fr); }
    .sf-stages { grid-template-columns: minmax(0, 1fr); }
    .sf-name { width: 6rem; }
  }
</style>
