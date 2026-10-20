<script lang="ts">
  /**
   * The clip maker's inspector: one row under the stage holding the controls for whatever is
   * selected (text, a sticker, a sound, an added video, or the clip itself when nothing is).
   *
   * It only shows values and reports changes. The editor turns each report into a document change.
   * A run of reports from one control (typing a word, dragging a slider) is one undo step:
   * `onsettle` marks where that run ends (the control's `change`/blur), and the editor groups
   * everything before it.
   */
  import { TEXT_EFFECT_GROUPS, TEXT_EFFECTS } from "./message-effects.ts";
  import type { Overlay, TextContent } from "./clip-doc.ts";
  import { formatClock } from "./video-edit.ts";
  import { TEXT_COLOURS, type InspectorTarget } from "./video-editor-ui.ts";

  type Props = {
    target: InspectorTarget | null;
    recording: boolean;
    /** Editing is locked (exporting). */
    locked: boolean;
    ontext: (patch: { text?: string; effect?: string | null; color?: string }) => void;
    /** Size as a fraction of the frame height. */
    onsize: (scale: number) => void;
    onturn: (degrees: number) => void;
    onrecord: () => void;
    onclearpath: () => void;
    onaudio: (patch: { volume?: number; muted?: boolean }) => void;
    onlayer: (patch: { volume?: number; muted?: boolean; size?: number }) => void;
    onmovelayer: (step: 1 | -1) => void;
    ondelete: () => void;
    onsettle: () => void;
  };
  let { target, recording, locked, ontext, onsize, onturn, onrecord, onclearpath, onaudio, onlayer, onmovelayer, ondelete, onsettle }: Props = $props();

  const effectGroups = TEXT_EFFECT_GROUPS.map((group) => ({ group, items: TEXT_EFFECTS.filter((e) => e.group === group) }));
  const pct = (v: number) => Math.round(v * 100);
  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
</script>

<div class="insp" class:locked data-inspector={target?.kind ?? "none"}>
  {#if target?.kind === "overlay" && target.overlay.content.kind === "text"}
    {@const content = target.overlay.content as TextContent}
    <span class="badge" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M3 4V2.5h10V4M8 2.5v11M6 13.5h4"></path></svg></span>
    <input class="field text" type="text" data-field="text" aria-label="Text" maxlength="120" value={content.text} disabled={locked}
      oninput={(e) => ontext({ text: (e.currentTarget as HTMLInputElement).value })} onchange={onsettle} onblur={onsettle} />
    <label class="pair"><span class="micro">effect</span>
      <select class="field" data-field="effect" disabled={locked} value={content.effect ?? ""}
        onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; ontext({ effect: v || null }); onsettle(); }}>
        <option value="">Plain</option>
        {#each effectGroups as g (g.group)}
          <optgroup label={g.group}>{#each g.items as fx (fx.id)}<option value={fx.id}>{fx.label}</option>{/each}</optgroup>
        {/each}
      </select>
    </label>
    <span class="swatches" role="group" aria-label="Colour">
      {#each TEXT_COLOURS as c (c.hex)}
        <button type="button" class="swatch" class:on={content.color.toLowerCase() === c.hex} style:background={c.hex} aria-pressed={content.color.toLowerCase() === c.hex}
          title={c.name} aria-label={c.name} disabled={locked} onclick={() => { ontext({ color: c.hex }); onsettle(); }}></button>
      {/each}
    </span>
    <label class="pair"><span class="micro">size</span>
      <input type="range" min="3" max="60" value={pct(target.placement.scale)} disabled={locked} data-field="size" aria-label="Size"
        oninput={(e) => onsize(num(e) / 100)} onchange={onsettle} /></label>
    <span class="grow"></span>
    <!-- No record button here: the words need the room, and R (or the transport's record tool)
         records any selected overlay. -->
    {@render keys(target.overlay, false)}
    <button type="button" class="st-tool ol" data-action="delete-item" title="Delete (Del)" aria-label="Delete text" disabled={locked} onclick={ondelete}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg></button>
  {:else if target?.kind === "overlay"}
    <span class="badge" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M13.5 9.5V4A1.5 1.5 0 0 0 12 2.5H4A1.5 1.5 0 0 0 2.5 4v8A1.5 1.5 0 0 0 4 13.5h5.5M13.5 9.5l-4 4"></path></svg></span>
    <span class="name" title={target.name}>{target.name}</span>
    <label class="pair"><span class="micro">size</span>
      <input type="range" min="3" max="100" value={pct(target.placement.scale)} disabled={locked} data-field="size" aria-label="Size"
        oninput={(e) => onsize(num(e) / 100)} onchange={onsettle} /></label>
    <label class="pair"><span class="micro">turn</span>
      <input class="field deg" type="number" min="-360" max="360" step="1" value={Math.round(target.placement.rotation)} disabled={locked} data-field="turn" aria-label="Turn, in degrees"
        oninput={(e) => { const v = num(e); if (Number.isFinite(v)) onturn(v); }} onchange={onsettle} /></label>
    <span class="grow"></span>
    {@render keys(target.overlay, true)}
    <button type="button" class="st-tool ol" data-action="delete-item" title="Delete (Del)" aria-label="Delete sticker" disabled={locked} onclick={ondelete}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg></button>
  {:else if target?.kind === "audio"}
    {@const item = target.item}
    <span class="badge" aria-hidden="true">{#if item.kind === "music"}<svg viewBox="0 0 16 16"><path d="M6 12V3.5l7-1.5v8.5M3 12a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0M10 10.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0"></path></svg>{:else}<svg viewBox="0 0 16 16"><path d="M1.5 8h1M4.5 5.5v5M7 2.5v11M9.5 5v6M12 3.5v9M14.5 7v2"></path></svg>{/if}</span>
    <span class="name" title={target.name}>{target.name}</span>
    <span class="ve-chip"><i></i><span class="micro">{target.origin}</span></span>
    {@render volume(item.volume, (v) => onaudio({ volume: v }))}
    {@render mute(item.muted, () => { onaudio({ muted: !item.muted }); onsettle(); }, target.name)}
    <span class="micro" title="Starts at">at</span><span class="mono">{formatClock(item.start)}</span>
    <span class="grow"></span>
    <button type="button" class="st-tool ol" data-action="delete-item" title="Delete (Del)" aria-label={`Delete ${target.name}`} disabled={locked} onclick={ondelete}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg></button>
  {:else if target?.kind === "layer"}
    {@const layer = target.layer}
    <span class="badge" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M2 4.5h8.5v7H2zM10.5 7l3.5-2v6l-3.5-2z"></path></svg></span>
    <span class="name" title={target.name}>{target.name}</span>
    <span class="ve-chip"><i></i><span class="micro">{target.label}</span></span>
    {@render mute(layer.muted, () => { onlayer({ muted: !layer.muted }); onsettle(); }, target.label)}
    {@render volume(layer.volume, (v) => onlayer({ volume: v }))}
    <label class="pair"><span class="micro">size</span>
      <input type="range" min="10" max="100" value={pct(layer.box.w)} disabled={locked} data-field="layer-size" aria-label="Size"
        oninput={(e) => onlayer({ size: num(e) / 100 })} onchange={onsettle} /></label>
    <button type="button" class="st-tool ol" data-action="layer-forward" title="Bring forward" aria-label="Bring forward" disabled={locked || target.last} onclick={() => onmovelayer(1)}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 13V3M4 7l4-4 4 4"></path></svg></button>
    <button type="button" class="st-tool ol" data-action="layer-back" title="Send back" aria-label="Send back" disabled={locked || target.first} onclick={() => onmovelayer(-1)}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M4 9l4 4 4-4"></path></svg></button>
    <span class="grow"></span>
    <button type="button" class="st-tool ol" data-action="delete-item" title="Delete (Del)" aria-label={`Delete ${target.label}`} disabled={locked} onclick={ondelete}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg></button>
  {:else if target?.kind === "base"}
    {@const base = target.layer}
    <span class="badge" aria-hidden="true"><svg viewBox="0 0 16 16"><path d="M2 4.5h8.5v7H2zM10.5 7l3.5-2v6l-3.5-2z"></path></svg></span>
    <span class="name" title={target.name}>{target.name}</span>
    <span class="ve-chip"><i></i><span class="micro">V1</span></span>
    <!-- A clip with no sound has nothing to mute or turn down (doing so would only re-render it). -->
    {#if target.hasAudio || base.muted || base.volume < 1}
      {@render mute(base.muted, () => { onlayer({ muted: !base.muted }); onsettle(); }, "V1")}
      {@render volume(base.volume, (v) => onlayer({ volume: v }))}
    {/if}
  {/if}
</div>

{#snippet keys(overlay: Overlay, withRecord: boolean)}
  {#if overlay.keyframes.length > 1}
    <span class="ve-chip accent" title="Keyframes on its path"><i class="diamond"></i><span class="micro">{overlay.keyframes.length} keys</span></span>
    <button type="button" class="st-tool ol" data-action="clear-path" title="Clear path" aria-label="Clear path" disabled={locked || recording} onclick={onclearpath}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 12.5c2-4 4.5-6.5 8-7" stroke-dasharray="0.1 2.6"></path><path d="M10.5 10.5l3 3M13.5 10.5l-3 3"></path></svg></button>
  {/if}
  {#if withRecord}
    <button type="button" class="st-btn rec" class:on={recording} data-action="record-item" aria-pressed={recording} disabled={locked}
      title={recording ? "Stop recording (R)" : "Record motion: plays, and your drag becomes its path (R)"} onclick={onrecord}>
      {#if recording}<svg viewBox="0 0 16 16" aria-hidden="true"><rect x="3" y="3" width="10" height="10" rx="1.5"></rect></svg>stop{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6"></circle></svg>record{/if}
    </button>
  {/if}
{/snippet}

{#snippet volume(value: number, set: (v: number) => void)}
  <label class="pair"><span class="micro">vol</span>
    <input type="range" min="0" max="100" value={pct(value)} disabled={locked} data-field="volume" aria-label="Volume"
      oninput={(e) => set(num(e) / 100)} onchange={onsettle} /><span class="mono">{pct(value)}%</span></label>
{/snippet}

{#snippet mute(muted: boolean, toggle: () => void, what: string)}
  <button type="button" class="st-tool ol" class:on={muted} data-action="mute-item" aria-pressed={muted} title={muted ? "Unmute" : "Mute"} aria-label={`${muted ? "Unmute" : "Mute"} ${what}`} disabled={locked} onclick={toggle}>
    {#if muted}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 6l3 4M14 6l-3 4"></path></svg>
    {:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4"></path></svg>{/if}
  </button>
{/snippet}

<style>
  .insp { height: 34px; flex: none; display: flex; align-items: center; gap: 8px; padding: 0 10px 0 12px; box-sizing: border-box; border-top: 1px solid var(--border); background: var(--panel); min-width: 0; overflow: hidden; }
  .insp.locked { opacity: 0.6; }
  .badge { width: 22px; height: 22px; flex: none; box-sizing: border-box; border-radius: var(--r); border: 1px solid color-mix(in oklab, var(--accent) 40%, transparent); background: var(--accent-dim); color: var(--accent-hi); display: inline-flex; align-items: center; justify-content: center; }
  .badge svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  .name { font-size: 0.8rem; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 130px; min-width: 0; }
  .field { height: 24px; box-sizing: border-box; padding: 0 8px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text); font: inherit; font-size: 0.8rem; color-scheme: dark; outline: none; min-width: 0; }
  .field:focus-visible { border-color: var(--accent); }
  .field.text { width: 120px; font-weight: 600; flex: none; }
  select.field { width: 92px; padding: 0 4px; }
  .field.deg { width: 52px; font-family: var(--mono); font-size: 0.72rem; text-align: right; }
  .pair { display: inline-flex; align-items: center; gap: 6px; flex: none; }
  .pair input[type="range"] { width: 60px; height: 14px; margin: 0; accent-color: var(--accent); color-scheme: dark; }
  .swatches { display: inline-flex; gap: 4px; flex: none; }
  .swatch { width: 16px; height: 16px; padding: 0; border: 1px solid var(--border); border-radius: 4px; cursor: pointer; }
  .swatch.on { box-shadow: 0 0 0 1.5px var(--panel), 0 0 0 3px var(--accent-hi); }
  .grow { flex: 1; }
  .ve-chip.accent { border-color: color-mix(in oklab, var(--accent) 45%, transparent); background: var(--accent-dim); }
  .ve-chip.accent .micro { color: var(--accent-hi); }
  .diamond { width: 5px !important; height: 5px !important; border-radius: 0 !important; transform: rotate(45deg); background: var(--accent-hi) !important; }
  .rec svg { width: 10px; height: 10px; fill: var(--danger); stroke: none; }
  .rec.on { border-color: var(--danger-brd); background: var(--danger-dim); color: var(--danger-hi); }
  .rec.on svg { fill: currentColor; }
</style>
