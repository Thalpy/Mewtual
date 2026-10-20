<script module lang="ts">
  import { JobQueue } from "./clip-media.ts";

  /**
   * Song renders and picture and sound decodes, one at a time across every editor (see
   * `JobQueue`). Each can hold hundreds of megabytes while it runs.
   */
  const heavyJobs = new JobQueue();

  /**
   * Where focus goes back to when the editing session ends, shared across instances. Reviews are
   * queued, and the next editor mounts while the previous one is still in the page, so the next
   * one would otherwise capture a button inside the outgoing dialog as its "previous focus" and
   * hand focus back to nothing. Each instance inherits this instead when focus is inside an editor.
   */
  let sessionReturnFocus: HTMLElement | null = null;
</script>

<script lang="ts">
  /**
   * The clip maker: a modal opened by the host for each video picked, dropped or pasted into a
   * composer, before anything is uploaded. It began as a cut-only editor and keeps every part of
   * that (trim, blade, remove and restore pieces), and adds a small composition in the spirit of a
   * cartoon maker: text in the app's text effects, stickers (pictures and emoji), up to three
   * videos, Studio music and sound effects, and motion recorded by dragging while the clip plays.
   *
   * Layout, top to bottom: header; the stage (the clip as it will look, drawn by the same
   * `drawClipFrame` the export uses) with the inspector row under it and the library beside it;
   * transport and tools; the timeline (V1 with its cuts, one lane per added video, the text and
   * sticker lane, the sound lane); the footer with the one action.
   *
   * The document is `clip-doc.ts` (pure, undoable by identity); the timeline's geometry is
   * `clip-timeline.ts`, the stage's `clip-stage.ts`, the preview's `clip-preview.ts`. This
   * component is the wiring: pointer and key handling, playback, the media bin, and the lifecycle
   * around the probe and the export.
   *
   * Outcomes go to `onfinish` exactly once (see `VideoEditorResult`). There is one action button
   * (also Ctrl+Enter); its word depends on `context` ("post now", "embed", "upload"; see
   * `editorWords`), and in the chat and announcement composers its result carries `post`:
   * - untouched (nothing cut, added or muted): `original`, the picked bytes, never re-rendered;
   * - only cut: `edited`, from the cut-only exporter, which copies kept frames without drawing them;
   * - anything added or any sound changed: `edited`, from the compositor (`exportClip`);
   * - the close button and Escape: `skip`, after asking "Discard this clip?" when there is work to
   *   lose.
   *
   * The backdrop deliberately does not dismiss: a stray click outside the card would throw away
   * every edit. While an export runs, Escape cancels the export instead of closing.
   *
   * `file` is read once at mount; the host mounts a fresh instance for every file.
   */
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { formatBytes } from "./transfer-visual.ts";
  import {
    canSplitAt, formatClock, initialEdit, isUnedited, keptDuration, keptRanges, moveCut, pieceIndexAt, pieces,
    playbackJump, pushEdit, redo, replaceEdit, rulerLabel, rulerTicks, setInPoint, setOutPoint, sourceToOutput,
    splitAt, startHistory, toggleRemoved, undo, type EditHistory, type VideoEdit,
  } from "./video-edit.ts";
  import {
    ExportCanceledError, FILMSTRIP_MAX_TILES, OutputTooLargeError, editableContainer, exportEditedVideo, filmstrip,
    probeVideo, type VideoEditorContext, type VideoEditorResult,
  } from "./video-export.ts";
  import {
    MAX_VIDEO_LAYERS, MIN_ITEM_SECONDS, addAudio, addImage, addLayer, addText, baseLayer, clearPath, clipDuration,
    editText, isUntouched, layerSpan, moveLayer, needsCompositor, newClipDoc, placeAt, placementAt, recordPath, removeAudio,
    removeLayer, removeOverlay, setOverlayTimes, transformOverlay, updateAudio, updateLayer, withBaseEdit,
    type AudioItem, type ClipDoc, type Overlay, type PathSample,
  } from "./clip-doc.ts";
  import { exportClip, type ClipMedia } from "./clip-export.ts";
  import { drawClipFrame, textFontPx, type FrameAssets, type TextSprite } from "./clip-render.ts";
  import {
    PathRecorder, layerStageBox, overlayShape, overlayStageBox, pickLayer, pickOverlay, resizeBox, scaleTurn,
    stageFrame, toFraction, type Corner,
  } from "./clip-stage.ts";
  import { layerTargets, needsResync, soundStarts } from "./clip-preview.ts";
  import { LANE, axisSpan, keyframeMarks, laneLayout, layerAxisSpan, outputDelta, outputToAxis, packRows, thinMarks } from "./clip-timeline.ts";
  import {
    BIN_FULL, MediaBin, MediaRefusal, decodeImageFile, decodeSoundFile, emojiImage, fileRefusal, fileRole, mediaName,
    readTakeFile,
  } from "./clip-media.ts";
  import { renderSfx, sfxRecipe } from "./clip-sfx.ts";
  import { CLIP_SAMPLE_RATE, TakeRenderError, renderTakeOffline } from "./jam-offline.ts";
  import { collectTextEffectCss, isAnimated, prepareText, snapshotImage, type PreparedText } from "./text-fx-raster.ts";
  import { checkSongsOffered, songKey, songOfKey, type ClipMusicSource, type ClipSong } from "./clip-music.ts";
  import { soundPieces } from "./clip-audio.ts";
  import { carriesFiles } from "./drop-guard.ts";
  import {
    NEW_TEXT, editorWords, type ClipSelection, type FileIntent, type InspectorTarget, type LibraryTab, type SongState,
    type SongsStatus,
  } from "./video-editor-ui.ts";
  import ClipLibrary from "./ClipLibrary.svelte";
  import ClipInspector from "./ClipInspector.svelte";

  /** The host/editor contract; the host is written against exactly this. */
  type Props = {
    /** The picked video. The editor makes and revokes its own object URL for the preview. */
    file: File;
    /** Largest upload the server accepts, in bytes: the export cap and the over-limit test. */
    capBytes: number;
    /** "2 of 3" when several files were picked at once; omitted for a single file. */
    position?: { index: number; count: number };
    /**
     * Where the video is going: decides the wording and whether "post now" is offered. Required,
     * so a new call site cannot forget it and quietly offer "post now" where nothing can be sent.
     */
    context: VideoEditorContext;
    /**
     * Called exactly once with the outcome; the host then unmounts the editor. An editor unmounted
     * before it answered (the host's branch went away, say) answers `skip` on its way out, so the
     * host's review can never be left waiting.
     */
    onfinish: (result: VideoEditorResult) => void;
    /**
     * Keys the modal editor must let through to the app untouched (Lock, push-to-talk). Every other
     * key stays inside the dialog.
     */
    passKey?: (e: KeyboardEvent) => boolean;
    /** The channel's Studio songs, for the Music tab; absent where there is no Studio. */
    music?: ClipMusicSource | null;
    /** Seams for tests; the real implementations by default. */
    probe?: typeof probeVideo;
    exporter?: typeof exportEditedVideo;
    composer?: typeof exportClip;
    thumbnails?: typeof filmstrip;
  };
  let {
    file, capBytes, position, context, onfinish, passKey = () => false, music = null,
    probe = probeVideo, exporter = exportEditedVideo, composer = exportClip, thumbnails = filmstrip,
  }: Props = $props();

  /** Nudge sizes for the arrow keys and the step buttons, in seconds. */
  const NUDGE = 0.1;
  const NUDGE_LARGE = 1;
  /** Filmstrip tile height in CSS pixels; also the decode height. */
  const TILE_HEIGHT = 40;
  /** Music is rendered no longer than the clip needs, plus room to slide it. */
  const MUSIC_SLACK_SECONDS = 30;
  /** How long a refusal stays in the footer. */
  const NOTICE_MS = 5000;
  const CORNERS: readonly Corner[] = ["nw", "ne", "sw", "se"];
  /** How much of a recording's trail is drawn (about ten seconds at 60 frames a second). */
  const TRAIL_SAMPLES = 600;

  // --- State ------------------------------------------------------------------------------------
  type Phase = "probing" | "unavailable" | "ready" | "exporting" | "failed";
  let phase = $state<Phase>("probing");
  /** The probe's refusal (unavailable) or the export's failure (failed), shown as an alert. */
  let message = $state("");
  /** A tooltip with the way out of a failure, when there is one. */
  let messageTip = $state("");
  let duration = $state(0);
  let size = $state({ width: 0, height: 0 });
  /** The clip and its undo/redo stacks. Raw: every change replaces it whole. */
  let history = $state.raw<EditHistory<ClipDoc> | null>(null);
  /** What the stage, timeline and inspector act on. A piece is what Delete toggles on V1. */
  let selection = $state<ClipSelection>(null);
  let bladeMode = $state(false);
  /** Where the blade guide sits while the pointer is over the timeline in blade mode. */
  let bladeAt = $state<number | null>(null);
  /** Playhead, in the clip's own source time (what V1 shows). */
  let playhead = $state(0);
  let playing = $state(false);
  let muted = $state(false);
  let progress = $state(0);
  /**
   * A cancelled export is still tearing down (its decoders and encoders close asynchronously).
   * Upload stays disabled until it has, so a quick cancel-and-retry never runs two conversions
   * side by side competing for the same hardware coders.
   */
  let draining = $state(false);
  /** The "Discard this clip?" question is showing (see `closeRequest`). */
  let confirmingDiscard = $state(false);
  let keepButton = $state<HTMLButtonElement | null>(null);
  let confirmBox = $state<HTMLDivElement | null>(null);
  /** Filmstrip tiles as they arrive; `null` until a tile is decoded. */
  let tiles = $state.raw<(CanvasImageSource | null)[]>([]);
  let libraryOpen = $state(true);
  let libraryTab = $state<LibraryTab>("text");
  /** Record mode (R): the clip plays and dragging moves the selected overlay along a new path. */
  let recording = $state(false);
  /** Overlays hidden on the stage while editing (the T lane's eye). Preview only: they still post. */
  let hideOverlays = $state(false);
  /** A short refusal ("that picture is too big"), shown in the footer for a few seconds. */
  let notice = $state("");
  /** Bumped whenever the media bin changes, so what reads it re-reads. */
  let mediaRev = $state(0);
  let songs = $state.raw<ClipSong[]>([]);
  /** Where reading the channel's songs stands. Only "idle" asks by itself (once, when the Music
   * tab first opens); a failure stays failed until the person asks again. */
  let songsStatus = $state<SongsStatus>("idle");
  let songsError = $state("");
  let songState = $state.raw<Record<string, SongState>>({});
  let musicFileIds = $state.raw<string[]>([]);
  let soundFileIds = $state.raw<string[]>([]);
  let auditioning = $state<string | null>(null);
  /** This editor's adds still pending (see `counting`); posting waits for them. */
  let adding = $state(0);
  /** Bumped by "stop adding" (`stopAdding`); adds started before it drop what they were making. */
  let addEpoch = 0;
  /** Aborted (then replaced) by "stop adding": stops the renders and decodes of the adds it ends. */
  let addsAbort = new AbortController();
  /** The stage's size in CSS px (a ResizeObserver keeps it). */
  let stageSize = $state({ w: 0, h: 0 });
  /** While a drag is being recorded: its samples so far, and where the overlay is right now. */
  let trail = $state.raw<PathSample[]>([]);
  let live = $state.raw<{ id: string; x: number; y: number } | null>(null);
  /** The pointer is over something the stage can grab. */
  let stageHover = $state<"overlay" | "layer" | null>(null);

  let root = $state<HTMLDivElement | null>(null);
  let card = $state<HTMLDivElement | null>(null);
  let videoEl = $state<HTMLVideoElement | null>(null);
  let trackEl = $state<HTMLDivElement | null>(null);
  let timelineEl = $state<HTMLDivElement | null>(null);
  let stageEl = $state<HTMLDivElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  /** One hidden element per added video, by layer id; they play under the base video's clock.
   * A removed layer's entry goes null when its element unmounts. */
  let layerEls = $state<Record<string, HTMLVideoElement | null>>({});

  // Plain bookkeeping, read only from handlers and callbacks.
  /** Set once `onfinish` has fired, or the component is gone; every late callback checks it. */
  let finished = false;
  let destroyed = false;
  let exportAbort: AbortController | null = null;
  const stripAbort = new AbortController();
  /** Aborted when the editor answers or goes: stops music renders still running for it. */
  const sessionAbort = new AbortController();
  let frameHandle: number | null = null;
  /** The timeline drag in progress, and whether it has already made its one undo step. */
  type TimelineDrag =
    | { kind: "in" | "out" | "scrub" | "cut"; cut: number; stepped: boolean }
    | { kind: "bar"; item: "overlay" | "audio" | "layer"; id: string; edge: "move" | "start" | "end"; from: number; origin: ClipDoc; stepped: boolean };
  let drag: TimelineDrag | null = null;
  /** The stage drag in progress. Every kind starts from `origin`, so each move replaces the last. */
  type StageDrag =
    | { kind: "move"; id: string; origin: ClipDoc; dx: number; dy: number; stepped: boolean }
    | { kind: "record"; id: string; recorder: PathRecorder; dx: number; dy: number; px: number; py: number }
    | { kind: "turn"; id: string; origin: ClipDoc; cx: number; cy: number; fx: number; fy: number; stepped: boolean }
    | { kind: "box"; id: string; origin: ClipDoc; dx: number; dy: number; stepped: boolean }
    | { kind: "corner"; id: string; origin: ClipDoc; corner: Corner; stepped: boolean }
    | { kind: "tap"; toggle: boolean };
  let stageDrag: StageDrag | null = null;
  /** Edits from one inspector control run (a word typed, a slider dragged) group into one step. */
  let lastGroup: string | null = null;
  let noticeTimer: ReturnType<typeof setTimeout> | null = null;
  /** The session's media. Raw state: what reads it re-reads when it is made or released; changes
   * inside it are announced by `mediaRev`. */
  let bin = $state.raw<MediaBin | null>(null);
  /** Object URLs for added videos, by media id; revoked on close. */
  const layerUrls = new Map<string, string>();
  /** Rendered built-in sounds, by recipe id; shared by auditions and the clip. */
  const sfxBuffers = new Map<string, Promise<AudioBuffer>>();
  let previewCtx: AudioContext | null = null;
  let previewGain: GainNode | null = null;
  let previewNodes: AudioBufferSourceNode[] = [];
  let auditionNode: AudioBufferSourceNode | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let drawQueued = false;
  /**
   * The element focus returns to on close. Captured at init, before this component's DOM exists.
   * When focus is inside another editor at that moment (the previous review of a queue, still
   * being torn down), the session's original return target is inherited instead.
   */
  const previousFocus = (() => {
    if (typeof document === "undefined") return null;
    const active = document.activeElement as HTMLElement | null;
    const inEditor = !!active?.closest?.("[data-video-editor]");
    const target = inEditor ? sessionReturnFocus : active;
    sessionReturnFocus = target;
    return target;
  })();
  /**
   * The preview's source, made once from the file the editor opened on (see the header: `file`
   * is read at mount). jsdom, the unit-test DOM, has no object URLs; the preview is then empty.
   */
  const previewUrl = untrack(() => objectUrl(file));
  function objectUrl(blob: Blob): string {
    return typeof URL.createObjectURL === "function" ? URL.createObjectURL(blob) : "";
  }

  // --- Derived view -----------------------------------------------------------------------------
  const doc = $derived<ClipDoc | null>(history?.present ?? null);
  const edit = $derived<VideoEdit | null>(doc ? baseLayer(doc).edit : null);
  /** The playhead in the clip as posted (what overlays, sounds and added videos are timed in). */
  const outT = $derived(edit ? sourceToOutput(edit, playhead) : 0);
  const pieceList = $derived(edit ? pieces(edit) : []);
  const kept = $derived(edit ? keptRanges(edit) : []);
  const keptSeconds = $derived(edit ? keptDuration(edit) : 0);
  const unedited = $derived(edit ? isUnedited(edit) : true);
  const untouched = $derived(doc ? isUntouched(doc) : true);
  const composed = $derived(doc ? needsCompositor(doc) : false);
  const removedCount = $derived(edit ? edit.removed.filter((r) => r).length : 0);
  /** A piece selection that undo or a merge left pointing past the end is no selection. */
  const selectedPiece = $derived(edit && selection?.kind === "piece" && selection.index < edit.removed.length ? selection.index : null);
  /** The piece Delete would toggle when nothing else is selected. */
  const targetPiece = $derived(edit ? (selectedPiece ?? pieceIndexAt(edit, playhead)) : null);
  const targetRemoved = $derived(edit !== null && targetPiece !== null && edit.removed[targetPiece]);
  /** The selected overlay, sound or layer, if it still exists (undo can take it away). */
  const selOverlay = $derived.by(() => { const s = selection; return doc && s?.kind === "overlay" ? doc.overlays.find((o) => o.id === s.id) ?? null : null; });
  const selAudio = $derived.by(() => { const s = selection; return doc && s?.kind === "audio" ? doc.audio.find((a) => a.id === s.id) ?? null : null; });
  const selLayerIndex = $derived.by(() => { const s = selection; return doc && s?.kind === "layer" ? doc.layers.findIndex((l) => l.id === s.id) : -1; });
  const ruler = $derived(rulerTicks(duration, 8));
  const editing = $derived(phase === "ready" && doc !== null);
  /** The original only goes up when it fits; the native side would refuse it anyway. */
  const originalTooBig = $derived(file.size > capBytes);
  const words = $derived(editorWords(context));
  /** Work that closing would throw away: anything cut, trimmed, removed, added or muted, including
   * things that would not change the posted clip (a muted sound, text past the end) but took work. */
  const dirty = $derived(doc !== null && (!untouched || (edit?.cuts.length ?? 0) > 0
    || doc.overlays.length > 0 || doc.audio.length > 0 || doc.layers.length > 1));
  /** Whether the clip itself has sound to mute. Muting a silent clip would only force a re-render. */
  const baseHasAudio = $derived.by(() => { void mediaRev; return bin?.video("base")?.hasAudio ?? false; });
  // Not while something is still being added: it would finish after the post began and be left
  // out of the clip (found in review; the queue can keep a job waiting for a while).
  const uploadDisabled = $derived(!editing || draining || recording || adding > 0 || kept.length === 0 || (untouched && originalTooBig));
  const uploadTitle = $derived(
    kept.length === 0 ? "Nothing is kept"
      : draining ? "Finishing the cancelled export"
      : recording ? "Stop recording first"
      : adding > 0 ? "Still adding to the clip"
      : untouched && originalTooBig ? `Over this server's ${formatBytes(capBytes)} limit: trim it to fit`
      : untouched ? `${words.Verb} the original video`
      : `${words.Verb} the edited video (re-encoded)`,
  );
  const frame = $derived(stageFrame(stageSize.w, stageSize.h, size.width, size.height));

  // Timeline lanes.
  const overlayRows = $derived(doc ? packRows(doc.overlays) : []);
  const audioRows = $derived(doc ? packRows(doc.audio.map((a) => ({ start: a.start, end: a.start + a.length }))) : []);
  const lanes = $derived(laneLayout(
    doc ? doc.layers.length - 1 : 0,
    overlayRows.length ? Math.max(...overlayRows) + 1 : 0,
    audioRows.length ? Math.max(...audioRows) + 1 : 0,
  ));
  /** Stretches of V1 that are not in the clip, dimmed across every lane below it. */
  const gaps = $derived(edit ? [
    { from: 0, to: edit.inPoint, removed: false },
    ...pieceList.filter((p) => p.removed).map((p) => ({ from: p.start, to: p.end, removed: true })),
    { from: edit.outPoint, to: duration, removed: false },
  ].filter((g) => g.to > g.from) : []);

  const inspectorTarget = $derived.by<InspectorTarget | null>(() => {
    void mediaRev;
    if (!doc) return null;
    if (selOverlay) {
      const c = selOverlay.content;
      const name = c.kind === "text" ? c.text : bin?.image(c.source)?.name ?? "sticker";
      return { kind: "overlay", overlay: selOverlay, placement: placementAt(selOverlay, outT), name };
    }
    if (selAudio) {
      const m = bin?.sound(selAudio.source);
      const origin = m?.origin === "studio" ? "studio" : m?.origin === "builtin" ? "built in" : "file";
      return { kind: "audio", item: selAudio, name: m?.name ?? "sound", origin };
    }
    if (selLayerIndex > 0) {
      const layer = doc.layers[selLayerIndex];
      return { kind: "layer", layer, label: `V${selLayerIndex + 1}`, name: bin?.video(layer.source)?.name ?? "video",
        first: selLayerIndex === 1, last: selLayerIndex === doc.layers.length - 1, hasAudio: hasSound(layer.source) };
    }
    return { kind: "base", layer: baseLayer(doc), name: mediaName(file.name), hasAudio: baseHasAudio };
  });

  // Library lists.
  const libImages = $derived.by(() => { void mediaRev; return (bin?.list("image") ?? []).filter((m) => m.origin === "file").map((m) => ({ id: m.id, name: m.name, image: m.image, width: m.width, height: m.height })); });
  const soundRow = (id: string) => { const m = bin?.sound(id); return m ? { id, name: m.name, seconds: m.buffer.duration } : null; };
  const libMusicFiles = $derived.by(() => { void mediaRev; return musicFileIds.map(soundRow).filter((r) => r !== null); });
  const libSoundFiles = $derived.by(() => { void mediaRev; return soundFileIds.map(soundRow).filter((r) => r !== null); });
  const onClip = $derived.by(() => {
    void mediaRev;
    const keys = new Set<string>();
    for (const a of doc?.audio ?? []) { const m = bin?.sound(a.source); if (m?.key) keys.add(m.key); }
    return keys;
  });
  const libLayers = $derived.by(() => {
    void mediaRev;
    return (doc?.layers ?? []).map((l, i) => ({
      id: l.id, label: `V${i + 1}`, muted: l.muted, silent: !hasSound(l.source),
      name: i === 0 ? mediaName(file.name) : bin?.video(l.source)?.name ?? "video",
      detail: i === 0 ? `base ${formatClock(keptSeconds)}` : `pip ${formatClock(keptDuration(l.edit))}`,
    }));
  });
  /** What the posted clip will hold: things placed after its end are not counted. */
  const counts = $derived.by(() => {
    if (!doc) return null;
    const total = clipDuration(doc);
    return {
      overlays: doc.overlays.filter((o) => o.start < total).length,
      audio: doc.audio.filter((a) => a.start < total).length,
      layers: 1 + doc.layers.slice(1).filter((l) => l.start < total).length,
    };
  });

  // --- Lifecycle --------------------------------------------------------------------------------
  onMount(() => {
    // The dialog takes focus so Escape, Space and the editing keys work without a click first.
    card?.focus();
    // Focus that lands anywhere outside the dialog is brought back. The keys are only kept from
    // the app behind it while focus is inside (see `onKey`); without this, a click on the title
    // bar or anything else focusable outside the overlay would hand the app's shortcuts back.
    document.addEventListener("focusin", keepFocus);
    if (typeof ResizeObserver === "function" && stageEl) {
      resizeObserver = new ResizeObserver((entries) => {
        const r = entries[0]?.contentRect;
        if (r) stageSize = { w: r.width, h: r.height };
      });
      resizeObserver.observe(stageEl);
    }
    void begin();
    return () => document.removeEventListener("focusin", keepFocus);
  });

  onDestroy(() => {
    destroyed = true;
    // Unmounted without an answer: the host's branch went away under it. Answer `skip` so the
    // host's review settles; a host that closed the editor itself ignores the late answer (it no
    // longer has this review open). Deferred, because state changes do not belong in a teardown.
    if (!finished) {
      finished = true;
      queueMicrotask(() => onfinish({ kind: "skip" }));
    }
    stripAbort.abort();
    sessionAbort.abort();
    exportAbort?.abort(); exportAbort = null;
    stopLoop();
    // Closed mid-drag (a lock, a rescope): the window's release listeners go with it.
    cancelStageDrag();
    resizeObserver?.disconnect();
    releaseMedia();
    if (videoEl) { videoEl.pause(); videoEl.removeAttribute("src"); videoEl.load(); }
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    if (noticeTimer) clearTimeout(noticeTimer);
    // Give focus back to where the person was (the composer, normally) if it still exists, but
    // only if focus is still ours to give: the next queued editor may already hold it.
    const here = document.activeElement;
    const ours = !here || here === document.body || (root?.contains(here) ?? false);
    if (ours && previousFocus && previousFocus.isConnected && typeof previousFocus.focus === "function") previousFocus.focus();
  });

  /** Stop every sound and release every decoded thing this session holds. Idempotent. */
  function releaseMedia() {
    stopSounds();
    stopAudition();
    for (const el of Object.values(layerEls)) { if (el) { el.pause(); el.removeAttribute("src"); el.load(); } }
    for (const url of layerUrls.values()) URL.revokeObjectURL(url);
    layerUrls.clear();
    void previewCtx?.close().catch(() => {});
    previewCtx = null; previewGain = null;
    sprites.clear();
    bin?.dispose();
    bin = null;
  }

  /**
   * Focus that lands outside the dialog is brought back, with two exceptions: once this editor has
   * answered it is on its way out and holds nothing, and focus moving into another editor is the
   * host's next queued review taking over. Without both, the outgoing and incoming editors would
   * pull focus back and forth while they briefly share the page.
   */
  function keepFocus(e: FocusEvent) {
    const target = e.target as Element | null;
    if (finished || destroyed || !root || !card || !target) return;
    // The discard question holds focus inside itself the way the editor holds it inside the card.
    if (confirmingDiscard && root.contains(target) && !confirmBox?.contains(target)) { keepButton?.focus(); return; }
    if (root.contains(target)) return;
    if (target.closest?.("[data-video-editor]")) return;
    card.focus();
  }
  /**
   * Focus that leaves for nowhere comes back. Chromium moves focus to <body> when the focused
   * control is disabled or removed (Split at a cut, Undo at the end of its stack, Cancel once the
   * export stops), firing only `focusout`, never `focusin`, so `keepFocus` never sees it. Keys
   * typed after that would go straight to the app's shortcuts behind the dialog.
   */
  function onFocusOut(e: FocusEvent) {
    if (e.relatedTarget) return; // going somewhere real: `keepFocus` judges that
    setTimeout(() => {
      if (finished || destroyed || !root || !card) return;
      const now = document.activeElement;
      if (confirmingDiscard) {
        if (!now || !confirmBox?.contains(now)) keepButton?.focus();
        return;
      }
      if (now && now !== document.body && (root.contains(now) || now.closest?.("[data-video-editor]"))) return;
      card.focus();
    }, 0);
  }
  /** A press on the backdrop must not move focus out of the dialog (to the page body). */
  function onBackdropDown(e: MouseEvent) {
    if (e.target === e.currentTarget) { e.preventDefault(); card?.focus(); }
  }

  /** Probe the file, then open the editor on it or explain why it cannot be edited. */
  async function begin() {
    let result: Awaited<ReturnType<typeof probeVideo>>;
    try { result = await probe(file); }
    catch { result = { ok: false, reason: "This file could not be read as a video." }; }
    if (finished) return;
    if (!result.ok) { message = result.reason; phase = "unavailable"; return; }
    duration = result.duration;
    size = { width: result.width, height: result.height };
    bin = new MediaBin({ name: mediaName(file.name), file, duration: result.duration, width: result.width, height: result.height, hasAudio: result.hasAudio });
    history = startHistory(newClipDoc("base", result.duration));
    phase = "ready";
    // The track only exists once "ready" renders; measure it for the tile count after that.
    await tick();
    if (!finished) void loadStrip();
  }

  /** Fill the filmstrip: about one tile per tile-width of track, evenly spaced, centre-sampled. */
  async function loadStrip() {
    const aspect = size.width > 0 && size.height > 0 ? size.width / size.height : 16 / 9;
    const trackWidth = trackEl?.clientWidth || 640;
    const count = Math.max(1, Math.min(FILMSTRIP_MAX_TILES, Math.ceil(trackWidth / (TILE_HEIGHT * aspect))));
    const times = Array.from({ length: count }, (_, i) => ((i + 0.5) * duration) / count);
    tiles = new Array(count).fill(null);
    await thumbnails(file, times, TILE_HEIGHT * (globalThis.devicePixelRatio || 1), (index, image) => {
      if (finished || index < 0 || index >= tiles.length) return;
      const next = [...tiles];
      next[index] = image;
      tiles = next;
    }, stripAbort.signal);
  }

  /** Hand the outcome to the host once; stop everything this editor started. */
  function finish(result: VideoEditorResult) {
    if (finished) return;
    finished = true;
    exportAbort?.abort(); exportAbort = null;
    stripAbort.abort();
    sessionAbort.abort();
    pause();
    stopSounds();
    stopAudition();
    onfinish(result);
  }

  /** Show a short refusal in the footer. */
  function notify(text: string) {
    notice = text;
    if (noticeTimer) clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => { notice = ""; noticeTimer = null; }, NOTICE_MS);
  }
  /** A thrown error as a sentence for the person, or `fallback` when it is not one. */
  function sentence(error: unknown, fallback: string): string {
    if (error instanceof DOMException) return fallback;
    const text = error instanceof Error ? error.message.trim() : "";
    return text ? text.charAt(0).toUpperCase() + text.slice(1) : fallback;
  }

  // --- Editing ----------------------------------------------------------------------------------
  /**
   * Record a discrete edit as one undo step. A refused operation returns the same document: no
   * step. `group` names a run of edits from one control (typing a word, dragging a slider): every
   * edit in the run after the first replaces the last, so the run is one step. Any other edit,
   * undo or redo ends the run (and any drag in progress).
   */
  function apply(next: ClipDoc, group: string | null = null, keepStageDrag = false): boolean {
    if (!history || !editing) return false;
    drag = null;
    // An addition (a file that finished decoding, say) may land mid-recording: the recording goes
    // on, since it commits onto whatever the document is when it ends. Only a recording: every
    // other stage drag replaces the document from the one it began with, which would undo the
    // addition, so any other drag (and any other edit) ends here.
    if (!keepStageDrag || stageDrag?.kind !== "record") cancelStageDrag();
    const before = history.present;
    if (next === before) return false;
    history = group !== null && group === lastGroup ? replaceEdit(history, next) : pushEdit(history, next);
    lastGroup = group;
    // A split or reset renumbers the pieces after it; a selection would silently move to another.
    if (selection?.kind === "piece" && baseLayer(next).edit.removed.length !== baseLayer(before).edit.removed.length) selection = null;
    return true;
  }
  function settle() { lastGroup = null; }
  function applyBase(next: VideoEdit) { if (doc) apply(withBaseEdit(doc, next)); }
  function splitHere() { if (edit) applyBase(splitAt(edit, playhead)); }
  function toggleTarget() { if (edit && targetPiece !== null) applyBase(toggleRemoved(edit, targetPiece)); }
  function setInHere() { if (edit) applyBase(setInPoint(edit, playhead)); }
  function setOutHere() { if (edit) applyBase(setOutPoint(edit, playhead)); }
  /** Start over: the untouched clip. Media brought in stays in the library. The id counter keeps
   * counting, so no id in undo history is ever given to something else. */
  function reset() { if (duration > 0 && doc) apply({ ...newClipDoc("base", duration), seq: doc.seq }); }
  // Not while recording: the motion being recorded is the step in progress (the buttons are
  // disabled then too; this covers Ctrl+Z / Ctrl+Y).
  function undoStep() { if (history && editing && !recording) { drag = null; cancelStageDrag(); lastGroup = null; history = undo(history); selection = null; } }
  function redoStep() { if (history && editing && !recording) { drag = null; cancelStageDrag(); lastGroup = null; history = redo(history); selection = null; } }

  /** Delete the selected overlay, sound or added layer. */
  function deleteSelected(): boolean {
    if (!doc || !selection) return false;
    const next = selection.kind === "overlay" ? removeOverlay(doc, selection.id)
      : selection.kind === "audio" ? removeAudio(doc, selection.id)
      : selection.kind === "layer" ? removeLayer(doc, selection.id)
      : doc;
    if (next === doc) return false;
    apply(next);
    selection = null;
    return true;
  }

  /** Apply an addition and select what it added (the newest item of that kind). */
  function addAndSelect(next: ClipDoc, kind: "overlay" | "audio" | "layer"): boolean {
    const recordingDrag = stageDrag?.kind === "record";
    if (!apply(next, null, true)) return false;
    const list = kind === "overlay" ? next.overlays : kind === "audio" ? next.audio : next.layers;
    const added = list[list.length - 1];
    // Mid-recording the selection stays on what is being animated.
    if (added && !recordingDrag) selection = { kind, id: added.id };
    return true;
  }

  // --- Library: bringing things in --------------------------------------------------------------
  async function addTextFx(effect: string | null) {
    if (!doc || !editing) return;
    if (!addAndSelect(addText(doc, outT, NEW_TEXT, effect), "overlay")) { notify("No more text fits on this clip."); return; }
    // The new words are selected in the inspector, so typing replaces them.
    await tick();
    const input = card?.querySelector<HTMLInputElement>('[data-field="text"]');
    input?.focus();
    input?.select();
  }

  async function addEmoji(glyph: string, name: string) {
    if (!bin || !editing) return;
    let m = bin.byKey(`emoji:${glyph}`);
    if (!m) {
      const gone = addGone();
      try {
        const pic = await counting(emojiImage(glyph));
        // A drawn emoji not kept is released at once (found in review: both refusals leaked it).
        if (!bin || gone()) { pic.image.close(); return; }
        const id = bin.add({ kind: "image", name, image: pic.image, width: pic.width, height: pic.height, origin: "emoji", key: `emoji:${glyph}` });
        if (!id) { pic.image.close(); notify(BIN_FULL); return; }
        m = bin.get(id); mediaRev++;
      } catch { notify("Emoji can't be drawn in this window."); return; }
    }
    if (doc && m && !addAndSelect(addImage(doc, outT, m.id, 1), "overlay")) notify("No more stickers fit on this clip.");
  }

  function addPicture(id: string) {
    const m = bin?.image(id);
    if (doc && m && !addAndSelect(addImage(doc, outT, m.id, m.width / m.height), "overlay")) notify("No more stickers fit on this clip.");
  }

  function sfxBuffer(id: string): Promise<AudioBuffer> {
    let p = sfxBuffers.get(id);
    if (!p) {
      const recipe = sfxRecipe(id);
      if (!recipe) return Promise.reject(new Error("unknown sound"));
      p = renderSfx(recipe, CLIP_SAMPLE_RATE);
      p.catch(() => sfxBuffers.delete(id));
      sfxBuffers.set(id, p);
    }
    return p;
  }

  async function addSfx(id: string) {
    if (!bin || !editing) return;
    const key = `sfx:${id}`;
    let m = bin.byKey(key);
    if (!m) {
      const gone = addGone();
      try {
        const buffer = await counting(sfxBuffer(id));
        if (!bin || gone()) return;
        const added = bin.add({ kind: "sound", name: sfxRecipe(id)?.name ?? id, buffer, origin: "builtin", key });
        if (!added) { notify(BIN_FULL); return; }
        m = bin.get(added); mediaRev++;
      } catch { notify("Sounds can't be made in this window."); return; }
    }
    if (doc && m?.kind === "sound" && !addAndSelect(addAudio(doc, "sfx", m.id, m.buffer.duration, outT), "audio")) notify("No more sounds fit on this clip.");
  }

  function addSound(id: string, kind: "music" | "sfx") {
    const m = bin?.sound(id);
    if (doc && m && !addAndSelect(addAudio(doc, kind, m.id, m.buffer.duration, outT), "audio")) notify("No more sounds fit on this clip.");
  }

  /** How much music to render: as long as the clip, with room to slide it along. */
  function musicCapMs(): number {
    return Math.min(600, (doc ? clipDuration(doc) : duration) + MUSIC_SLACK_SECONDS) * 1000;
  }

  /**
   * Read the channel's songs: on the Music tab's first opening, and again only when asked (the
   * retry). A failure must not look like "not asked yet", or the tab would ask again forever.
   */
  async function listSongs() {
    if (!music || songsStatus === "loading" || songsStatus === "done") return;
    songsStatus = "loading";
    songsError = "";
    try {
      const list = await music.list();
      if (destroyed) return;
      songs = list;
      songsStatus = "done";
    } catch (e) {
      if (destroyed) return;
      songs = [];
      songsError = sentence(e, "This channel's songs could not be read.");
      songsStatus = "error";
    }
  }

  /**
   * Taken when an add starts: whether it has since been given up, because the editor answered or
   * the person pressed "stop adding". A given-up add drops whatever it was making.
   */
  function addGone(): () => boolean {
    const epoch = addEpoch;
    return () => finished || epoch !== addEpoch;
  }

  /** The admission check every heavy job makes: room in the bin, for an add not given up. */
  function admitFor(gone: () => boolean): (bytes: number) => boolean {
    return (bytes) => !gone() && !!bin?.room(bytes);
  }

  /** Counts `work` in `adding` until it settles (or is given up), so posting waits for it. */
  async function counting<T>(work: Promise<T>): Promise<T> {
    const epoch = addEpoch;
    adding++;
    try { return await work; } finally { if (epoch === addEpoch) adding--; }
  }

  /**
   * Run a heavy job in its turn (`heavyJobs`), counted in `adding`. The job's `signal` aborts when
   * the editor answers or the person stops adding.
   *
   * What lets go of the queue is narrower. The editor answering always does (see `JobQueue`).
   * "Stop adding" does only until the job calls `rendering()` just before an offline render: a
   * read or a decode can hang, but a render cannot be interrupted and always ends on its own,
   * holding its whole output until then. Letting go during one would let the next render run
   * beside it, and stopping and re-adding could stack them (found in re-review). Posting does not
   * wait for it either way: "stop adding" uncounts it at once.
   */
  function heavy<T>(job: (signal: AbortSignal, rendering: () => void) => Promise<T>): Promise<T> {
    const stop = addsAbort.signal;
    const signal = AbortSignal.any([sessionAbort.signal, stop]);
    let inRender = false;
    const beforeRender = new AbortController();
    if (stop.aborted) beforeRender.abort();
    else stop.addEventListener("abort", () => { if (!inRender) beforeRender.abort(); }, { once: true });
    const release = AbortSignal.any([sessionAbort.signal, beforeRender.signal]);
    return counting(heavyJobs.run(() => job(signal, () => { inRender = true; }), release));
  }

  /**
   * "Stop adding": give up on every add still pending. Posting is possible again at once, and each
   * of those adds drops its result if it ever arrives. Without this, one decode that never ended
   * disabled posting for good, and the only way out lost the clip (found in re-review).
   */
  function stopAdding() {
    addEpoch++;
    adding = 0;
    addsAbort.abort();
    addsAbort = new AbortController();
    // Song rows still showing "rendering" go back to plain rows.
    songState = Object.fromEntries(Object.entries(songState).filter(([, s]) => s.state !== "rendering"));
  }

  async function addSong(song: ClipSong) {
    const source = music;
    if (!bin || !source || !editing) return;
    const key = songKey(song.id);
    let m = bin.byKey(key);
    if (!m) {
      const gone = addGone();
      const fail = (message: string) => { if (!gone()) songState = { ...songState, [song.id]: { state: "error", message } }; };
      // Refused before anything is read or rendered when nothing more can be kept.
      if (bin.full) { fail(BIN_FULL); return; }
      songState = { ...songState, [song.id]: { state: "rendering" } };
      try {
        // The read and the render wait their turn together: several rows clicked in a row render
        // one after another, each admitted against what the ones before it left room for.
        const buffer = await heavy(async (signal, rendering) => {
          // Closed, rescoped or locked (or given up) while queued or reading: other members' song
          // content is not read or rendered for an editor that has already answered.
          if (gone()) return null;
          const playback = await source.load(song.id);
          if (gone()) return null;
          rendering();
          return renderTakeOffline(playback.take, { gridMs: playback.gridMs, maxMs: musicCapMs(), signal, admit: admitFor(gone) });
        });
        if (!buffer || gone() || !bin) return;
        const id = bin.add({ kind: "sound", name: song.title, buffer, origin: "studio", key });
        if (!id) { fail(BIN_FULL); return; }
        m = bin.get(id); mediaRev++;
        songState = { ...songState, [song.id]: { state: "ready", seconds: buffer.duration } };
      } catch (e) {
        fail(sentence(e, "This song could not be played."));
        return;
      }
    }
    if (doc && m?.kind === "sound" && !addAndSelect(addAudio(doc, "music", m.id, m.buffer.duration, outT), "audio")) notify("No more sounds fit on this clip.");
  }

  /** Files picked in the library or dropped on the editor. `intent` is the tab they came from. */
  async function addFiles(files: File[], intent: FileIntent | null) {
    // "Stop adding" ends the rest of these files too.
    const gone = addGone();
    for (const f of files) {
      if (!bin || gone() || !editing) return;
      const role = fileRole(f.name, f.type);
      if (!role) { notify("That kind of file can't go on a clip."); continue; }
      const refusal = fileRefusal(role, f.size);
      if (refusal) { notify(refusal); continue; }
      if (bin.full) { notify(BIN_FULL); continue; }
      try {
        if (role === "image") {
          // Decodes and renders wait their turn in `heavyJobs`, and each is admitted against the
          // bin before it allocates. A job reached after its add was given up does nothing.
          const pic = await heavy(async () => (gone() ? null : decodeImageFile(f, admitFor(gone))));
          if (!pic) return;
          if (!bin || gone()) { pic.image.close(); return; }
          const id = bin.add({ kind: "image", name: mediaName(f.name), image: pic.image, width: pic.width, height: pic.height, origin: "file" });
          if (!id) { pic.image.close(); notify(BIN_FULL); continue; }
          mediaRev++;
          addPicture(id);
        } else if (role === "video") {
          if (!doc || doc.layers.length >= MAX_VIDEO_LAYERS) { notify("Three videos is the limit."); continue; }
          const probed = await counting(probe(f));
          if (!bin || gone()) return;
          if (!probed.ok) { notify(probed.reason); continue; }
          const id = bin.add({ kind: "video", name: mediaName(f.name), file: f, duration: probed.duration, width: probed.width, height: probed.height, hasAudio: probed.hasAudio });
          if (!id) { notify(BIN_FULL); continue; }
          const url = objectUrl(f);
          if (url) layerUrls.set(id, url);
          mediaRev++;
          if (doc && !addAndSelect(addLayer(doc, id, probed.duration, outT), "layer")) notify("Three videos is the limit.");
        } else if (role === "audio" || role === "take") {
          // Decoded only as far as the clip can use (and within the sample cap), whatever the
          // file's length: see `decodeSoundFile`.
          const buffer = await heavy(async (signal, rendering) => {
            if (gone()) return null;
            if (role === "audio") return decodeSoundFile(f, musicCapMs() / 1000, { admit: admitFor(gone), signal });
            const take = await readTakeFile(f);
            if (gone()) return null;
            rendering();
            return renderTakeOffline(take, { maxMs: musicCapMs(), signal, admit: admitFor(gone) });
          });
          if (!buffer || gone() || !bin) return;
          const asMusic = role === "take" || intent === "music" || (intent !== "sound" && buffer.duration > 8);
          const id = bin.add({ kind: "sound", name: mediaName(f.name), buffer, origin: "file" });
          if (!id) { notify(BIN_FULL); continue; }
          if (asMusic) musicFileIds = [...musicFileIds, id]; else soundFileIds = [...soundFileIds, id];
          mediaRev++;
          addSound(id, asMusic ? "music" : "sfx");
        }
      } catch (e) {
        if (gone()) return;
        notify(e instanceof MediaRefusal || e instanceof TakeRenderError ? e.message
          : role === "image" ? "That picture could not be read." : role === "video" ? "That video could not be read."
          : role === "take" ? "That music file could not be played." : "That sound file could not be read.");
      }
    }
  }

  /** Whether a dropped file may go onto the clip now: not while "Discard this clip?" is asked,
   * which would change the clip behind the question (found in review). */
  const takesDrops = $derived(editing && !confirmingDiscard);
  function onDragOver(e: DragEvent) {
    if (!carriesFiles(e)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = takesDrops ? "copy" : "none";
  }
  function onDrop(e: DragEvent) {
    if (!carriesFiles(e)) return;
    // The editor is modal: a file dropped on it is for the clip, never for the page behind, even
    // when the clip is not taking files right now.
    e.preventDefault();
    const files = [...(e.dataTransfer?.files ?? [])];
    if (takesDrops && files.length) void addFiles(files, null);
  }

  // --- Inspector ----------------------------------------------------------------------------------
  function inspectText(patch: { text?: string; effect?: string | null; color?: string }) {
    if (doc && selOverlay) apply(editText(doc, selOverlay.id, patch), `text:${selOverlay.id}`);
  }
  function inspectSize(scale: number) {
    if (!doc || !selOverlay) return;
    const now = placementAt(selOverlay, outT).scale;
    if (now > 0) apply(transformOverlay(doc, selOverlay.id, scale / now, 0), `size:${selOverlay.id}`);
  }
  function inspectTurn(degrees: number) {
    if (!doc || !selOverlay) return;
    apply(transformOverlay(doc, selOverlay.id, 1, degrees - placementAt(selOverlay, outT).rotation), `turn:${selOverlay.id}`);
  }
  function inspectAudio(patch: { volume?: number; muted?: boolean }) {
    if (doc && selAudio) apply(updateAudio(doc, selAudio.id, patch), `audio:${selAudio.id}`);
  }
  function inspectLayer(patch: { volume?: number; muted?: boolean; size?: number }) {
    if (!doc) return;
    const layer = selLayerIndex > 0 ? doc.layers[selLayerIndex] : baseLayer(doc);
    const box = patch.size !== undefined && selLayerIndex > 0
      ? { ...layer.box, w: patch.size, h: patch.size * (layer.box.h / layer.box.w) }
      : undefined;
    apply(updateLayer(doc, layer.id, { volume: patch.volume, muted: patch.muted, ...(box ? { box } : {}) }), `layer:${layer.id}`);
  }
  /** Whether a video in the bin has a sound track (unknown counts as none). */
  function hasSound(source: string): boolean {
    return bin?.video(source)?.hasAudio ?? false;
  }
  /**
   * Mute or unmute a video layer. A layer with no sound is never muted: the mute would change
   * nothing anyone hears, but would still count as an edit and send an untouched clip through a
   * re-render instead of as its original bytes (found in review, via the library's layer list).
   * Unmuting is always allowed, so a mute that somehow exists can be taken back.
   */
  function toggleLayerMute(id: string) {
    const layer = doc?.layers.find((l) => l.id === id);
    if (!doc || !layer || (!layer.muted && !hasSound(layer.source))) return;
    apply(updateLayer(doc, id, { muted: !layer.muted }));
  }
  /** The A lane's mute: everything added mutes together, or unmutes together if all were muted. */
  function toggleAudioLane() {
    if (!doc || !doc.audio.length) return;
    const mute = doc.audio.some((a) => !a.muted);
    let next = doc;
    for (const a of doc.audio) next = updateAudio(next, a.id, { muted: mute });
    apply(next);
  }

  // --- Preview playback -------------------------------------------------------------------------
  /**
   * Keep the playhead on the video and the video on the edit. Runs every animation frame while
   * playing and on every `timeupdate`: animation frames stop in a hidden window, and without the
   * second source a minimised editor would play straight through removed pieces.
   */
  function follow() {
    const video = videoEl;
    if (!video) return;
    const t = video.currentTime;
    playhead = t;
    if (edit && !video.paused) {
      const jump = playbackJump(edit, t);
      if (jump && "seek" in jump) { video.currentTime = jump.seek; playhead = jump.seek; }
      else if (jump) {
        // Nothing kept after here: stop at the end of the last kept range.
        video.pause();
        const end = kept.length ? kept[kept.length - 1].end : t;
        video.currentTime = end; playhead = end;
      }
    }
    syncLayers();
    if (stageDrag?.kind === "record") sampleRecording(stageDrag);
  }
  function loop() {
    frameHandle = null;
    follow();
    if (playing) frameHandle = requestAnimationFrame(loop);
  }
  function stopLoop() {
    if (frameHandle !== null) { cancelAnimationFrame(frameHandle); frameHandle = null; }
  }
  function play() {
    const video = videoEl;
    if (!video || phase === "exporting") return;
    if (edit) {
      // From the end (or from trimmed/removed time) start at the next kept frame; from past the
      // last kept frame, play the upload again from its beginning.
      const jump = playbackJump(edit, video.currentTime);
      if (jump && "end" in jump && kept.length) video.currentTime = kept[0].start;
      else if (jump && "seek" in jump) video.currentTime = jump.seek;
    }
    video.play().catch(() => { playing = false; });
  }
  function pause() { videoEl?.pause(); }
  function togglePlay() { if (playing) pause(); else play(); }
  function seek(t: number) {
    const clamped = Math.min(Math.max(0, t), duration || 0);
    if (videoEl) videoEl.currentTime = clamped;
    playhead = clamped;
    syncLayers();
  }
  function nudge(delta: number) { pause(); seek(playhead + delta); }
  function onVideoPlay() { playing = true; stopLoop(); frameHandle = requestAnimationFrame(loop); startSounds(); }
  function onVideoPause() {
    playing = false; stopLoop(); stopSounds();
    if (videoEl) playhead = videoEl.currentTime;
    syncLayers();
    // The clip stopped: a recording ends with it.
    if (stageDrag?.kind === "record") commitRecording(stageDrag);
    recording = false;
  }

  /** Put each added video where the clip's output time says it is, and play or pause it with the base. */
  function syncLayers() {
    if (!doc) return;
    const t = edit ? sourceToOutput(edit, videoEl?.currentTime ?? playhead) : 0;
    const targets = layerTargets(doc, t);
    for (const layer of doc.layers.slice(1)) {
      const el = layerEls[layer.id];
      if (!el) continue;
      el.muted = muted || layer.muted;
      el.volume = layer.volume;
      const target = targets.get(layer.id);
      if (target === null || target === undefined) { if (!el.paused) el.pause(); continue; }
      if (needsResync(el.currentTime, target, playing)) el.currentTime = target;
      if (playing && el.paused) el.play().catch(() => {});
      else if (!playing && !el.paused) el.pause();
    }
  }

  /** The preview's AudioContext, made on first use (a play or a listen is the gesture it needs). */
  function audioContext(): AudioContext | null {
    if (!previewCtx && typeof AudioContext === "function") {
      previewCtx = new AudioContext();
      previewGain = previewCtx.createGain();
      previewGain.gain.value = muted ? 0 : 1;
      previewGain.connect(previewCtx.destination);
    }
    return previewCtx;
  }
  function stopSounds() {
    for (const node of previewNodes) { try { node.stop(); } catch { /* never started */ } node.disconnect(); }
    previewNodes = [];
  }
  /** Start the music and sound effects from where playback is. */
  function startSounds() {
    stopSounds();
    if (!doc || !playing || !bin) return;
    const starts = soundStarts(doc, outT);
    if (!starts.length) return;
    const ctx = audioContext();
    if (!ctx || !previewGain) return;
    void ctx.resume().catch(() => {});
    const now = ctx.currentTime + 0.03;
    for (const s of starts) {
      const m = bin.sound(s.source);
      if (!m) continue;
      const node = ctx.createBufferSource();
      node.buffer = m.buffer;
      const gain = ctx.createGain();
      gain.gain.value = s.gain;
      node.connect(gain).connect(previewGain);
      node.start(now + s.delay, s.offset, s.duration);
      previewNodes.push(node);
    }
  }
  function stopAudition() {
    if (auditionNode) { try { auditionNode.stop(); } catch { /* never started */ } auditionNode.disconnect(); }
    auditionNode = null;
    auditioning = null;
  }
  /** Listen to a sound in the library once (a built-in by `sfx:id`, or a media id). Again stops it. */
  async function audition(key: string) {
    if (auditioning === key) { stopAudition(); return; }
    stopAudition();
    if (finished) return;
    const ctx = audioContext();
    if (!ctx || !previewGain) return;
    let buffer: AudioBuffer | null = null;
    try { buffer = key.startsWith("sfx:") ? await sfxBuffer(key.slice(4)) : bin?.sound(key)?.buffer ?? null; }
    catch { notify("Sounds can't be made in this window."); return; }
    // The editor may have answered while the sound was being made: nothing plays after that.
    if (!buffer || finished || destroyed) return;
    void ctx.resume().catch(() => {});
    const node = ctx.createBufferSource();
    node.buffer = buffer;
    node.connect(previewGain);
    node.onended = () => { if (auditionNode === node) { auditionNode = null; auditioning = null; } };
    node.start();
    auditionNode = node;
    auditioning = key;
  }
  // The preview's sound follows the mute button and the clip's own volumes.
  $effect(() => { if (previewGain) previewGain.gain.value = muted ? 0 : 1; });
  $effect(() => {
    const el = videoEl, base = doc ? baseLayer(doc) : null;
    if (el && base) { el.muted = muted || base.muted; el.volume = base.volume; }
  });
  // Sounds restart when what is on the A lane changes mid-play.
  $effect(() => {
    void doc?.audio;
    untrack(() => { if (playing) startSounds(); });
  });

  // --- Stage: drawing -----------------------------------------------------------------------------
  /** Text overlays as pictures, per overlay: prepared once per look and size, snapshotted per moment. */
  type Sprite = { key: string; prepared: PreparedText; fontPx: number; animated: boolean; image: HTMLImageElement | null; at: number; want: number | null };
  const sprites = new Map<string, Sprite>();
  let effectCss: string | null = null;

  function spriteFor(overlay: Overlay, local: number | null, pixelHeight: number): TextSprite | null {
    if (overlay.content.kind !== "text") return null;
    const fontPx = textFontPx(overlay, pixelHeight);
    const key = `${overlay.content.text}\u0000${overlay.content.effect ?? ""}\u0000${overlay.content.color}\u0000${fontPx}`;
    let entry = sprites.get(overlay.id);
    if (!entry || entry.key !== key) {
      if (local === null) return null;
      let prepared: PreparedText;
      try {
        effectCss ??= collectTextEffectCss(document.styleSheets);
        prepared = prepareText(document, effectCss, { text: overlay.content.text, effect: overlay.content.effect, color: overlay.content.color, fontPx });
      } catch { return null; }
      entry = { key, prepared, fontPx, animated: isAnimated(prepared), image: entry?.image ?? null, at: -1, want: null };
      sprites.set(overlay.id, entry);
    }
    const at = entry.animated ? Math.max(0, local ?? entry.at) : 0;
    // One snapshot in flight per overlay; a newer moment waits for it. When it lands for the
    // moment still wanted, the stage redraws once (no loop while paused).
    if (local !== null && entry.want === null && at !== entry.at) {
      const mine = entry;
      mine.want = at;
      snapshotImage(mine.prepared, at).then((image) => {
        if (sprites.get(overlay.id) !== mine) return;
        mine.image = image; mine.at = at;
        requestDraw();
      }).catch(() => {}).finally(() => { mine.want = null; });
    }
    return entry.image ? { image: entry.image, width: entry.prepared.width, height: entry.prepared.height, pad: entry.prepared.pad, fontPx: entry.fontPx } : null;
  }

  function stageAssets(pixelHeight: number): FrameAssets {
    return {
      layer: (id) => {
        const el = doc && id === doc.layers[0].id ? videoEl : layerEls[id];
        if (!el || el.readyState < 2 || !el.videoWidth) return null;
        return { image: el, width: el.videoWidth, height: el.videoHeight };
      },
      image: (source) => { const m = bin?.image(source); return m ? { image: m.image, width: m.width, height: m.height } : null; },
      text: (overlay, local) => spriteFor(overlay, local, pixelHeight),
    };
  }

  /** The document as the stage shows it: a live recording drags its overlay; the eye hides overlays. */
  function shownDoc(d: ClipDoc): ClipDoc {
    let shown = d;
    if (live) shown = placeAt(shown, live.id, outT, { x: live.x, y: live.y });
    if (hideOverlays) shown = { ...shown, overlays: [] };
    return shown;
  }

  function requestDraw() {
    if (drawQueued || destroyed) return;
    drawQueued = true;
    requestAnimationFrame(() => { drawQueued = false; draw(); });
  }
  function draw() {
    const canvas = canvasEl, d = doc, f = frame;
    if (!canvas || !d || f.w < 2 || f.h < 2) return;
    const dpr = globalThis.devicePixelRatio || 1;
    const w = Math.round(f.w * dpr), h = Math.round(f.h * dpr);
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    drawClipFrame(ctx, w, h, shownDoc(d), outT, stageAssets(h));
    // Pictures of text that is no longer on the clip are let go (an undo brings them back by
    // preparing again, which takes a few milliseconds).
    if (sprites.size > d.overlays.length) for (const id of sprites.keys()) if (!d.overlays.some((o) => o.id === id)) sprites.delete(id);
  }
  $effect(() => { void doc; void outT; void frame; void mediaRev; void hideOverlays; void live; requestDraw(); });

  /** An overlay's shape for picking and handles, from its latest picture when there is one. */
  function shapeOf(o: Overlay) {
    return overlayShape(o, spriteFor(o, null, Math.round(frame.h * (globalThis.devicePixelRatio || 1))));
  }
  const selBox = $derived.by(() => {
    void mediaRev; void live;
    if (!selOverlay || !(outT >= selOverlay.start && outT < selOverlay.end) || hideOverlays) return null;
    const o = live?.id === selOverlay.id && doc ? shownDoc(doc).overlays.find((x) => x.id === selOverlay.id) ?? selOverlay : selOverlay;
    return overlayStageBox(o, outT, frame, shapeOf(o));
  });
  const selLayerBox = $derived.by(() => {
    if (!doc || selLayerIndex <= 0) return null;
    const layer = doc.layers[selLayerIndex];
    const span = layerSpan(doc, layer);
    return outT >= span.start && outT < span.end ? layerStageBox(layer.box, frame) : null;
  });
  /** The selected overlay's path, as stage points, when it has one. */
  const pathPoints = $derived.by(() => {
    if (!selOverlay || selOverlay.keyframes.length < 2 || hideOverlays) return "";
    return selOverlay.keyframes.map((k) => `${(frame.x + k.x * frame.w).toFixed(1)},${(frame.y + k.y * frame.h).toFixed(1)}`).join(" ");
  });
  const trailPoints = $derived(trail.map((s) => `${(frame.x + s.x * frame.w).toFixed(1)},${(frame.y + s.y * frame.h).toFixed(1)}`).join(" "));

  // --- Stage: pointer ---------------------------------------------------------------------------
  function stagePoint(e: PointerEvent): { px: number; py: number } {
    const r = stageEl?.getBoundingClientRect();
    return { px: e.clientX - (r?.left ?? 0), py: e.clientY - (r?.top ?? 0) };
  }
  function onStageDown(e: PointerEvent) {
    if (!editing || !doc || e.button !== 0) return;
    const { px, py } = stagePoint(e);
    const p = toFraction(frame, px, py);
    const handle = (e.target as Element | null)?.closest?.("[data-stage-handle]")?.getAttribute("data-stage-handle");
    e.preventDefault();
    card?.focus();
    if (handle === "turn" && selOverlay && selBox) {
      stageDrag = { kind: "turn", id: selOverlay.id, origin: doc, cx: selBox.cx, cy: selBox.cy, fx: px, fy: py, stepped: false };
    } else if (handle && selLayerIndex > 0) {
      stageDrag = { kind: "corner", id: doc.layers[selLayerIndex].id, origin: doc, corner: handle as Corner, stepped: false };
    } else {
      const overlayId = hideOverlays ? null : pickOverlay(doc, outT, frame, px, py, shapeOf);
      // In record mode the selected overlay follows the pointer from wherever the drag starts:
      // grabbed off itself, it jumps to the pointer; grabbed on itself, it keeps the grab offset.
      // Armed even before playback has visibly started (R starts it, and the first frames take a
      // moment): the recorder only keeps samples once the clip's time moves.
      const armedId = recording && selOverlay ? selOverlay.id : null;
      const id = armedId ?? overlayId;
      const o = id ? doc.overlays.find((x) => x.id === id) : undefined;
      if (id && o) {
        selection = { kind: "overlay", id };
        const at = placementAt(o, outT);
        const jump = armedId !== null && overlayId !== armedId;
        const dx = jump ? 0 : p.x - at.x, dy = jump ? 0 : p.y - at.y;
        if (playing || recording) {
          const recorder = new PathRecorder();
          stageDrag = { kind: "record", id, recorder, dx, dy, px, py };
          sampleRecording(stageDrag);
        } else {
          stageDrag = { kind: "move", id, origin: doc, dx, dy, stepped: false };
        }
      } else {
        const layerId = pickLayer(doc, outT, frame, px, py);
        if (layerId) {
          const layer = doc.layers.find((l) => l.id === layerId)!;
          selection = { kind: "layer", id: layerId };
          stageDrag = { kind: "box", id: layerId, origin: doc, dx: p.x - layer.box.x, dy: p.y - layer.box.y, stepped: false };
        } else {
          // An empty spot: deselect; a tap with nothing selected plays or pauses, as before.
          const toggle = selection === null || selection.kind === "piece";
          if (selection && selection.kind !== "piece") selection = null;
          stageDrag = { kind: "tap", toggle };
        }
      }
    }
    watchRelease();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }
  function onStageMove(e: PointerEvent) {
    const { px, py } = stagePoint(e);
    if (!stageDrag) {
      if (editing && doc) stageHover = (!hideOverlays && pickOverlay(doc, outT, frame, px, py, shapeOf)) ? "overlay" : pickLayer(doc, outT, frame, px, py) ? "layer" : null;
      return;
    }
    if (!history || !editing) { cancelStageDrag(); return; }
    // A drag whose button is no longer down lost its release; end it here.
    if ((e.buttons & 1) === 0) { endStageDrag(); return; }
    const p = toFraction(frame, px, py);
    const d = stageDrag;
    if (d.kind === "tap") return;
    if (d.kind === "record") {
      // The overlay follows the pointer at once; the PATH is sampled once per frame (in `follow`),
      // so samples are evenly spaced in time however the pointer's events happen to arrive.
      d.px = px; d.py = py;
      live = { id: d.id, x: p.x - d.dx, y: p.y - d.dy };
      return;
    }
    let next: ClipDoc;
    if (d.kind === "move") next = placeAt(d.origin, d.id, outT, { x: p.x - d.dx, y: p.y - d.dy });
    else if (d.kind === "turn") {
      const { factor, degrees } = scaleTurn(d.cx, d.cy, d.fx, d.fy, px, py);
      next = transformOverlay(d.origin, d.id, factor, degrees);
    } else {
      const layer = d.origin.layers.find((l) => l.id === d.id);
      if (!layer) return;
      const box = d.kind === "box" ? { ...layer.box, x: p.x - d.dx, y: p.y - d.dy } : resizeBox(layer.box, d.corner, p.x, p.y);
      next = updateLayer(d.origin, d.id, { box });
    }
    stageStep(d, next);
  }
  /** One stage drag is one undo step: the first real change pushes, every later one replaces it. */
  function stageStep(d: { origin: ClipDoc; stepped: boolean }, next: ClipDoc) {
    if (!history) return;
    if (!d.stepped && next === d.origin) return;
    history = d.stepped ? replaceEdit(history, next) : pushEdit(history, next);
    d.stepped = true;
    lastGroup = null;
  }
  function onStageUp(e: PointerEvent) {
    const el = e.currentTarget as Element;
    if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture?.(e.pointerId);
    endStageDrag();
  }
  /**
   * Capture lost while the button is still down is not the end of the drag. Chromium can drop a
   * held capture on its own (seen in headless Edge mid-recording, with the button down and nothing
   * else touching the stage), and ending there would throw away the motion being recorded. A real
   * release still ends it: `pointerup`, or the next move with no button held (`onStageMove`).
   */
  function onStageLostCapture(e: PointerEvent) {
    if ((e.buttons & 1) === 0) endStageDrag();
  }
  function endStageDrag() {
    const d = stageDrag;
    stageDrag = null;
    unwatchRelease();
    if (!d) return;
    if (d.kind === "tap" && d.toggle) togglePlay();
    if (d.kind === "record") commitRecording(d);
  }
  /**
   * Drop the stage drag in progress WITHOUT applying it, along with its live preview. Any other
   * edit, undo or redo does this: left behind, a recording's live position would keep drawing the
   * overlay where the pointer was, so the stage would stop matching what will be posted.
   */
  function cancelStageDrag() {
    stageDrag = null;
    live = null;
    trail = [];
    unwatchRelease();
  }
  /**
   * While a stage drag runs, a release anywhere ends it, even one the stage never hears because
   * Chromium dropped its pointer capture (see `onStageLostCapture`), and so does the window losing
   * focus. Without this, a recording would keep sampling the last place the pointer was seen.
   */
  function onWindowRelease() { endStageDrag(); }
  function watchRelease() {
    window.addEventListener("pointerup", onWindowRelease);
    window.addEventListener("blur", onWindowRelease);
  }
  function unwatchRelease() {
    window.removeEventListener("pointerup", onWindowRelease);
    window.removeEventListener("blur", onWindowRelease);
  }

  /** Add a recording sample at the clip's current time, where the pointer is. Called on the drag's
   * first press and then once per animation frame while playing. */
  function sampleRecording(d: Extract<StageDrag, { kind: "record" }>) {
    const p = toFraction(frame, d.px, d.py);
    const x = p.x - d.dx, y = p.y - d.dy;
    live = { id: d.id, x, y };
    // The drawn trail is the most recent stretch only: rebuilding an ever longer line every frame
    // would slow a long recording down. Every sample is still kept for the path itself.
    if (d.recorder.add(outT, x, y)) trail = d.recorder.samples.slice(-TRAIL_SAMPLES);
  }
  /** The recorded stretch becomes the overlay's path: one undo step. */
  function commitRecording(d: Extract<StageDrag, { kind: "record" }>) {
    if (stageDrag === d) { stageDrag = null; unwatchRelease(); }
    live = null;
    trail = [];
    if (!history || !editing || d.recorder.samples.length < 2) return;
    const next = recordPath(history.present, d.id, d.recorder.samples);
    if (next !== history.present) { history = pushEdit(history, next); lastGroup = null; }
  }

  /** R: record motion for the selected text or sticker. Plays; dragging then draws its path. */
  function toggleRecord() {
    if (recording) { recording = false; pause(); return; }
    if (!editing || !selOverlay) return;
    recording = true;
    bladeMode = false; bladeAt = null;
    if (!playing) play();
  }

  // --- Timeline pointer handling ----------------------------------------------------------------
  /** Source seconds under a pointer, from the track's live geometry (never a cached width). */
  function timeAt(e: PointerEvent): number {
    const rect = trackEl?.getBoundingClientRect();
    if (!rect || rect.width <= 0 || duration <= 0) return 0;
    return Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)) * duration;
  }
  function onTimelineDown(e: PointerEvent) {
    if (!editing || !edit || !doc || e.button !== 0) return;
    // A press on the timeline's own scrollbar (past its client width) is scrolling, not scrubbing.
    const box = e.currentTarget as HTMLElement;
    if (e.target === box && box.clientWidth > 0 && e.offsetX >= box.clientWidth) return;
    const t = timeAt(e);
    const target = e.target as Element | null;
    if (target?.closest("[data-lane-head]")) return;
    const handle = target?.closest("[data-handle]")?.getAttribute("data-handle");
    const cut = target?.closest("[data-cut]")?.getAttribute("data-cut");
    const bar = target?.closest("[data-bar]")?.getAttribute("data-bar");
    const barEdge = target?.closest("[data-edge]")?.getAttribute("data-edge");
    e.preventDefault();
    if (handle === "in" || handle === "out") {
      drag = { kind: handle, cut: -1, stepped: false };
    } else if (cut !== null && cut !== undefined) {
      drag = { kind: "cut", cut: Number(cut), stepped: false };
    } else if (bladeMode) {
      // The blade cuts where it is, and that is the whole gesture: no drag follows.
      applyBase(splitAt(edit, t));
      return;
    } else if (bar) {
      const [item, id] = bar.split(":") as ["overlay" | "audio" | "layer", string];
      selection = { kind: item, id };
      const edge = barEdge === "start" || barEdge === "end" ? barEdge : "move";
      drag = { kind: "bar", item, id, edge, from: t, origin: doc, stepped: false };
    } else {
      drag = { kind: "scrub", cut: -1, stepped: false };
      pause();
      seek(t);
      // On V1 the piece under the pointer is selected; elsewhere the click just moves the playhead.
      selection = target?.closest(".track") ? { kind: "piece", index: pieceIndexAt(edit, t) } : null;
    }
    pause();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }
  function onTimelineMove(e: PointerEvent) {
    if (bladeMode && editing) bladeAt = timeAt(e);
    if (!drag || !history || !editing) return;
    // A drag whose button is no longer down lost its release somewhere (focus left the window
    // mid-drag); end it here rather than letting a plain hover keep editing.
    if ((e.buttons & 1) === 0) { drag = null; return; }
    const t = timeAt(e);
    if (drag.kind === "scrub") { seek(t); return; }
    if (drag.kind === "bar") { barStep(drag, t); return; }
    const present = history.present;
    const presentEdit = baseLayer(present).edit;
    const nextEdit = drag.kind === "in" ? setInPoint(presentEdit, t)
      : drag.kind === "out" ? setOutPoint(presentEdit, t)
      : moveCut(presentEdit, drag.cut, t);
    if (nextEdit === presentEdit) return;
    const next = withBaseEdit(present, nextEdit);
    // One drag is one undo step: the first real change pushes, every later one replaces it.
    history = drag.stepped ? replaceEdit(history, next) : pushEdit(history, next);
    drag.stepped = true;
    lastGroup = null;
    // Show the frame at what is being moved, so a trim lands on the picture the person wants.
    seek(drag.kind === "in" ? nextEdit.inPoint : drag.kind === "out" ? nextEdit.outPoint : nextEdit.cuts[drag.cut]);
  }
  /**
   * Move or trim a bar. Always computed from the document as it was when the drag began, so a
   * drag that runs into the clip's end and comes back gets its full length back.
   */
  function barStep(d: Extract<TimelineDrag, { kind: "bar" }>, t: number) {
    if (!history) return;
    const origin = d.origin;
    const total = clipDuration(origin);
    const delta = outputDelta(baseLayer(origin).edit, d.from, t);
    let next = origin;
    if (d.item === "overlay") {
      const o = origin.overlays.find((x) => x.id === d.id);
      if (!o) return;
      if (d.edge === "move") {
        const dd = Math.min(Math.max(delta, -o.start), total - o.end);
        next = setOverlayTimes(origin, d.id, o.start + dd, o.end + dd);
      } else if (d.edge === "start") next = setOverlayTimes(origin, d.id, Math.min(o.start + delta, o.end - MIN_ITEM_SECONDS), o.end, false);
      else next = setOverlayTimes(origin, d.id, o.start, o.end + delta, false);
    } else if (d.item === "audio") {
      const a = origin.audio.find((x) => x.id === d.id);
      if (!a) return;
      if (d.edge === "move") next = updateAudio(origin, d.id, { start: a.start + delta });
      else if (d.edge === "start") {
        const dd = Math.min(Math.max(delta, -a.offset, -a.start), a.length - MIN_ITEM_SECONDS);
        next = updateAudio(origin, d.id, { start: a.start + dd, offset: a.offset + dd, length: a.length - dd });
      } else next = updateAudio(origin, d.id, { length: a.length + delta });
    } else {
      const layer = origin.layers.find((x) => x.id === d.id);
      if (!layer) return;
      if (d.edge === "move") next = updateLayer(origin, d.id, { start: layer.start + delta });
      else if (d.edge === "start") {
        const trimmed = setInPoint(layer.edit, layer.edit.inPoint + delta);
        next = updateLayer(origin, d.id, { edit: trimmed, start: layer.start + (trimmed.inPoint - layer.edit.inPoint) });
      } else next = updateLayer(origin, d.id, { edit: setOutPoint(layer.edit, layer.edit.outPoint + delta) });
    }
    if (!d.stepped && next === origin) return;
    history = d.stepped ? replaceEdit(history, next) : pushEdit(history, next);
    d.stepped = true;
    lastGroup = null;
  }
  /** Release, cancel and a lost pointer capture all end the drag. */
  function onTimelineUp(e: PointerEvent) {
    if (!drag) return;
    drag = null;
    const el = e.currentTarget as Element;
    if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture?.(e.pointerId);
  }

  // --- Export -----------------------------------------------------------------------------------
  /** What the compositor reads the document's media from. */
  function clipMedia(): ClipMedia {
    return {
      video: (source) => bin?.video(source)?.file ?? null,
      image: (source) => { const m = bin?.image(source); return m ? { image: m.image, width: m.width, height: m.height } : null; },
      sound: (source) => bin?.sound(source)?.buffer ?? null,
    };
  }
  /**
   * The one action: post (embed, upload) the clip. Where it posts (chat, announcements) the result
   * carries `post`, and the host sends straight away. An untouched clip goes up as the original
   * bytes, a cut-only one through the light exporter, anything else through the compositor.
   */
  async function upload() {
    // Never while "Discard this clip?" is up: the answer to that question is not "post it".
    if (!doc || !editing || uploadDisabled || confirmingDiscard) return;
    const post = words.posts;
    if (untouched) { finish(post ? { kind: "original", post: true } : { kind: "original" }); return; }
    pause();
    bladeMode = false; bladeAt = null;
    const controller = new AbortController();
    exportAbort = controller;
    progress = 0;
    phase = "exporting";
    // The progress row (with its cancel) covers the timeline's top; a timeline scrolled down
    // through many rows would hide it. Back to the top, held there by `.exporting` until it ends.
    if (timelineEl) timelineEl.scrollTop = 0;
    const onProgress = (fraction: number) => { if (exportAbort === controller) progress = fraction; };
    const posting = doc;
    // The Studio songs this clip would sound are checked against the channel's list again first:
    // one may have expired while the editor stayed open (see `checkSongsOffered`). Only a composed
    // clip can hold a song.
    const running = composed
      ? checkSongsOffered(music, songsInClip(posting)).then(() => {
        if (controller.signal.aborted) throw new ExportCanceledError();
        return composer(posting, clipMedia(), { capBytes, signal: controller.signal, onProgress, container: editableContainer(file.type) ?? "mp4", name: file.name });
      })
      : exporter(file, baseLayer(posting).edit, { capBytes, signal: controller.signal, onProgress });
    exportSettled = running.then(() => {}, () => {});
    try {
      const result = await running;
      // A cancel, a close or a newer export already moved on; this result belongs to none of them.
      if (exportAbort !== controller || finished) return;
      exportAbort = null;
      finish(post ? { kind: "edited", file: result, post: true } : { kind: "edited", file: result });
    } catch (error) {
      if (exportAbort !== controller || finished) return;
      exportAbort = null;
      if (error instanceof ExportCanceledError) { phase = "ready"; return; }
      if (error instanceof OutputTooLargeError) {
        message = `The edited video is over this server's ${formatBytes(capBytes)} limit`;
        messageTip = "Trim or remove more to make it smaller";
      } else {
        // The exporters' own errors are sentences meant for the person; anything else they have
        // already replaced with one (see `exportEditedVideo` and `exportClip`).
        message = error instanceof Error && error.message ? error.message : "The edited video could not be made.";
        messageTip = "";
      }
      phase = "failed";
    }
  }
  /** The Studio songs `d` would sound when posted (muted ones and ones past the end are silent). */
  function songsInClip(d: ClipDoc): string[] {
    const ids = new Set<string>();
    for (const piece of soundPieces(d)) {
      // By the bin's key, not the piece's kind: a song is a song however it was laid down.
      const id = piece.kind === "video" ? null : songOfKey(bin?.sound(piece.source)?.key);
      if (id) ids.add(id);
    }
    return [...ids];
  }
  /** The last export's settlement, whatever its outcome; what a cancel waits on before Upload returns. */
  let exportSettled: Promise<void> = Promise.resolve();
  /** Cancel answers at once; the export's own rejection then finds itself superseded. */
  function cancelExport() {
    if (phase !== "exporting") return;
    const controller = exportAbort;
    exportAbort = null;
    controller?.abort();
    phase = "ready";
    draining = true;
    void exportSettled.then(() => { if (!destroyed) draining = false; });
  }
  /** The original bytes, when the editor cannot open the file or the export failed. Never re-encodes. */
  function sendOriginal() {
    if (originalTooBig || finished || confirmingDiscard) return;
    finish(words.posts ? { kind: "original", post: true } : { kind: "original" });
  }
  /**
   * The ×, and Escape once nothing else claims it. During an export it cancels the export rather
   * than leaving; with work on the clip it asks before throwing that away; otherwise it leaves.
   */
  function closeRequest() {
    if (phase === "exporting") { cancelExport(); return; }
    if (dirty && (phase === "ready" || phase === "failed")) { askDiscard(); return; }
    finish({ kind: "skip" });
  }
  async function askDiscard() {
    if (recording) { recording = false; pause(); }
    confirmingDiscard = true;
    // The safe answer takes focus, so a stray Enter keeps the work.
    await tick();
    keepButton?.focus();
  }
  function backToEditing() { if (phase === "failed") { message = ""; messageTip = ""; phase = "ready"; } }

  // --- Keyboard ---------------------------------------------------------------------------------
  /** Focus is in a control that takes typed keys (the text field, a slider, the effect list). */
  function typingIn(target: HTMLElement | null): boolean {
    if (!target) return false;
    if (target.tagName === "TEXTAREA" || target.tagName === "SELECT" || target.isContentEditable) return true;
    return target.tagName === "INPUT" && !["button", "checkbox", "radio", "file"].includes((target as HTMLInputElement).type);
  }
  function onKey(e: KeyboardEvent) {
    // A few keys belong to the app even over a modal (Lock, push-to-talk): those pass untouched.
    if (passKey(e)) {
      // A plain passed key (push-to-talk) must not also activate the focused editor button the
      // way Space or Enter would. App's handlers do not consult `defaultPrevented`. In a text
      // field it is a character the person is typing (App's own push-to-talk ignores keys typed
      // into fields), so it is left to type: a push-to-talk Space must still put spaces in words.
      if (!e.ctrlKey && !e.metaKey && !e.altKey && !typingIn(e.target as HTMLElement | null)) e.preventDefault();
      return;
    }
    // The editor is modal: no other key reaches the app's own shortcuts behind it.
    e.stopPropagation();
    if (e.key === "Tab") { trapTab(e); return; }
    const target = e.target as HTMLElement | null;
    const typing = typingIn(target);
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Escape") {
      e.preventDefault();
      if (confirmingDiscard) { confirmingDiscard = false; card?.focus(); }
      else if (typing) { settle(); card?.focus(); }
      else if (phase === "exporting") cancelExport();
      else if (recording) toggleRecord();
      else if (bladeMode) { bladeMode = false; bladeAt = null; }
      else if (selection && selection.kind !== "piece") selection = null;
      else closeRequest();
      return;
    }
    // While the discard question is up, only its buttons act (Enter and Space press the focused
    // one natively); no editing key may change the clip behind it.
    if (confirmingDiscard) return;
    // In a text field, a slider or the effect list, keys are the control's own: typing "s" types.
    if (typing) {
      if (mod && e.key === "Enter") { e.preventDefault(); settle(); void upload(); }
      return;
    }
    const onButton = !!target && target.tagName === "BUTTON";
    if (e.key === " ") {
      // A focused button activates on Space natively; toggling playback too would double it.
      if (onButton || phase === "exporting") return;
      e.preventDefault(); togglePlay(); return;
    }
    if (!editing) return;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    let handled = true;
    if (mod && key === "z") { if (e.shiftKey) redoStep(); else undoStep(); }
    else if (mod && key === "y") redoStep();
    else if (mod && key === "Enter") void upload();
    else if (mod || e.altKey) handled = false;
    else if (key === "ArrowLeft") nudge(-(e.shiftKey ? NUDGE_LARGE : NUDGE));
    else if (key === "ArrowRight") nudge(e.shiftKey ? NUDGE_LARGE : NUDGE);
    else if (key === "Home" && edit) { pause(); seek(edit.inPoint); }
    else if (key === "End" && edit) { pause(); seek(edit.outPoint); }
    else if (key === "b" && !recording) { bladeMode = !bladeMode; if (!bladeMode) bladeAt = null; }
    else if (key === "s" && !recording) splitHere();
    else if (key === "i" && !recording) setInHere();
    else if (key === "o" && !recording) setOutHere();
    else if (key === "r") toggleRecord();
    else if (key === "l") libraryOpen = !libraryOpen;
    else if (key === "m") muted = !muted;
    else if (key === "Delete" || key === "Backspace") { if (!deleteSelected() && !recording) toggleTarget(); }
    else handled = false;
    if (handled) e.preventDefault();
  }
  /**
   * Keep Tab inside the dialog: `aria-modal` promises that nothing behind the card is reachable.
   * Only the wrap-around edges are handled; in between, the natural order stays inside the card.
   */
  function trapTab(e: KeyboardEvent) {
    // While "Discard this clip?" is up it is the dialog: Tab cycles its two answers only, never
    // back into the editor behind it (whose post button Shift+Tab from "keep editing" reached).
    const scope = confirmingDiscard ? confirmBox : card;
    if (!scope) return;
    const focusable = [...scope.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]):not([type="file"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')];
    if (!focusable.length) { e.preventDefault(); return; }
    const index = focusable.indexOf(document.activeElement as HTMLElement);
    let next: HTMLElement | null = null;
    if (e.shiftKey) { if (index <= 0) next = focusable[focusable.length - 1]; }
    else if (index === -1 || index === focusable.length - 1) next = focusable[0];
    if (next) { e.preventDefault(); next.focus(); }
  }

  // --- Rendering helpers ------------------------------------------------------------------------
  /** Percentage of the track width for source time `t`. */
  function pct(t: number): number {
    return duration > 0 ? (Math.min(Math.max(0, t), duration) / duration) * 100 : 0;
  }
  /** A bar's left and width, in track percentages, for an output-time span. */
  function barPct(start: number, end: number): { left: number; width: number } {
    if (!doc) return { left: 0, width: 0 };
    const span = axisSpan(doc, start, end);
    return { left: pct(span.from), width: Math.max(0.4, pct(span.to) - pct(span.from)) };
  }
  /** Where an output time falls along a bar, in percent of the bar. */
  function withinBar(bar: { left: number; width: number }, t: number): number {
    if (!edit || bar.width <= 0) return 0;
    return ((pct(outputToAxis(edit, t)) - bar.left) / bar.width) * 100;
  }
  function overlayLabel(o: Overlay): string {
    void mediaRev;
    return o.content.kind === "text" ? o.content.text : bin?.image(o.content.source)?.name ?? "sticker";
  }
  function audioLabel(a: AudioItem): string {
    void mediaRev;
    return bin?.sound(a.source)?.name ?? (a.kind === "music" ? "music" : "sound");
  }
  /** Action: paint a decoded tile into its canvas, cover-fitted, whenever the tile changes. */
  function paint(node: HTMLCanvasElement, image: CanvasImageSource | null) {
    const draw = (source: CanvasImageSource | null) => {
      const ctx = node.getContext?.("2d");
      if (!ctx) return;
      ctx.clearRect(0, 0, node.width, node.height);
      if (!source) return;
      const sw = (source as { width: number }).width || node.width;
      const sh = (source as { height: number }).height || node.height;
      const scale = Math.max(node.width / sw, node.height / sh);
      const w = sw * scale, h = sh * scale;
      try { ctx.drawImage(source, (node.width - w) / 2, (node.height - h) / 2, w, h); }
      catch { /* a tile that cannot be drawn stays blank; the strip is decoration */ }
    };
    draw(image);
    return { update: draw };
  }
  /** Action: keep a canvas sized and placed over the stage's frame (CSSOM, which the CSP allows). */
  function place(node: HTMLElement, rect: { x: number; y: number; w: number; h: number }) {
    const set = (r: typeof rect) => {
      node.style.setProperty("left", `${r.x}px`); node.style.setProperty("top", `${r.y}px`);
      node.style.setProperty("width", `${r.w}px`); node.style.setProperty("height", `${r.h}px`);
    };
    set(rect);
    return { update: set };
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- The app's own context menus would open above the dialog and lose their focus to `keepFocus`,
     leaving Esc to close the editor instead of the menu: the editor has no context menu at all. -->
<div class="video-editor" data-video-editor role="presentation" bind:this={root} onkeydown={onKey} onmousedown={onBackdropDown}
  onfocusout={onFocusOut} oncontextmenu={(e) => e.preventDefault()} ondragover={onDragOver} ondrop={onDrop}>
  <div class="card" role="dialog" aria-modal="true" aria-label="Edit video before upload" tabindex="-1" data-state={phase} bind:this={card}>
    <!-- Everything but the discard question is inert while it is up (see `trapTab`). -->
    <header class="head" inert={confirmingDiscard}>
      <svg class="film" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="3" width="13" height="10" rx="1"></rect><path d="M4.5 3v10M11.5 3v10M1.5 6.5h3M11.5 6.5h3M1.5 9.5h3M11.5 9.5h3"></path></svg>
      <h2 class="title" title={file.name}>{file.name}</h2>
      <span class="facts">
        {#if size.width && size.height}<span class="micro">{size.width}×{size.height}</span>{/if}
        {#if duration}<span class="micro">{formatClock(duration)}</span>{/if}
        <span class="micro">{formatBytes(file.size)}</span>
      </span>
      <span class="grow"></span>
      {#if position && position.count > 1}<span class="micro">{position.index} of {position.count}</span>{/if}
      {#if doc}
        <button type="button" class="st-btn ghost x" class:on={libraryOpen} data-action="library" aria-pressed={libraryOpen} aria-label="Library" title="Library (L)" onclick={() => (libraryOpen = !libraryOpen)}><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><rect x="1.5" y="2.5" width="13" height="11" rx="1"></rect><path d="M10 2.5v11"></path></svg></button>
      {/if}
      <button type="button" class="st-btn ghost x" data-action="close" aria-label={phase === "exporting" ? "Cancel export" : "Don't upload"} title={phase === "exporting" ? "Cancel export (Esc)" : "Don't upload (Esc)"} onclick={closeRequest}><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"></path></svg></button>
    </header>

    <div class="middle" inert={confirmingDiscard}>
      <div class="left">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="ve-stage" class:grab={stageHover !== null} class:rec={recording} bind:this={stageEl}
          onpointerdown={onStageDown} onpointermove={onStageMove} onpointerup={onStageUp} onpointercancel={onStageUp} onlostpointercapture={onStageLostCapture}
          onpointerleave={() => { if (!stageDrag) stageHover = null; }}>
          <canvas class="stage-canvas" bind:this={canvasEl} use:place={frame} aria-hidden="true"></canvas>
          {#if pathPoints || trailPoints}
            <svg class="paths" aria-hidden="true">
              {#if pathPoints}<polyline class="path" points={pathPoints}></polyline>{/if}
              {#if trailPoints}<polyline class="trail" points={trailPoints}></polyline>{/if}
            </svg>
          {/if}
          {#if selBox}
            <div class="sel" style:left="{selBox.cx - selBox.w / 2}px" style:top="{selBox.cy - selBox.h / 2}px" style:width="{selBox.w}px" style:height="{selBox.h}px" style:transform="rotate({selBox.rotation}deg)">
              <span class="corner nw"></span><span class="corner ne"></span><span class="corner sw"></span>
              <span class="turn" data-stage-handle="turn" title="Drag to scale and turn"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M12.5 8A4.5 4.5 0 1 1 11 4.6"></path><path d="M11.5 1.5v3.5H8"></path></svg></span>
            </div>
          {/if}
          {#if selLayerBox}
            <div class="sel box" style:left="{selLayerBox.x}px" style:top="{selLayerBox.y}px" style:width="{selLayerBox.w}px" style:height="{selLayerBox.h}px">
              {#each CORNERS as c (c)}<span class="resize {c}" data-stage-handle={c} title="Resize"></span>{/each}
            </div>
          {/if}
          {#if phase === "probing"}<span class="ve-chip ve-stage-chip" role="status"><i></i><span class="micro">reading video</span></span>{/if}
        </div>
        {#if editing || phase === "exporting" || phase === "failed"}
          <ClipInspector target={inspectorTarget} {recording} locked={phase !== "ready"}
            ontext={inspectText} onsize={inspectSize} onturn={inspectTurn} onrecord={toggleRecord}
            onclearpath={() => { if (doc && selOverlay) apply(clearPath(doc, selOverlay.id, outT)); }}
            onaudio={inspectAudio} onlayer={inspectLayer}
            onmovelayer={(step) => { if (doc && selLayerIndex > 0) apply(moveLayer(doc, doc.layers[selLayerIndex].id, step)); }}
            ondelete={deleteSelected} onsettle={settle} />
        {/if}
      </div>
      {#if libraryOpen && doc && phase !== "unavailable"}
        <ClipLibrary tab={libraryTab} ontab={(t) => (libraryTab = t)} images={libImages} musicFiles={libMusicFiles} soundFiles={libSoundFiles}
          {songs} {songsStatus} {songsError} {songState} hasStudio={!!music} {onClip} layers={libLayers} layersFull={doc.layers.length >= MAX_VIDEO_LAYERS}
          {auditioning} locked={phase !== "ready" || recording}
          onaddtext={addTextFx} onaddemoji={addEmoji} onaddimage={addPicture} onaddsfx={addSfx} onaudition={audition}
          onaddsound={addSound} onaddsong={addSong} onlistsongs={listSongs} onfiles={addFiles} onlayermute={toggleLayerMute} />
      {/if}
    </div>

    <!-- The media the stage draws from: the clip itself and each added video. Never shown directly. -->
    <div class="pool" aria-hidden="true">
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={videoEl} src={previewUrl || undefined} preload="auto" playsinline
        onplay={onVideoPlay} onpause={onVideoPause} onended={onVideoPause} ontimeupdate={follow}
        onloadeddata={requestDraw} onseeked={() => { if (videoEl && !playing) playhead = videoEl.currentTime; requestDraw(); }}></video>
      {#each doc?.layers.slice(1) ?? [] as layer (layer.id)}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={layerEls[layer.id]} src={layerUrls.get(layer.source) || undefined} preload="auto" playsinline muted
          onloadeddata={() => { syncLayers(); requestDraw(); }} onseeked={requestDraw}></video>
      {/each}
    </div>

    {#if phase === "unavailable"}
      <p class="alert" role="alert">{message}</p>
    {:else if edit && doc}
      <div class="transport" inert={confirmingDiscard}>
        <!-- The visible text is exactly "Play" or "Pause": scripts and screen readers rely on it. -->
        <button type="button" class="st-tool play" class:on={playing} data-action="play" disabled={phase === "exporting"} title={playing ? "Pause (Space)" : "Play (Space)"} onclick={togglePlay}>{#if playing}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}<span class="vh">{playing ? "Pause" : "Play"}</span></button>
        <button type="button" class="st-tool sm" data-action="back" disabled={!editing} title="Back 0.1 s (Left, Shift for 1 s)" onclick={() => nudge(-NUDGE)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg><span class="vh">Back</span></button>
        <button type="button" class="st-tool sm" data-action="forward" disabled={!editing} title="Forward 0.1 s (Right, Shift for 1 s)" onclick={() => nudge(NUDGE)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg><span class="vh">Forward</span></button>
        <span class="mono strong" title="Playhead in the original">{formatClock(playhead)}</span>
        <span class="micro" title="Position in the upload / length of the upload">{formatClock(outT)} / {formatClock(keptSeconds)}</span>
        {#if recording}<span class="ve-chip rec-chip" role="status"><i></i><span class="micro">rec</span></span>{/if}

        <span class="sep" aria-hidden="true"></span>
        <button type="button" class="st-tool ol" class:on={bladeMode} data-action="blade" aria-pressed={bladeMode} disabled={!editing || recording} title={bladeMode ? "Blade: on, click the timeline to cut (B, Esc to leave)" : "Blade (B)"} onclick={() => { bladeMode = !bladeMode; if (!bladeMode) bladeAt = null; }}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 1.5 14.5 6.5 6 15 1 10z"></path><path d="M4 12l2 2M9.5 1.5 7 4"></path></svg><span class="vh">Blade</span></button>
        <button type="button" class="st-tool ol" data-action="split" disabled={!editing || recording || !canSplitAt(edit, playhead)} title="Split at playhead (S)" onclick={splitHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.5v13"></path><path d="M2 5h4v6H2M14 5h-4v6h4"></path></svg><span class="vh">Split</span></button>
        <button type="button" class="st-tool ol" class:danger={!targetRemoved} data-action="remove" disabled={!editing || recording || targetPiece === null} title={targetRemoved ? "Restore piece (Delete)" : "Remove piece (Delete)"} onclick={toggleTarget}>{#if targetRemoved}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8a5 5 0 1 0 1.5-3.5"></path><path d="M3 2.5v3h3"></path></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg>{/if}<span class="vh">{targetRemoved ? "Restore piece" : "Remove piece"}</span></button>
        <button type="button" class="st-tool ol" data-action="set-in" disabled={!editing || recording} title="Trim start to playhead (I)" onclick={setInHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6.5 2.5h-3v11h3"></path><path d="M9 8h5M11.5 5.5 14 8l-2.5 2.5"></path></svg><span class="vh">Trim start</span></button>
        <button type="button" class="st-tool ol" data-action="set-out" disabled={!editing || recording} title="Trim end to playhead (O)" onclick={setOutHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 2.5h3v11h-3"></path><path d="M7 8H2M4.5 5.5 2 8l2.5 2.5"></path></svg><span class="vh">Trim end</span></button>

        <span class="sep" aria-hidden="true"></span>
        <button type="button" class="st-tool ol rec-tool" class:recording data-action="record" aria-pressed={recording} disabled={!editing || (!recording && !selOverlay)}
          title={recording ? "Stop recording (R)" : selOverlay ? "Record motion: plays, and your drag becomes its path (R)" : "Record motion: select text or a sticker first (R)"} onclick={toggleRecord}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 14c1.5-.5 2.5-2 3.5-3.5S7.5 7.5 9 7" stroke-dasharray="0.1 2.6"></path><circle cx="11.5" cy="5" r="2.6" class="dot"></circle></svg><span class="vh">Record motion</span></button>

        <span class="sep" aria-hidden="true"></span>
        <button type="button" class="st-tool ol" data-action="undo" disabled={!editing || recording || !history || history.past.length === 0} title="Undo (Ctrl+Z)" onclick={undoStep}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3 2.5 6l3 3"></path><path d="M2.5 6h7a4 4 0 0 1 0 8H7"></path></svg><span class="vh">Undo</span></button>
        <button type="button" class="st-tool ol" data-action="redo" disabled={!editing || recording || !history || history.future.length === 0} title="Redo (Ctrl+Shift+Z)" onclick={redoStep}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10.5 3l3 3-3 3"></path><path d="M13.5 6h-7a4 4 0 0 0 0 8H9"></path></svg><span class="vh">Redo</span></button>
        <button type="button" class="st-tool ol" data-action="reset" disabled={!editing || recording || !dirty} title="Reset all edits" onclick={reset}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 8a5.5 5.5 0 1 0 1.8-4.1"></path><path d="M2.5 2v3.5H6"></path></svg><span class="vh">Reset</span></button>

        <span class="grow"></span>
        <button type="button" class="st-tool ol" class:on={muted} data-action="mute" aria-pressed={muted} title={muted ? "Sound off (M)" : "Sound on (M)"} onclick={() => (muted = !muted)}>{#if muted}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 6l3 4M14 6l-3 4"></path></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4"></path></svg>{/if}<span class="vh">{muted ? "Unmute" : "Mute"}</span></button>
      </div>

      <!-- The timeline is the playhead's slider for assistive tech: the arrow keys (handled on the
           overlay) move the playhead, and the value reads as a clock. Cuts, handles and bars are
           pointer affordances; their keyboard equivalents are S, I, O and Delete. -->
      <div class="timeline" class:blade={bladeMode} class:locked={!editing} class:exporting={phase === "exporting"} role="slider" tabindex="0" data-timeline inert={confirmingDiscard}
        bind:this={timelineEl}
        aria-label="Playhead" aria-valuemin={0} aria-valuemax={Math.round(duration * 10) / 10}
        aria-valuenow={Math.round(playhead * 10) / 10} aria-valuetext={formatClock(playhead)}
        onpointerdown={onTimelineDown} onpointermove={onTimelineMove} onpointerup={onTimelineUp} onpointercancel={onTimelineUp}
        onlostpointercapture={onTimelineUp}
        onpointerleave={() => { bladeAt = null; }}>
        <div class="ruler" aria-hidden="true">
          {#each ruler.ticks as t (t)}
            <span class="tick" class:last={t === ruler.ticks[ruler.ticks.length - 1] && pct(t) > 92} style:left="{pct(t)}%"><span class="micro">{rulerLabel(t, ruler.step)}</span></span>
          {/each}
          <span class="head-mark" style:left="{pct(playhead)}%"></span>
        </div>
        <div class="lanes" style:height="{lanes.height - LANE.ruler - 4}px">
          <!-- Lane heads: what each lane is, and its mute (or, for text and stickers, an eye). -->
          <div class="lane-head" data-lane-head style:top="{lanes.v1}px" style:height="{LANE.v1}px">
            <span class="lane-name">V1</span>
            <button type="button" class="st-tool tiny ol" class:on={baseLayer(doc).muted} data-action="mute-v1" aria-pressed={baseLayer(doc).muted} title={!baseHasAudio ? "This clip has no sound" : baseLayer(doc).muted ? "Unmute the clip's sound" : "Mute the clip's sound"} disabled={!editing || (!baseHasAudio && !baseLayer(doc).muted)} onclick={() => toggleLayerMute(baseLayer(doc).id)}>{@render speaker(baseLayer(doc).muted)}</button>
          </div>
          {#each doc.layers.slice(1) as layer, i (layer.id)}
            {@const silent = !layer.muted && !hasSound(layer.source)}
            <div class="lane-head" data-lane-head style:top="{lanes.layers[i]}px" style:height="{LANE.layer}px">
              <span class="lane-name">V{i + 2}</span>
              <button type="button" class="st-tool tiny ol" class:on={layer.muted} aria-pressed={layer.muted} title={layer.muted ? "Unmute layer" : silent ? "This video has no sound" : "Mute layer"} aria-label={`${layer.muted ? "Unmute" : "Mute"} V${i + 2}`} disabled={!editing || silent} onclick={() => toggleLayerMute(layer.id)}>{@render speaker(layer.muted)}</button>
            </div>
          {/each}
          <div class="lane-head" data-lane-head title="Text and stickers" style:top="{lanes.overlays}px" style:height="{lanes.overlayHeight}px">
            <span class="lane-name">T</span>
            <button type="button" class="st-tool tiny ol" class:on={hideOverlays} data-action="hide-overlays" aria-pressed={hideOverlays} title={hideOverlays ? "Show text and stickers" : "Hide text and stickers while editing"} onclick={() => (hideOverlays = !hideOverlays)}>{#if hideOverlays}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 2l12 12M6.5 4a6.6 6.6 0 0 1 8 4s-.8 1.5-2.3 2.7M9.5 12.2A6.6 6.6 0 0 1 1.5 8s.8-1.5 2.3-2.7"></path></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z"></path><circle cx="8" cy="8" r="2"></circle></svg>{/if}</button>
          </div>
          <div class="lane-head" data-lane-head title="Music and sounds" style:top="{lanes.audio}px" style:height="{lanes.audioHeight}px">
            <span class="lane-name">A</span>
            <button type="button" class="st-tool tiny ol" class:on={doc.audio.length > 0 && doc.audio.every((a) => a.muted)} data-action="mute-audio" aria-pressed={doc.audio.length > 0 && doc.audio.every((a) => a.muted)} title="Mute or unmute all music and sounds" disabled={!editing || doc.audio.length === 0} onclick={toggleAudioLane}>{@render speaker(doc.audio.length > 0 && doc.audio.every((a) => a.muted))}</button>
          </div>

          <div class="track" bind:this={trackEl} style:top="{lanes.v1}px">
            <div class="strip" aria-hidden="true">
              {#each tiles as image, i (i)}
                <canvas width={Math.round(TILE_HEIGHT * 1.78)} height={TILE_HEIGHT} use:paint={image}></canvas>
              {/each}
            </div>
            {#each pieceList as piece (piece.index)}
              <div class="piece" class:removed={piece.removed} class:selected={piece.index === selectedPiece}
                style:left="{pct(piece.start)}%" style:width="{pct(piece.end) - pct(piece.start)}%"
                data-piece={piece.index} data-removed={piece.removed}></div>
            {/each}
            <div class="trimmed" style:left="0%" style:width="{pct(edit.inPoint)}%" aria-hidden="true"></div>
            <div class="trimmed" style:left="{pct(edit.outPoint)}%" style:width="{100 - pct(edit.outPoint)}%" aria-hidden="true"></div>
            {#each edit.cuts as cut, i (i)}
              <div class="cut" style:left="{pct(cut)}%" data-cut={i} title={`Cut at ${formatClock(cut)} (drag to move)`}><span class="knob"></span></div>
            {/each}
            <div class="handle in" style:left="{pct(edit.inPoint)}%" data-handle="in" title={`Start ${formatClock(edit.inPoint)} (drag to trim)`}></div>
            <div class="handle out" style:left="{pct(edit.outPoint)}%" data-handle="out" title={`End ${formatClock(edit.outPoint)} (drag to trim)`}></div>
          </div>

          {#each doc.layers.slice(1) as layer, i (layer.id)}
            {@const span = layerAxisSpan(doc, i + 1)}
            <div class="lane" style:top="{lanes.layers[i]}px" style:height="{LANE.layer}px">
              <div class="bar layer-bar" class:sel={selection?.kind === "layer" && selection.id === layer.id} class:muted={layer.muted}
                data-bar="layer:{layer.id}" style:left="{pct(span.from)}%" style:width="{Math.max(0.4, pct(span.to) - pct(span.from))}%"
                title={bin?.video(layer.source)?.name ?? "video"}>
                <span class="edge" data-edge="start"></span>
                <span class="bar-label">{#if layer.muted}{@render speaker(true)}{/if}{bin?.video(layer.source)?.name ?? "video"}</span>
                <span class="edge end" data-edge="end"></span>
              </div>
            </div>
          {/each}

          <div class="lane" style:top="{lanes.overlays}px" style:height="{lanes.overlayHeight}px">
            {#each doc.overlays as o, i (o.id)}
              {@const bar = barPct(o.start, o.end)}
              <div class="bar ov-bar" class:sel={selOverlay?.id === o.id} class:image={o.content.kind === "image"}
                data-bar="overlay:{o.id}" style:left="{bar.left}%" style:width="{bar.width}%" style:top="{overlayRows[i] * (LANE.row + 1) + 1}px"
                title={overlayLabel(o)}>
                <span class="edge" data-edge="start"></span>
                <span class="bar-label">{overlayLabel(o)}</span>
                {#each thinMarks(keyframeMarks(o).map((k) => withinBar(bar, k)), Math.max(2, 120 / Math.max(1, bar.width))) as k, j (j)}<span class="key" style:left="{k}%"></span>{/each}
                <span class="edge end" data-edge="end"></span>
              </div>
            {/each}
          </div>

          <div class="lane" style:top="{lanes.audio}px" style:height="{lanes.audioHeight}px">
            {#each doc.audio as a, i (a.id)}
              {@const bar = barPct(a.start, a.start + a.length)}
              <div class="bar au-bar" class:sel={selAudio?.id === a.id} class:music={a.kind === "music"} class:muted={a.muted}
                data-bar="audio:{a.id}" style:left="{bar.left}%" style:width="{bar.width}%" style:top="{audioRows[i] * (LANE.row + 1) + 1}px"
                title={audioLabel(a)}>
                <span class="edge" data-edge="start"></span>
                <span class="bar-label">{#if a.muted}{@render speaker(true)}{/if}{audioLabel(a)}</span>
                <span class="edge end" data-edge="end"></span>
              </div>
            {/each}
          </div>

          <!-- What is not in the clip is dimmed across every lane, not just V1. -->
          {#each gaps as g, i (i)}
            <div class="gap" class:removed={g.removed} style:top="{lanes.v1 + LANE.v1 + LANE.gap}px" style:left="{pct(g.from)}%" style:width="{pct(g.to) - pct(g.from)}%" aria-hidden="true"></div>
          {/each}
          <div class="playhead" style:left="{pct(playhead)}%" aria-hidden="true"></div>
          {#if bladeMode && bladeAt !== null}
            <div class="blade-guide" class:refused={!canSplitAt(edit, bladeAt)} style:left="{pct(bladeAt)}%" aria-hidden="true"></div>
          {/if}
        </div>
        {#if phase === "exporting"}
          <div class="busy" role="status">
            <span class="micro">{composed ? "rendering" : "re-encoding"}</span>
            <span class="bar-progress" role="progressbar" aria-label="Export progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(progress * 100)}><span style:width="{progress * 100}%"></span></span>
            <span class="mono strong">{Math.round(progress * 100)}%</span>
            <button type="button" class="st-btn" data-action="cancel-export" title="Cancel (Esc)" onclick={cancelExport}>cancel</button>
          </div>
        {/if}
      </div>
      {#if phase === "failed"}
        <p class="alert" role="alert" title={messageTip || undefined}>{message}</p>
      {/if}
    {:else}
      <div class="transport placeholder" aria-hidden="true"></div>
      <div class="timeline placeholder" aria-hidden="true"></div>
    {/if}

    <footer class="foot" inert={confirmingDiscard}>
      {#if edit && phase !== "unavailable"}
        {#if kept.length === 0}
          <span class="ve-chip warn"><i></i><span class="micro">nothing kept</span></span>
        {:else if untouched}
          <span class="ve-chip"><i></i><span class="micro">unedited</span></span>
        {:else if unedited}
          <span class="ve-chip ok"><i></i><span class="micro">full clip {formatClock(keptSeconds)}</span></span>
        {:else}
          <span class="ve-chip ok" title={removedCount ? `${removedCount} ${removedCount === 1 ? "piece" : "pieces"} removed` : undefined}><i></i><span class="micro">keeps {formatClock(keptSeconds)} of {formatClock(duration)}</span></span>
        {/if}
        {#if counts && counts.overlays}<span class="ve-chip count" title="Text and stickers"><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M3 4V2.5h10V4M8 2.5v11"></path></svg><span class="micro">{counts.overlays}</span></span>{/if}
        {#if counts && counts.audio}<span class="ve-chip count" title="Music and sounds"><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M6 12V3.5l7-1.5v8.5"></path></svg><span class="micro">{counts.audio}</span></span>{/if}
        {#if counts && counts.layers > 1}<span class="ve-chip count" title="Videos"><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M2 4.5h8.5v7H2zM10.5 7l3.5-2v6l-3.5-2z"></path></svg><span class="micro">{counts.layers}</span></span>{/if}
        {#if notice}<span class="ve-chip warn notice" role="status" title={notice}><i></i><span class="note-text">{notice}</span></span>{/if}
      {/if}
      <span class="grow"></span>
      {#if phase === "failed"}
        <button type="button" class="st-btn ghost" data-action="back-to-edit" onclick={backToEditing}>back to editing</button>
      {/if}
      <!-- Posting waits while something is still being added; this gives up on it instead. -->
      {#if adding > 0 && phase === "ready"}
        <button type="button" class="st-btn ghost" data-action="stop-adding" onclick={stopAdding}>stop adding</button>
      {/if}
      <!-- One action, as the user asked: post (or embed, or upload) what is on the timeline. An
           untouched clip goes up as its original bytes, with no re-encode. Leaving is the ×/Esc,
           which asks first when there is work to lose. -->
      {#if phase === "unavailable" || phase === "failed"}
        <button type="button" class="st-btn primary" data-action="upload-original" disabled={originalTooBig}
          title={originalTooBig ? `Over this server's ${formatBytes(capBytes)} limit` : `${words.Verb} the original video`}
          onclick={sendOriginal}>{phase === "failed" ? `${words.action} original` : words.action}</button>
      {:else}
        <button type="button" class="st-btn primary" data-action="upload" disabled={uploadDisabled} title={uploadTitle} onclick={() => upload()}>
          {#if words.posts}<svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M2 8h10M8.5 4.5 12 8l-3.5 3.5"></path></svg>{/if}{words.action}</button>
      {/if}
    </footer>

    {#if confirmingDiscard}
      <!-- Inside the card, so the dialog's focus trap and key isolation still hold; no browser
           confirm(), which would block the whole window. -->
      <div class="ve-confirm" role="alertdialog" aria-modal="true" aria-label="Discard this clip?" bind:this={confirmBox}>
        <div class="ve-confirm-box">
          <h3>Discard this clip?</h3>
          <div class="ve-confirm-actions">
            <button type="button" class="st-btn" data-action="keep-editing" bind:this={keepButton} onclick={() => (confirmingDiscard = false)}>keep editing</button>
            <button type="button" class="st-btn danger" data-action="discard" onclick={() => finish({ kind: "skip" })}>discard</button>
          </div>
        </div>
      </div>
    {/if}
  </div>
</div>

{#snippet speaker(off: boolean)}
  {#if off}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 6l3 4M14 6l-3 4"></path></svg>
  {:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4"></path></svg>{/if}
{/snippet}

<style>
  /* Overlay and card. z-index 59: above ordinary overlays (50), and just BELOW the call bar, the
     incoming-call prompt and the floating call stage (all 60), so an incoming call stays visible
     and answerable over an editor. Those are small fixed pills at the top and bottom centre and
     cover no editor control. The overlay starts below the app's own title bar, like the other
     full-window layers, so the card is never clipped by it at the native 900×640 size. */
  .video-editor { position: fixed; inset: var(--titlebar-h, 0px) 0 0; z-index: 59; display: flex; align-items: center; justify-content: center; padding: 16px; background: color-mix(in oklab, var(--bg-0) 60%, transparent); color: var(--text); font-family: var(--ui); font-size: 0.84rem; }
  .card { position: relative; display: flex; flex-direction: column; width: min(900px, 100%); height: min(640px, 100%); background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 18px 44px color-mix(in oklab, var(--bg-0) 60%, transparent); overflow: hidden; outline: none; }
  .card:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .grow { flex: 1; }

  /* The editor's control grammar, shared with its panels (ClipLibrary, ClipInspector), which is
     why these are global under the editor's root. Operator-terminal style, as in the Studio. */
  .video-editor :global(.micro) { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .video-editor :global(.mono) { font-family: var(--mono); font-size: 0.7rem; color: var(--text-2); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .mono.strong { color: var(--text); min-width: 4.6em; }
  /* `ve-` prefixed where app.css has global rules of the same name (`.chip`, `.stage`). */
  .video-editor :global(.ve-chip) { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; flex: none; background: var(--panel); }
  .video-editor :global(.ve-chip i) { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .ve-chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); }
  .ve-chip.ok .micro { color: var(--ok); }
  .ve-chip.ok i { background: var(--ok); }
  .ve-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); }
  .ve-chip.warn .micro, .ve-chip.warn .note-text { color: var(--warn); }
  .ve-chip.warn i { background: var(--warn); }
  .ve-chip.count { gap: 4px; padding: 2px 8px; }
  .ve-chip.count svg { width: 11px; height: 11px; color: var(--muted); }
  .ve-chip.notice { min-width: 0; flex: 0 1 auto; overflow: hidden; }
  .note-text { font-size: 0.74rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .rec-chip { border-color: var(--danger-brd); background: var(--danger-dim); margin-left: 4px; }
  .rec-chip i { background: var(--danger) !important; animation: rec-pulse 1.2s ease-in-out infinite; }
  .rec-chip .micro { color: var(--danger-hi); font-weight: 600; }
  @keyframes rec-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.3; } }
  @media (prefers-reduced-motion: reduce) { .rec-chip i { animation: none; } }
  .video-editor :global(.st-btn) { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; cursor: pointer; flex: none; }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn.ghost.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); padding: 0 16px; }
  .st-btn svg.ol { width: 12px; height: 12px; margin-left: -4px; }
  .video-editor :global(.st-btn:disabled) { opacity: 0.45; cursor: default; }
  .video-editor :global(.st-btn svg) { width: 10px; height: 10px; }
  .st-btn.x { width: 26px; padding: 0; justify-content: center; }
  .st-btn.x svg { width: 12px; height: 12px; margin: 0; }
  .video-editor :global(.st-tool) { width: 28px; height: 28px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: inline-flex; align-items: center; justify-content: center; position: relative; padding: 0; flex: none; cursor: pointer; }
  .video-editor :global(.st-tool:hover:not(:disabled)) { color: var(--text); border-color: var(--border); }
  .video-editor :global(.st-tool svg) { width: 11px; height: 11px; }
  .video-editor :global(.st-tool.sm) { width: 24px; height: 24px; }
  .video-editor :global(.st-tool.tiny) { width: 20px; height: 20px; }
  .video-editor :global(.st-tool.tiny svg) { width: 12px !important; height: 12px !important; }
  .video-editor :global(.st-tool.on) { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tool.danger:hover:not(:disabled) { color: var(--danger-hi); border-color: var(--danger-brd); background: var(--danger-dim); }
  .video-editor :global(.st-tool:disabled) { opacity: 0.4; cursor: default; }
  .st-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin-right: 2px; }
  .st-tool.play svg { width: 14px; height: 14px; }
  .st-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  .video-editor :global(svg.solid) { fill: currentColor; stroke: none; }
  .video-editor :global(.st-tool.ol svg), .video-editor :global(svg.ol) { fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; width: 14px; height: 14px; }
  .rec-tool .dot { fill: var(--danger); stroke: none; }
  .rec-tool.recording { border-color: var(--danger); background: var(--danger-dim); color: var(--danger-hi); }

  /* Head row */
  .head { display: flex; align-items: center; gap: 10px; padding: 0 10px 0 14px; height: 38px; flex: none; box-sizing: border-box; border-bottom: 1px solid var(--border); min-width: 0; }
  .film { width: 14px; height: 14px; flex: none; fill: none; stroke: var(--accent); stroke-width: 1.5; stroke-linejoin: round; }
  .title { margin: 0; font-size: 0.95rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }
  .facts { display: inline-flex; gap: 10px; min-width: 0; overflow: hidden; flex: 0 1 auto; }

  /* Stage, inspector and library */
  .middle { flex: 1; min-height: 0; display: flex; }
  .left { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .ve-stage { position: relative; flex: 1; min-height: 0; background: var(--bg-0); overflow: hidden; touch-action: none; user-select: none; cursor: default; }
  .ve-stage.grab { cursor: grab; }
  .ve-stage.rec { box-shadow: inset 0 0 0 2px color-mix(in oklab, var(--danger) 85%, transparent); cursor: crosshair; }
  .stage-canvas { position: absolute; display: block; border-radius: 2px; background: #000; }
  .ve-stage-chip { position: absolute; left: 12px; top: 10px; }
  .paths { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; overflow: visible; }
  .path { fill: none; stroke: var(--text); stroke-width: 2.5; stroke-linecap: round; stroke-linejoin: round; stroke-dasharray: 0.1 6; opacity: 0.9; }
  .trail { fill: none; stroke: var(--danger); stroke-width: 2.5; stroke-linecap: round; stroke-linejoin: round; }
  .sel { position: absolute; box-sizing: border-box; border: 1.5px solid var(--accent-hi); border-radius: 3px; pointer-events: none; }
  .sel .corner { position: absolute; width: 7px; height: 7px; box-sizing: border-box; background: var(--text); border: 1px solid var(--accent); }
  .corner.nw { left: -5px; top: -5px; } .corner.ne { right: -5px; top: -5px; } .corner.sw { left: -5px; bottom: -5px; }
  .turn { position: absolute; right: -10px; bottom: -10px; width: 17px; height: 17px; box-sizing: border-box; border-radius: 50%; background: var(--accent-hi); border: 1.5px solid var(--bg-0); color: var(--bg-0); display: flex; align-items: center; justify-content: center; pointer-events: auto; cursor: nwse-resize; }
  .turn svg { width: 10px; height: 10px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  .sel.box .resize { position: absolute; width: 9px; height: 9px; box-sizing: border-box; background: var(--text); border: 1px solid var(--accent); pointer-events: auto; }
  .resize.nw { left: -5px; top: -5px; cursor: nwse-resize; } .resize.ne { right: -5px; top: -5px; cursor: nesw-resize; }
  .resize.sw { left: -5px; bottom: -5px; cursor: nesw-resize; } .resize.se { right: -5px; bottom: -5px; cursor: nwse-resize; }
  .pool { position: absolute; width: 1px; height: 1px; overflow: hidden; opacity: 0; pointer-events: none; left: 0; top: 0; }
  .pool video { width: 1px; height: 1px; }

  /* Transport and tools */
  .transport { display: flex; align-items: center; gap: 6px; padding: 0 14px; height: 40px; flex: none; border-top: 1px solid var(--border); box-sizing: border-box; }
  .transport .micro { color: var(--muted); }
  .sep { width: 1px; height: 18px; background: var(--border); margin: 0 4px; flex: none; }

  /* Timeline: the ruler over the lanes. Every lane's positions are percentages of the same width
     (V1's track), so nothing depends on a measured width and one pointer mapping serves them all. */
  /* Lanes grow a row for every overlapping bar, so the timeline's height is capped and it scrolls
     past that (found in review: overlapping text and stickers, up to 24 rows, and sounds, up to 32,
     pushed the stage and the footer, with the post button, out of the 640 px card). The ruler
     scrolls with the lanes so the two always share one width, scrollbar or not. */
  .timeline { position: relative; flex: none; max-height: min(220px, 35%); overflow-x: hidden; overflow-y: auto; scrollbar-width: thin; padding: 2px 14px 8px; border-top: 1px solid var(--border-soft); background: color-mix(in oklab, var(--bg-0) 35%, var(--panel)); user-select: none; touch-action: none; outline: none; }
  .timeline.exporting { overflow-y: hidden; }
  .timeline:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
  .timeline.placeholder, .transport.placeholder { min-height: 40px; }
  .timeline.placeholder { min-height: 120px; }
  .ruler { position: relative; height: 16px; margin: 0 0 4px 52px; }
  .tick { position: absolute; bottom: 0; height: 6px; border-left: 1px solid var(--border); }
  .tick .micro { position: absolute; bottom: 7px; left: 3px; font-size: 0.56rem; letter-spacing: 0.06em; }
  .tick.last .micro { left: auto; right: 3px; }
  .head-mark { position: absolute; bottom: -1px; width: 0; height: 0; margin-left: -5px; border-left: 5px solid transparent; border-right: 5px solid transparent; border-top: 7px solid var(--accent-hi); pointer-events: none; z-index: 2; }
  .lanes { position: relative; margin-left: 52px; }
  .lane-head { position: absolute; left: -52px; width: 46px; display: flex; align-items: center; justify-content: space-between; }
  .lane-name { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.08em; color: var(--text-2); }
  .track { position: absolute; left: 0; right: 0; height: 40px; border: 1px solid var(--border); border-radius: 3px; background: var(--bg-0); cursor: text; box-sizing: border-box; }
  .lane { position: absolute; left: 0; right: 0; border: 1px solid var(--border-soft); border-radius: 3px; background: var(--bg-0); box-sizing: border-box; cursor: text; }
  .timeline.blade .track, .timeline.blade .lane { cursor: crosshair; }
  .timeline.locked .track, .timeline.locked .lane { cursor: default; }
  .strip { position: absolute; inset: 0; display: flex; overflow: hidden; border-radius: 2px; pointer-events: none; }
  .strip canvas { flex: 1 1 0; min-width: 0; height: 100%; display: block; object-fit: cover; opacity: 0.88; border-right: 1px solid color-mix(in oklab, var(--bg-0) 70%, transparent); }

  /* Pieces: kept ones are clear, removed ones are hatched and dimmed, the selected one is ringed. */
  .piece { position: absolute; top: 0; bottom: 0; pointer-events: none; box-sizing: border-box; }
  .piece.removed { background:
      repeating-linear-gradient(135deg, color-mix(in oklab, var(--danger) 55%, transparent) 0 2px, transparent 2px 8px),
      color-mix(in oklab, var(--bg-0) 62%, transparent); }
  .piece.selected { box-shadow: inset 0 0 0 2px var(--accent); }
  .piece.removed.selected { box-shadow: inset 0 0 0 2px var(--danger); }
  .trimmed { position: absolute; top: 0; bottom: 0; background: color-mix(in oklab, var(--bg-0) 82%, transparent); pointer-events: none; }
  .gap { position: absolute; bottom: 0; background: color-mix(in oklab, var(--bg-0) 60%, transparent); pointer-events: none; z-index: 2; }
  .gap.removed { background: repeating-linear-gradient(135deg, color-mix(in oklab, var(--danger) 20%, transparent) 0 2px, transparent 2px 8px), color-mix(in oklab, var(--bg-0) 45%, transparent); }

  /* Cuts: a hairline with a knob above the strip to grab. */
  .cut { position: absolute; top: -6px; bottom: 0; width: 13px; margin-left: -6px; cursor: ew-resize; z-index: 3; }
  .cut::before { content: ""; position: absolute; left: 6px; top: 6px; bottom: 0; width: 1px; background: var(--text); box-shadow: 0 0 0 1px color-mix(in oklab, var(--bg-0) 55%, transparent); }
  .knob { position: absolute; left: 2px; top: 0; width: 9px; height: 9px; border-radius: 2px; background: var(--text); border: 1px solid var(--bg-0); box-sizing: border-box; }
  .cut:hover .knob { background: var(--accent-hi); }

  /* Trim handles: brackets that sit inside the kept span so they stay grabbable at either end. */
  .handle { position: absolute; top: -1px; bottom: -1px; width: 9px; cursor: ew-resize; z-index: 4; box-sizing: border-box; border: 2px solid var(--accent); background: color-mix(in oklab, var(--accent) 22%, transparent); }
  .handle.in { border-right: none; border-radius: 3px 0 0 3px; }
  .handle.out { margin-left: -9px; border-left: none; border-radius: 0 3px 3px 0; }
  .handle:hover { background: color-mix(in oklab, var(--accent) 45%, transparent); }

  /* Bars: text, stickers, sounds and added videos. Drag the middle to move, an end to trim. */
  .bar { position: absolute; height: 12px; box-sizing: border-box; border: 1px solid var(--border); border-radius: 2px; background: var(--bg-elev); color: var(--text-2); display: flex; align-items: center; overflow: hidden; cursor: grab; font-size: 0.6rem; line-height: 1; white-space: nowrap; z-index: 1; }
  .layer-bar { top: 1px; bottom: 1px; height: auto; background: linear-gradient(90deg, color-mix(in oklab, var(--accent) 30%, var(--bg-elev)), var(--bg-elev)); }
  .bar.music { background: repeating-linear-gradient(90deg, color-mix(in oklab, var(--accent) 22%, var(--bg-elev)) 0 2px, var(--bg-elev) 2px 5px); }
  .bar.muted { opacity: 0.55; }
  .bar.sel { border-color: var(--accent-hi); background-color: color-mix(in oklab, var(--accent) 30%, var(--bg-elev)); color: var(--text); box-shadow: 0 0 0 1px var(--accent); z-index: 3; }
  .bar-label { padding: 0 6px; overflow: hidden; text-overflow: ellipsis; pointer-events: none; display: inline-flex; align-items: center; gap: 3px; }
  .bar-label svg { width: 8px; height: 8px; fill: none; stroke: currentColor; stroke-width: 2; }
  .ov-bar .bar-label { font-weight: 700; }
  .edge { position: absolute; top: 0; bottom: 0; left: 0; width: 6px; cursor: ew-resize; z-index: 2; }
  .edge.end { left: auto; right: 0; }
  .key { position: absolute; top: 2.5px; width: 5px; height: 5px; margin-left: -2.5px; transform: rotate(45deg); background: var(--muted); box-shadow: 0 0 0 1px var(--bg-0); pointer-events: none; }
  .bar.sel .key { background: var(--accent-hi); }

  .playhead { position: absolute; top: -3px; bottom: -3px; width: 2px; margin-left: -1px; background: var(--accent-hi); box-shadow: 0 0 0 1px color-mix(in oklab, var(--bg-0) 50%, transparent); pointer-events: none; z-index: 5; }
  .blade-guide { position: absolute; top: -4px; height: 44px; width: 0; margin-left: -1px; border-left: 2px dashed var(--warn); pointer-events: none; z-index: 6; }
  .blade-guide.refused { border-left-color: var(--faint); }

  /* Export progress replaces nothing: it sits over the timeline, which is locked meanwhile. */
  .busy { position: absolute; inset: 0; display: flex; align-items: center; gap: 10px; padding: 0 21px; background: color-mix(in oklab, var(--panel) 88%, transparent); z-index: 10; }
  .busy .micro { color: var(--text-2); }
  .bar-progress { flex: 1; height: 6px; border-radius: 999px; background: var(--bg-0); border: 1px solid var(--border); overflow: hidden; }
  .bar-progress span { display: block; height: 100%; background: var(--accent); }

  .alert { margin: 0; padding: 8px 14px; border-top: 1px solid var(--warn-brd); background: var(--warn-dim); color: var(--warn); font-size: 0.78rem; }

  /* "Discard this clip?": a small panel over the dimmed card, inside it so focus stays trapped. */
  .ve-confirm { position: absolute; inset: 0; z-index: 20; display: flex; align-items: center; justify-content: center; background: color-mix(in oklab, var(--bg-0) 62%, transparent); }
  .ve-confirm-box { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px; min-width: 280px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 12px 32px color-mix(in oklab, var(--bg-0) 60%, transparent); }
  .ve-confirm-box h3 { margin: 0; font-size: 0.95rem; font-weight: 600; color: var(--text); }
  .ve-confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .st-btn.danger { background: var(--danger-dim); border-color: var(--danger-brd); color: var(--danger-hi); }

  /* Footer */
  .foot { display: flex; align-items: center; gap: 8px; padding: 0 14px; height: 42px; flex: none; box-sizing: border-box; border-top: 1px solid var(--border); background: var(--bg-elev); min-width: 0; }

  /* Visually hidden button text: present for scripts and assistive tech, invisible on screen. */
  .vh { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
