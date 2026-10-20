<script lang="ts">
  /**
   * The clip maker's library: the column of things to put on a clip, in five tabs.
   *
   * - Text: every text effect from chat, grouped as in chat's picker, plus plain text. Each chip
   *   previews its effect on "Aa". The previews sit inside `.text-fx-key-preview`, which the app's
   *   effect runtime treats as a picker preview: one-shot entrances play silently there, never with
   *   the sound authored messages make (see App's `discover`).
   * - Stickers: pictures from disk, and emoji.
   * - Music: this channel's Studio songs, and music files (audio or `.jamtake`) from disk.
   * - Sounds: the built-in cartoon sounds (`clip-sfx.ts`), and sound files from disk.
   * - Video: the clip's layers, and another clip from disk (three layers at most).
   *
   * This component only shows and asks. Every addition goes to the editor through a callback, which
   * decodes, keeps the media and changes the document; nothing here touches the document or the
   * media bin. Adding always places the thing at the playhead.
   */
  import { TEXT_EFFECT_GROUPS, TEXT_EFFECTS, textEffectHtml } from "./message-effects.ts";
  import { SFX } from "./clip-sfx.ts";
  import type { ClipSong } from "./clip-music.ts";
  import { FILE_INTENTS, type FileIntent, type LibraryTab, type SongState, type SongsStatus } from "./video-editor-ui.ts";

  type Props = {
    tab: LibraryTab;
    ontab: (tab: LibraryTab) => void;
    /** Pictures already brought in from disk, newest last. */
    images: readonly { id: string; name: string; image: CanvasImageSource; width: number; height: number }[];
    /** Sound files from disk, by where they were added (the Music tab or the Sounds tab). */
    musicFiles: readonly { id: string; name: string; seconds: number }[];
    soundFiles: readonly { id: string; name: string; seconds: number }[];
    /** The channel's Studio songs, once read (`songsStatus` says whether they have been). */
    songs: readonly ClipSong[];
    songsStatus: SongsStatus;
    songsError: string;
    songState: Readonly<Record<string, SongState>>;
    /** Whether a Studio is there to ask at all. */
    hasStudio: boolean;
    /** Ids of sounds and songs already on the clip, shown ticked. */
    onClip: ReadonlySet<string>;
    /** `silent`: the video has no sound, so there is nothing to mute (the button is disabled). */
    layers: readonly { id: string; label: string; name: string; muted: boolean; silent: boolean; detail: string }[];
    layersFull: boolean;
    /** The sound currently auditioned, by key, so its button can stop it. */
    auditioning: string | null;
    /** Nothing can be added (exporting, recording). */
    locked: boolean;
    onaddtext: (effect: string | null) => void;
    onaddemoji: (glyph: string, name: string) => void;
    onaddimage: (id: string) => void;
    onaddsfx: (id: string) => void;
    onaudition: (key: string) => void;
    onaddsound: (id: string, kind: "music" | "sfx") => void;
    onaddsong: (song: ClipSong) => void;
    onlistsongs: () => void;
    onfiles: (files: File[], intent: FileIntent) => void;
    onlayermute: (id: string) => void;
  };
  let {
    tab, ontab, images, musicFiles, soundFiles, songs, songsStatus, songsError, songState, hasStudio, onClip, layers, layersFull,
    auditioning, locked, onaddtext, onaddemoji, onaddimage, onaddsfx, onaudition, onaddsound, onaddsong, onlistsongs,
    onfiles, onlayermute,
  }: Props = $props();

  const TABS: readonly { id: LibraryTab; label: string; icon: string }[] = [
    { id: "text", label: "Text", icon: "M3 4V2.5h10V4M8 2.5v11M6 13.5h4" },
    { id: "stickers", label: "Stickers", icon: "M13.5 9.5V4A1.5 1.5 0 0 0 12 2.5H4A1.5 1.5 0 0 0 2.5 4v8A1.5 1.5 0 0 0 4 13.5h5.5M13.5 9.5l-4 4M13.5 9.5H11a1.5 1.5 0 0 0-1.5 1.5v2.5M5.5 6.5h.01M9.5 6.5h.01" },
    { id: "music", label: "Music", icon: "M6 12V3.5l7-1.5v8.5M3 12a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0M10 10.5a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0" },
    { id: "sounds", label: "Sounds", icon: "M1.5 8h1M4.5 5.5v5M7 2.5v11M9.5 5v6M12 3.5v9M14.5 7v2" },
    { id: "video", label: "Video", icon: "M2 4.5h8.5v7H2zM10.5 7l3.5-2v6l-3.5-2z" },
  ];

  /** Sticker emoji: expressive, readable at small sizes, a few cats for the house style. */
  const EMOJI: readonly [string, string][] = [
    ["🐱", "cat face"], ["😹", "laughing cat"], ["🙀", "shocked cat"], ["😼", "smirking cat"], ["💥", "boom"], ["⭐", "star"],
    ["🔥", "fire"], ["💖", "heart"], ["👀", "eyes"], ["🎉", "party"], ["⚡", "zap"], ["💨", "whoosh"],
    ["🌙", "moon"], ["🛹", "skateboard"], ["😎", "cool"], ["💯", "hundred"], ["🍕", "pizza"], ["🎸", "guitar"],
    ["👑", "crown"], ["💀", "skull"], ["🌈", "rainbow"], ["🎈", "balloon"], ["✨", "sparkles"], ["🚀", "rocket"],
  ];

  /** Effects by group, as in chat's picker. Pride labels drop the word "pride": the group says it. */
  const groups = TEXT_EFFECT_GROUPS.map((group) => ({
    group,
    items: TEXT_EFFECTS.filter((e) => e.group === group).map((e) => ({
      id: e.id, label: group === "Pride" ? e.label.replace(/ pride$/i, "") : e.label, description: e.description,
    })),
  }));

  const ACCEPT: Record<FileIntent, string> = {
    image: "image/png,image/jpeg,image/webp,image/gif,image/bmp",
    music: "audio/*,.jamtake",
    sound: "audio/*",
    video: "video/mp4,video/quicktime,video/webm",
  };
  let inputs: Partial<Record<FileIntent, HTMLInputElement>> = {};
  function pick(intent: FileIntent) { if (!locked) inputs[intent]?.click(); }
  function picked(e: Event, intent: FileIntent) {
    const input = e.currentTarget as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = "";
    if (files.length) onfiles(files, intent);
  }

  function seconds(s: number): string {
    if (!(s > 0)) return "";
    const m = Math.floor(s / 60), r = s - m * 60;
    return m ? `${m}:${r.toFixed(0).padStart(2, "0")}` : `${r.toFixed(1)} s`;
  }
  /** Action: show a decoded picture as a thumbnail. */
  function thumb(node: HTMLCanvasElement, image: CanvasImageSource) {
    const draw = (source: CanvasImageSource) => {
      const ctx = node.getContext?.("2d");
      if (!ctx) return;
      ctx.clearRect(0, 0, node.width, node.height);
      const sw = (source as { width: number }).width || 1, sh = (source as { height: number }).height || 1;
      const k = Math.min(node.width / sw, node.height / sh);
      try { ctx.drawImage(source, (node.width - sw * k) / 2, (node.height - sh * k) / 2, sw * k, sh * k); } catch { /* decoration */ }
    };
    draw(image);
    return { update: draw };
  }

  // Asked once, when the tab first opens. A failed read waits for the person's retry: asking again
  // by itself would loop for as long as the Studio cannot answer.
  $effect(() => { if (tab === "music" && hasStudio && songsStatus === "idle") onlistsongs(); });
</script>

<aside class="lib" aria-label="Library">
  <div class="lib-tabs" role="tablist" aria-label="Library">
    {#each TABS as t (t.id)}
      <button type="button" role="tab" class="lib-tab" class:on={tab === t.id} aria-selected={tab === t.id} title={t.label} data-tab={t.id} onclick={() => ontab(t.id)}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d={t.icon}></path></svg><span>{t.label}</span>
      </button>
    {/each}
  </div>
  <div class="lib-body" role="tabpanel" class:locked>
    {#if tab === "text"}
      {#each groups as g (g.group)}
        <div class="micro lib-head">{g.group}</div>
        <div class="fx-grid">
          {#if g.group === "Utility"}
            <button type="button" class="fx-chip" data-add-text="plain" disabled={locked} title="Plain text" onclick={() => onaddtext(null)}>
              <span class="fx-aa plain">Aa</span><span class="fx-name">Plain</span>
            </button>
          {/if}
          {#each g.items as e (e.id)}
            <button type="button" class="fx-chip" data-add-text={e.id} disabled={locked} title={e.description} onclick={() => onaddtext(e.id)}>
              <!-- Our own module's markup for our own effect id and a fixed word. -->
              <span class="fx-aa text-fx-key-preview">{@html textEffectHtml(e.id, "Aa")}</span><span class="fx-name">{e.label}</span>
            </button>
          {/each}
        </div>
      {/each}
    {:else if tab === "stickers"}
      <div class="micro lib-head">pictures</div>
      <div class="pic-row">
        <button type="button" class="add-tile" data-action="add-image" disabled={locked} title="Add a picture from disk" onclick={() => pick("image")}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>add picture
        </button>
        {#each images as img (img.id)}
          <button type="button" class="pic" disabled={locked} title={img.name} aria-label={`Add ${img.name}`} onclick={() => onaddimage(img.id)}>
            <canvas width="64" height="56" use:thumb={img.image}></canvas>
          </button>
        {/each}
      </div>
      <div class="micro lib-head">emoji</div>
      <div class="emoji-grid">
        {#each EMOJI as [glyph, name] (glyph)}
          <button type="button" class="emoji" data-emoji={glyph} disabled={locked} title={name} aria-label={`Add ${name} sticker`} onclick={() => onaddemoji(glyph, name)}>{glyph}</button>
        {/each}
      </div>
    {:else if tab === "music"}
      {#if hasStudio}
        <div class="lib-head split"><span class="micro">studio songs</span>{#if songsStatus === "done"}<span class="micro">{songs.length}</span>{/if}</div>
        {#if songsStatus === "error"}
          <p class="lib-note warn">{songsError}</p>
          <button type="button" class="st-btn" data-action="retry-songs" onclick={onlistsongs}>try again</button>
        {:else if songsStatus !== "done"}
          <p class="lib-note" role="status">…</p>
        {:else if songs.length === 0}
          <p class="lib-note">none yet</p>
        {:else}
          {#each songs as song (song.id)}
            {@const s = songState[song.id]}
            <div class="row" class:busy={s?.state === "rendering"}>
              <span class="row-main">
                <span class="row-name" title={song.title}>{song.title}</span>
                {#if s?.state === "error"}<span class="row-sub warn" title={s.message}>{s.message}</span>
                {:else if s?.seconds}<span class="row-sub">{seconds(s.seconds)}</span>{/if}
              </span>
              {#if onClip.has(`studio:${song.id}`)}
                <span class="tick" title="On the clip"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8.5l3 3 7-7"></path></svg></span>
              {/if}
              <button type="button" class="st-tool sm ol" data-add-song={song.id} disabled={locked || s?.state === "rendering"} title="Add at the playhead" aria-label={`Add ${song.title}`} onclick={() => onaddsong(song)}>
                <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>
              </button>
            </div>
          {/each}
        {/if}
      {/if}
      <div class="micro lib-head">files</div>
      {#each musicFiles as m (m.id)}
        <div class="row">
          <button type="button" class="st-tool sm round" class:on={auditioning === m.id} title={auditioning === m.id ? "Stop" : "Listen"} aria-label={`Listen to ${m.name}`} onclick={() => onaudition(m.id)}>
            {#if auditioning === m.id}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 4h8v8H4z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}
          </button>
          <span class="row-main"><span class="row-name" title={m.name}>{m.name}</span><span class="row-sub">{seconds(m.seconds)}</span></span>
          <button type="button" class="st-tool sm ol" disabled={locked} title="Add at the playhead" aria-label={`Add ${m.name}`} onclick={() => onaddsound(m.id, "music")}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>
          </button>
        </div>
      {/each}
      <button type="button" class="add-wide" data-action="add-music-file" disabled={locked} title="Music from disk: a sound file, or a .jamtake from the Studio" onclick={() => pick("music")}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>add music file
      </button>
    {:else if tab === "sounds"}
      <div class="micro lib-head">built in</div>
      <div class="sfx-grid">
        {#each SFX as r (r.id)}
          <span class="sfx">
            <button type="button" class="sfx-add" data-add-sfx={r.id} disabled={locked} title={`Add ${r.name} at the playhead`} onclick={() => onaddsfx(r.id)}>{r.name}</button>
            <button type="button" class="sfx-hear" class:on={auditioning === `sfx:${r.id}`} title="Listen" aria-label={`Listen to ${r.name}`} onclick={() => onaudition(`sfx:${r.id}`)}>
              <svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>
            </button>
          </span>
        {/each}
      </div>
      {#if soundFiles.length}<div class="micro lib-head">files</div>{/if}
      {#each soundFiles as m (m.id)}
        <div class="row">
          <button type="button" class="st-tool sm round" class:on={auditioning === m.id} title={auditioning === m.id ? "Stop" : "Listen"} aria-label={`Listen to ${m.name}`} onclick={() => onaudition(m.id)}>
            {#if auditioning === m.id}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 4h8v8H4z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}
          </button>
          <span class="row-main"><span class="row-name" title={m.name}>{m.name}</span><span class="row-sub">{seconds(m.seconds)}</span></span>
          <button type="button" class="st-tool sm ol" disabled={locked} title="Add at the playhead" aria-label={`Add ${m.name}`} onclick={() => onaddsound(m.id, "sfx")}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>
          </button>
        </div>
      {/each}
      <button type="button" class="add-wide" data-action="add-sound-file" disabled={locked} title="A sound file from disk" onclick={() => pick("sound")}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>add sound file
      </button>
    {:else}
      <div class="lib-head split"><span class="micro">layers</span><span class="micro">{layers.length} of 3</span></div>
      {#each layers as layer (layer.id)}
        <div class="row tall">
          <span class="layer-tag">{layer.label}</span>
          <span class="row-main"><span class="row-name" title={layer.name}>{layer.name}</span><span class="row-sub">{layer.detail}</span></span>
          <button type="button" class="st-tool sm ol" class:on={layer.muted} aria-pressed={layer.muted} data-layer-mute={layer.label}
            title={layer.muted ? "Unmute layer" : layer.silent ? "This video has no sound" : "Mute layer"} aria-label={`${layer.muted ? "Unmute" : "Mute"} ${layer.label}`}
            disabled={layer.silent && !layer.muted} onclick={() => onlayermute(layer.id)}>
            {#if layer.muted}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 6l3 4M14 6l-3 4"></path></svg>
            {:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4"></path></svg>{/if}
          </button>
        </div>
      {/each}
      <button type="button" class="add-wide tall" data-action="add-video" disabled={locked || layersFull} title={layersFull ? "Three layers is the limit" : "Another clip from disk, shown picture in picture"} onclick={() => pick("video")}>
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10"></path></svg>add clip
      </button>
    {/if}
  </div>
  {#each FILE_INTENTS as intent (intent)}
    <input class="vh" type="file" tabindex="-1" aria-hidden="true" accept={ACCEPT[intent]} multiple={intent !== "video"} bind:this={inputs[intent]} onchange={(e) => picked(e, intent)} />
  {/each}
</aside>

<style>
  .lib { width: 236px; flex: none; display: flex; flex-direction: column; min-height: 0; border-left: 1px solid var(--border); background: var(--panel); }
  .lib-tabs { display: flex; height: 40px; flex: none; border-bottom: 1px solid var(--border); }
  .lib-tab { flex: 1; display: inline-flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; padding: 2px 0 0; margin: 0; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--muted); cursor: pointer; }
  .lib-tab svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; }
  .lib-tab span { font-family: var(--mono); font-size: 0.55rem; letter-spacing: 0.02em; text-transform: uppercase; line-height: 1; }
  .lib-tab:hover { color: var(--text); }
  .lib-tab.on { background: var(--bg-elev); border-bottom-color: var(--accent); color: var(--accent-hi); }
  .lib-body { flex: 1; min-height: 0; overflow-y: auto; padding: 10px; box-sizing: border-box; }
  .lib-body.locked { opacity: 0.6; }
  .lib-head { margin: 2px 0 6px; }
  .lib-head.split { display: flex; justify-content: space-between; }
  .lib-note { margin: 0 0 8px; font-size: 0.74rem; color: var(--muted); }
  .lib-note.warn, .row-sub.warn { color: var(--warn); }

  /* Text effects: three chips a row, each previewing its effect. */
  .fx-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 6px; margin-bottom: 12px; }
  .fx-chip { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px; height: 46px; min-width: 0; padding: 0 3px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text); cursor: pointer; overflow: hidden; }
  .fx-chip:hover:not(:disabled) { border-color: var(--accent); }
  .fx-aa { font-size: 16px; font-weight: 700; line-height: 1.1; max-width: 100%; white-space: nowrap; }
  .fx-aa.plain { font-weight: 600; }
  .fx-name { font-size: 0.6rem; color: var(--text-2); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .pic-row { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 12px; }
  .add-tile { display: inline-flex; align-items: center; justify-content: center; gap: 6px; width: 100px; height: 46px; border: 1px dashed var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); font-size: 0.76rem; cursor: pointer; }
  .add-tile svg, .add-wide svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; }
  .pic { width: 51px; height: 46px; padding: 0; border: 1px solid var(--border); border-radius: var(--r); cursor: pointer;
    background: repeating-conic-gradient(var(--bg-elev) 0 25%, var(--panel) 0 50%) 0 0 / 8px 8px; }
  .pic canvas { width: 100%; height: 100%; display: block; }
  .emoji-grid { display: grid; grid-template-columns: repeat(6, 1fr); gap: 4px; }
  .emoji { height: 30px; padding: 0; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); font-family: "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif; font-size: 17px; line-height: 1; cursor: pointer; }
  .emoji:hover:not(:disabled) { border-color: var(--accent); }

  .row { display: flex; align-items: center; gap: 8px; min-height: 38px; border-bottom: 1px solid var(--border-soft); }
  .row.tall { min-height: 44px; }
  .row.busy { opacity: 0.6; }
  .row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .row-name { font-size: 0.8rem; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .row-sub { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tick { width: 22px; height: 22px; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--accent); border-radius: var(--r); background: var(--accent-dim); color: var(--accent-hi); flex: none; }
  .tick svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .round { border-radius: 999px; }
  .layer-tag { width: 30px; height: 22px; flex: none; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--border); border-radius: 3px; font-family: var(--mono); font-size: 0.6rem; color: var(--text); background: var(--bg-elev); }

  .sfx-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 6px; margin-bottom: 12px; }
  .sfx { display: flex; height: 32px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); overflow: hidden; }
  .sfx-add { flex: 1; min-width: 0; padding: 0 8px; border: 0; background: transparent; color: var(--text); font-size: 0.76rem; text-align: left; cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sfx-add:hover:not(:disabled) { background: var(--accent-dim); }
  .sfx-hear { width: 26px; flex: none; border: 0; border-left: 1px solid var(--border-soft); background: transparent; color: var(--muted); cursor: pointer; display: inline-flex; align-items: center; justify-content: center; }
  .sfx-hear svg { width: 9px; height: 9px; }
  .sfx-hear:hover, .sfx-hear.on { color: var(--accent-hi); }

  .add-wide { display: flex; align-items: center; justify-content: center; gap: 8px; width: 100%; height: 30px; margin: 8px 0 0; border: 1px dashed var(--border); border-radius: var(--r); background: transparent; color: var(--text-2); font-size: 0.76rem; cursor: pointer; }
  .add-wide.tall { height: 44px; }
  button:disabled { opacity: 0.45; cursor: default; }
  .vh { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
