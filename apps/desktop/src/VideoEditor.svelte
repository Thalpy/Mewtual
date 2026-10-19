<script module lang="ts">
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
   * The pre-upload video editor: a modal opened by the host for each video picked, dropped or
   * pasted into a composer, before anything is uploaded.
   *
   * The person can trim the ends (in/out handles), cut the clip with a blade, remove or restore
   * the pieces between cuts, and preview the result: playback jumps over everything that will not
   * be uploaded. The whole editing model lives in `video-edit.ts`; this component is presentation,
   * pointer and key handling, and the lifecycle around the probe and the export.
   *
   * Outcomes go to `onfinish` exactly once (see `VideoEditorResult`). There is one action button
   * (also Ctrl+Enter); its word depends on `context` ("post now", "embed", "upload"; see
   * `editorWords`), and in the chat and announcement composers its result carries `post`, so the
   * host sends straight away:
   * - on an untouched edit it hands back `original`, so not editing costs no quality and no
   *   re-encode;
   * - on a real edit it runs the export and hands back the new file as `edited`;
   * - the close button and Escape hand back `skip`, after asking "Discard this clip?" when there
   *   is work to lose (anything trimmed, removed or cut).
   *
   * The backdrop deliberately does not dismiss: a stray click outside the card would throw away
   * every cut. While an export runs, Escape cancels the export instead of closing.
   *
   * `file` is read once at mount; the host mounts a fresh instance for every file.
   */
  import { onDestroy, onMount, tick, untrack } from "svelte";
  import { formatBytes } from "./transfer-visual.ts";
  import {
    canSplitAt, formatClock, initialEdit, isUnedited, keptDuration, keptRanges, pieceIndexAt, pieces,
    playbackJump, pushEdit, redo, replaceEdit, rulerLabel, rulerTicks, setInPoint, setOutPoint, moveCut,
    sourceToOutput, splitAt, startHistory, toggleRemoved, undo, type EditHistory, type VideoEdit,
  } from "./video-edit.ts";
  import {
    ExportCanceledError, FILMSTRIP_MAX_TILES, OutputTooLargeError, exportEditedVideo, filmstrip, probeVideo,
    type VideoEditorContext, type VideoEditorResult,
  } from "./video-export.ts";
  import { editorWords } from "./video-editor-ui.ts";

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
    /** Seams for tests; the real implementations by default. */
    probe?: typeof probeVideo;
    exporter?: typeof exportEditedVideo;
    thumbnails?: typeof filmstrip;
  };
  let {
    file, capBytes, position, context, onfinish, passKey = () => false,
    probe = probeVideo, exporter = exportEditedVideo, thumbnails = filmstrip,
  }: Props = $props();

  /** Nudge sizes for the arrow keys and the step buttons, in seconds. */
  const NUDGE = 0.1;
  const NUDGE_LARGE = 1;
  /** Filmstrip tile height in CSS pixels; also the decode height. */
  const TILE_HEIGHT = 56;

  // --- State ------------------------------------------------------------------------------------
  type Phase = "probing" | "unavailable" | "ready" | "exporting" | "failed";
  let phase = $state<Phase>("probing");
  /** The probe's refusal (unavailable) or the export's failure (failed), shown as an alert. */
  let message = $state("");
  /** A tooltip with the way out of a failure, when there is one. */
  let messageTip = $state("");
  let duration = $state(0);
  let size = $state({ width: 0, height: 0 });
  /** The edit and its undo/redo stacks. Raw: every change replaces it whole. */
  let history = $state.raw<EditHistory | null>(null);
  /** The piece clicked last; Delete acts on it, or on the piece under the playhead when null. */
  let selected = $state<number | null>(null);
  let bladeMode = $state(false);
  /** Where the blade guide sits while the pointer is over the timeline in blade mode. */
  let bladeAt = $state<number | null>(null);
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
  /** Filmstrip tiles as they arrive; `null` until a tile is decoded. */
  let tiles = $state.raw<(CanvasImageSource | null)[]>([]);

  let root = $state<HTMLDivElement | null>(null);
  let card = $state<HTMLDivElement | null>(null);
  let videoEl = $state<HTMLVideoElement | null>(null);
  let trackEl = $state<HTMLDivElement | null>(null);

  // Plain bookkeeping, read only from handlers and callbacks.
  /** Set once `onfinish` has fired, or the component is gone; every late callback checks it. */
  let finished = false;
  let destroyed = false;
  let exportAbort: AbortController | null = null;
  const stripAbort = new AbortController();
  let frameHandle: number | null = null;
  /** The drag in progress, and whether it has already made its one undo step. */
  let drag: { kind: "in" | "out" | "scrub" | "cut"; cut: number; stepped: boolean } | null = null;
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
  const previewUrl = untrack(() => typeof URL.createObjectURL === "function" ? URL.createObjectURL(file) : "");

  // --- Derived view -----------------------------------------------------------------------------
  const edit = $derived<VideoEdit | null>(history?.present ?? null);
  const pieceList = $derived(edit ? pieces(edit) : []);
  const kept = $derived(edit ? keptRanges(edit) : []);
  const keptSeconds = $derived(edit ? keptDuration(edit) : 0);
  const unedited = $derived(edit ? isUnedited(edit) : true);
  const removedCount = $derived(edit ? edit.removed.filter((r) => r).length : 0);
  /** A selection that undo or a merge left pointing past the end is no selection. */
  const selectedPiece = $derived(edit && selected !== null && selected < edit.removed.length ? selected : null);
  /** The piece Delete would toggle. */
  const targetPiece = $derived(edit ? (selectedPiece ?? pieceIndexAt(edit, playhead)) : null);
  const targetRemoved = $derived(edit !== null && targetPiece !== null && edit.removed[targetPiece]);
  const ruler = $derived(rulerTicks(duration, 8));
  const editing = $derived(phase === "ready" && edit !== null);
  /** The original only goes up when it fits; the native side would refuse it anyway. */
  const originalTooBig = $derived(file.size > capBytes);
  const facts = $derived([
    size.width && size.height ? `${size.width}×${size.height}` : "",
    duration ? formatClock(duration) : "",
    formatBytes(file.size),
  ].filter(Boolean).join(" · "));
  const uploadDisabled = $derived(!editing || draining || kept.length === 0 || (unedited && originalTooBig));
  const words = $derived(editorWords(context));
  /** Work that closing would throw away: anything trimmed, removed, or cut. */
  const dirty = $derived(edit !== null && (!unedited || edit.cuts.length > 0));
  const uploadTitle = $derived(
    kept.length === 0 ? "Nothing is kept"
      : draining ? "Finishing the cancelled export"
      : unedited && originalTooBig ? `Over this server's ${formatBytes(capBytes)} limit: trim it to fit`
      : unedited ? `${words.Verb} the original video`
      : `${words.Verb} the edited video (re-encoded)`,
  );

  // --- Lifecycle --------------------------------------------------------------------------------
  onMount(() => {
    // The dialog takes focus so Escape, Space and the editing keys work without a click first.
    card?.focus();
    // Focus that lands anywhere outside the dialog is brought back. The keys are only kept from
    // the app behind it while focus is inside (see `onKey`); without this, a click on the title
    // bar or anything else focusable outside the overlay would hand the app's shortcuts back.
    document.addEventListener("focusin", keepFocus);
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
    exportAbort?.abort(); exportAbort = null;
    stopLoop();
    if (videoEl) { videoEl.pause(); videoEl.removeAttribute("src"); videoEl.load(); }
    if (previewUrl) URL.revokeObjectURL(previewUrl);
    // Give focus back to where the person was (the composer, normally) if it still exists, but
    // only if focus is still ours to give: the next queued editor may already hold it.
    const here = document.activeElement;
    const ours = !here || here === document.body || (root?.contains(here) ?? false);
    if (ours && previousFocus && previousFocus.isConnected && typeof previousFocus.focus === "function") previousFocus.focus();
  });

  /**
   * Focus that lands outside the dialog is brought back, with two exceptions: once this editor has
   * answered it is on its way out and holds nothing, and focus moving into another editor is the
   * host's next queued review taking over. Without both, the outgoing and incoming editors would
   * pull focus back and forth while they briefly share the page.
   */
  function keepFocus(e: FocusEvent) {
    const target = e.target as Element | null;
    if (finished || destroyed || !root || !card || !target || root.contains(target)) return;
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
    history = startHistory(initialEdit(result.duration));
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
    pause();
    onfinish(result);
  }

  // --- Editing ----------------------------------------------------------------------------------
  /**
   * Record a discrete edit as one undo step. A refused operation returns the same edit: no step.
   *
   * Any discrete edit, undo or redo ends a drag in progress. The drag's later moves would
   * otherwise `replaceEdit` over the new present (undoing the undo while leaving the stacks
   * claiming otherwise) or move a cut whose index the edit just renumbered.
   */
  function apply(next: VideoEdit) {
    if (!history || !editing) return;
    drag = null;
    const before = history.present;
    history = pushEdit(history, next);
    // A split or reset renumbers the pieces after it; a selection would silently move to another.
    if (next.removed.length !== before.removed.length) selected = null;
  }
  function splitHere() { if (edit) apply(splitAt(edit, playhead)); }
  function toggleTarget() { if (edit && targetPiece !== null) apply(toggleRemoved(edit, targetPiece)); }
  function setInHere() { if (edit) apply(setInPoint(edit, playhead)); }
  function setOutHere() { if (edit) apply(setOutPoint(edit, playhead)); }
  function reset() { if (duration > 0) apply(initialEdit(duration)); }
  function undoStep() { if (history && editing) { drag = null; history = undo(history); selected = null; } }
  function redoStep() { if (history && editing) { drag = null; history = redo(history); selected = null; } }

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
    if (!edit || video.paused) return;
    const jump = playbackJump(edit, t);
    if (!jump) return;
    if ("seek" in jump) { video.currentTime = jump.seek; playhead = jump.seek; return; }
    // Nothing kept after here: stop at the end of the last kept range.
    video.pause();
    const end = kept.length ? kept[kept.length - 1].end : t;
    video.currentTime = end; playhead = end;
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
  }
  function nudge(delta: number) { pause(); seek(playhead + delta); }
  function onVideoPlay() { playing = true; stopLoop(); frameHandle = requestAnimationFrame(loop); }
  function onVideoPause() { playing = false; stopLoop(); if (videoEl) playhead = videoEl.currentTime; }

  // --- Timeline pointer handling ----------------------------------------------------------------
  /** Source seconds under a pointer, from the track's live geometry (never a cached width). */
  function timeAt(e: PointerEvent): number {
    const rect = trackEl?.getBoundingClientRect();
    if (!rect || rect.width <= 0 || duration <= 0) return 0;
    return Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)) * duration;
  }
  function onTimelineDown(e: PointerEvent) {
    if (!editing || !edit || e.button !== 0) return;
    const t = timeAt(e);
    const target = e.target as Element | null;
    const handle = target?.closest("[data-handle]")?.getAttribute("data-handle");
    const cut = target?.closest("[data-cut]")?.getAttribute("data-cut");
    e.preventDefault();
    if (handle === "in" || handle === "out") {
      drag = { kind: handle, cut: -1, stepped: false };
    } else if (cut !== null && cut !== undefined) {
      drag = { kind: "cut", cut: Number(cut), stepped: false };
    } else if (bladeMode) {
      // The blade cuts where it is, and that is the whole gesture: no drag follows.
      apply(splitAt(edit, t));
      return;
    } else {
      drag = { kind: "scrub", cut: -1, stepped: false };
      pause();
      seek(t);
      selected = pieceIndexAt(edit, t);
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
    const present = history.present;
    const next = drag.kind === "in" ? setInPoint(present, t)
      : drag.kind === "out" ? setOutPoint(present, t)
      : moveCut(present, drag.cut, t);
    if (next === present) return;
    // One drag is one undo step: the first real change pushes, every later one replaces it.
    history = drag.stepped ? replaceEdit(history, next) : pushEdit(history, next);
    drag.stepped = true;
    // Show the frame at what is being moved, so a trim lands on the picture the person wants.
    seek(drag.kind === "in" ? next.inPoint : drag.kind === "out" ? next.outPoint : next.cuts[drag.cut]);
  }
  /** Release, cancel and a lost pointer capture all end the drag. */
  function onTimelineUp(e: PointerEvent) {
    if (!drag) return;
    drag = null;
    const el = e.currentTarget as Element;
    if (el.hasPointerCapture?.(e.pointerId)) el.releasePointerCapture?.(e.pointerId);
  }

  // --- Export -----------------------------------------------------------------------------------
  /**
   * The one action: post (embed, upload) what the edit keeps. Where it posts (chat, announcements)
   * the result carries `post`, and the host sends straight away. An untouched edit goes up as the
   * original bytes, with no re-encode.
   */
  async function upload() {
    if (!edit || !editing || uploadDisabled) return;
    const post = words.posts;
    if (unedited) { finish(post ? { kind: "original", post: true } : { kind: "original" }); return; }
    pause();
    bladeMode = false; bladeAt = null;
    const controller = new AbortController();
    exportAbort = controller;
    progress = 0;
    phase = "exporting";
    const running = exporter(file, edit, {
      capBytes,
      signal: controller.signal,
      onProgress: (fraction) => { if (exportAbort === controller) progress = fraction; },
    });
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
        // The exporter's own errors are sentences meant for the person; anything else it has
        // already replaced with one (see `exportEditedVideo`).
        message = error instanceof Error && error.message ? error.message : "The edited video could not be made.";
        messageTip = "";
      }
      phase = "failed";
    }
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
    if (originalTooBig || finished) return;
    finish(words.posts ? { kind: "original", post: true } : { kind: "original" });
  }
  /**
   * The ×, and Escape once nothing else claims it. During an export it cancels the export rather
   * than leaving; with work on the timeline it asks before throwing that away; otherwise it leaves.
   */
  function closeRequest() {
    if (phase === "exporting") { cancelExport(); return; }
    if (dirty && (phase === "ready" || phase === "failed")) { askDiscard(); return; }
    finish({ kind: "skip" });
  }
  async function askDiscard() {
    confirmingDiscard = true;
    // The safe answer takes focus, so a stray Enter keeps the work.
    await tick();
    keepButton?.focus();
  }
  function backToEditing() { if (phase === "failed") { message = ""; messageTip = ""; phase = "ready"; } }

  // --- Keyboard ---------------------------------------------------------------------------------
  function onKey(e: KeyboardEvent) {
    // A few keys belong to the app even over a modal (Lock, push-to-talk): those pass untouched.
    if (passKey(e)) {
      // A plain passed key (push-to-talk) must not also activate the focused editor button the
      // way Space or Enter would. App's handlers do not consult `defaultPrevented`.
      if (!e.ctrlKey && !e.metaKey && !e.altKey) e.preventDefault();
      return;
    }
    // The editor is modal: no other key reaches the app's own shortcuts behind it.
    e.stopPropagation();
    if (e.key === "Tab") { trapTab(e); return; }
    if (e.key === "Escape") {
      e.preventDefault();
      if (confirmingDiscard) { confirmingDiscard = false; card?.focus(); }
      else if (phase === "exporting") cancelExport();
      else if (bladeMode) { bladeMode = false; bladeAt = null; }
      else closeRequest();
      return;
    }
    // While the discard question is up, only its buttons act (Enter and Space press the focused
    // one natively); no editing key may change the clip behind it.
    if (confirmingDiscard) return;
    const target = e.target as HTMLElement | null;
    const onButton = !!target && (target.tagName === "BUTTON" || target.tagName === "INPUT");
    const mod = e.ctrlKey || e.metaKey;
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
    else if (key === "b") { bladeMode = !bladeMode; if (!bladeMode) bladeAt = null; }
    else if (key === "s") splitHere();
    else if (key === "i") setInHere();
    else if (key === "o") setOutHere();
    else if (key === "m") muted = !muted;
    else if (key === "Delete" || key === "Backspace") toggleTarget();
    else handled = false;
    if (handled) e.preventDefault();
  }
  /**
   * Keep Tab inside the dialog: `aria-modal` promises that nothing behind the card is reachable.
   * Only the wrap-around edges are handled; in between, the natural order stays inside the card.
   */
  function trapTab(e: KeyboardEvent) {
    if (!card) return;
    const focusable = [...card.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')];
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
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- The app's own context menus would open above the dialog and lose their focus to `keepFocus`,
     leaving Esc to close the editor instead of the menu: the editor has no context menu at all. -->
<div class="video-editor" data-video-editor role="presentation" bind:this={root} onkeydown={onKey} onmousedown={onBackdropDown}
  onfocusout={onFocusOut} oncontextmenu={(e) => e.preventDefault()}>
  <div class="card" role="dialog" aria-modal="true" aria-label="Edit video before upload" tabindex="-1" data-state={phase} bind:this={card}>
    <header class="head">
      <svg class="film" viewBox="0 0 16 16" aria-hidden="true"><rect x="1.5" y="3" width="13" height="10" rx="1"></rect><path d="M4.5 3v10M11.5 3v10M1.5 6.5h3M11.5 6.5h3M1.5 9.5h3M11.5 9.5h3"></path></svg>
      <h2 class="title" title={file.name}>{file.name}</h2>
      {#if facts}<span class="micro facts" title={facts}>{facts}</span>{/if}
      <span class="grow"></span>
      {#if position && position.count > 1}<span class="micro">{position.index} of {position.count}</span>{/if}
      <button type="button" class="st-btn ghost x" data-action="close" aria-label={phase === "exporting" ? "Cancel export" : "Don't upload"} title={phase === "exporting" ? "Cancel export (Esc)" : "Don't upload (Esc)"} onclick={closeRequest}><svg viewBox="0 0 16 16" class="ol" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8"></path></svg></button>
    </header>

    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_noninteractive_element_interactions -->
    <div class="ve-stage" onclick={() => { if (phase !== "exporting") togglePlay(); }}>
      <!-- svelte-ignore a11y_media_has_caption -->
      <video bind:this={videoEl} src={previewUrl || undefined} preload="auto" playsinline bind:muted
        onplay={onVideoPlay} onpause={onVideoPause} onended={onVideoPause} ontimeupdate={follow} onseeked={() => { if (videoEl && !playing) playhead = videoEl.currentTime; }}></video>
      {#if phase === "probing"}<span class="ve-chip ve-stage-chip" role="status"><i></i><span class="micro">reading video</span></span>{/if}
    </div>

    {#if phase === "unavailable"}
      <p class="alert" role="alert">{message}</p>
    {:else if edit}
      <div class="transport">
        <!-- The visible text is exactly "Play" or "Pause": scripts and screen readers rely on it. -->
        <button type="button" class="st-tool play" class:on={playing} data-action="play" disabled={phase === "exporting"} title={playing ? "Pause (Space)" : "Play (Space)"} onclick={togglePlay}>{#if playing}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}<span class="vh">{playing ? "Pause" : "Play"}</span></button>
        <button type="button" class="st-tool sm" data-action="back" disabled={!editing} title="Back 0.1 s (Left, Shift for 1 s)" onclick={() => nudge(-NUDGE)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg><span class="vh">Back</span></button>
        <button type="button" class="st-tool sm" data-action="forward" disabled={!editing} title="Forward 0.1 s (Right, Shift for 1 s)" onclick={() => nudge(NUDGE)}><svg viewBox="0 0 16 16" class="solid" aria-hidden="true"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg><span class="vh">Forward</span></button>
        <span class="mono strong" title="Playhead in the original">{formatClock(playhead)}</span>
        <span class="micro" title="Position in the upload / length of the upload">{formatClock(sourceToOutput(edit, playhead))} / {formatClock(keptSeconds)}</span>

        <span class="sep" aria-hidden="true"></span>
        <button type="button" class="st-tool ol" class:on={bladeMode} data-action="blade" aria-pressed={bladeMode} disabled={!editing} title={bladeMode ? "Blade: on, click the timeline to cut (B, Esc to leave)" : "Blade (B)"} onclick={() => { bladeMode = !bladeMode; if (!bladeMode) bladeAt = null; }}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 1.5 14.5 6.5 6 15 1 10z"></path><path d="M4 12l2 2M9.5 1.5 7 4"></path></svg><span class="vh">Blade</span></button>
        <button type="button" class="st-tool ol" data-action="split" disabled={!editing || !canSplitAt(edit, playhead)} title="Split at playhead (S)" onclick={splitHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 1.5v13"></path><path d="M2 5h4v6H2M14 5h-4v6h4"></path></svg><span class="vh">Split</span></button>
        <button type="button" class="st-tool ol" class:danger={!targetRemoved} data-action="remove" disabled={!editing || targetPiece === null} title={targetRemoved ? "Restore piece (Delete)" : "Remove piece (Delete)"} onclick={toggleTarget}>{#if targetRemoved}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8a5 5 0 1 0 1.5-3.5"></path><path d="M3 2.5v3h3"></path></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.7 9h5.6l.7-9"></path></svg>{/if}<span class="vh">{targetRemoved ? "Restore piece" : "Remove piece"}</span></button>
        <button type="button" class="st-tool ol" data-action="set-in" disabled={!editing} title="Trim start to playhead (I)" onclick={setInHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6.5 2.5h-3v11h3"></path><path d="M9 8h5M11.5 5.5 14 8l-2.5 2.5"></path></svg><span class="vh">Trim start</span></button>
        <button type="button" class="st-tool ol" data-action="set-out" disabled={!editing} title="Trim end to playhead (O)" onclick={setOutHere}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 2.5h3v11h-3"></path><path d="M7 8H2M4.5 5.5 2 8l2.5 2.5"></path></svg><span class="vh">Trim end</span></button>

        <span class="sep" aria-hidden="true"></span>
        <button type="button" class="st-tool ol" data-action="undo" disabled={!editing || !history || history.past.length === 0} title="Undo (Ctrl+Z)" onclick={undoStep}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 3 2.5 6l3 3"></path><path d="M2.5 6h7a4 4 0 0 1 0 8H7"></path></svg><span class="vh">Undo</span></button>
        <button type="button" class="st-tool ol" data-action="redo" disabled={!editing || !history || history.future.length === 0} title="Redo (Ctrl+Shift+Z)" onclick={redoStep}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10.5 3l3 3-3 3"></path><path d="M13.5 6h-7a4 4 0 0 0 0 8H9"></path></svg><span class="vh">Redo</span></button>
        <button type="button" class="st-tool ol" data-action="reset" disabled={!editing || (unedited && edit.cuts.length === 0)} title="Reset all edits" onclick={reset}><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 8a5.5 5.5 0 1 0 1.8-4.1"></path><path d="M2.5 2v3.5H6"></path></svg><span class="vh">Reset</span></button>

        <span class="grow"></span>
        <button type="button" class="st-tool ol" class:on={muted} data-action="mute" aria-pressed={muted} title={muted ? "Sound off (M)" : "Sound on (M)"} onclick={() => (muted = !muted)}>{#if muted}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 6l3 4M14 6l-3 4"></path></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6h2.5l3.5-3v10L5 10H2.5z"></path><path d="M11 5.5a3.5 3.5 0 0 1 0 5M12.8 3.8a6 6 0 0 1 0 8.4"></path></svg>{/if}<span class="vh">{muted ? "Unmute" : "Mute"}</span></button>
      </div>

      <!-- The timeline is the playhead's slider for assistive tech: the arrow keys (handled on the
           overlay) move the playhead, and the value reads as a clock. Cuts and handles are pointer
           affordances; their keyboard equivalents are S, I, O and Delete. -->
      <div class="timeline" class:blade={bladeMode} class:locked={!editing} role="slider" tabindex="0" data-timeline
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
        <div class="track" bind:this={trackEl}>
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
          <div class="playhead" style:left="{pct(playhead)}%" aria-hidden="true"></div>
          {#if bladeMode && bladeAt !== null}
            <div class="blade-guide" class:refused={!canSplitAt(edit, bladeAt)} style:left="{pct(bladeAt)}%" aria-hidden="true"></div>
          {/if}
        </div>
        {#if phase === "exporting"}
          <div class="busy" role="status">
            <span class="micro">re-encoding</span>
            <span class="bar" role="progressbar" aria-label="Export progress" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(progress * 100)}><span style:width="{progress * 100}%"></span></span>
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

    <footer class="foot">
      {#if edit && phase !== "unavailable"}
        {#if kept.length === 0}
          <span class="ve-chip warn"><i></i><span class="micro">nothing kept</span></span>
        {:else if unedited}
          <span class="ve-chip"><i></i><span class="micro">unedited</span></span>
        {:else}
          <span class="ve-chip ok"><i></i><span class="micro">keeps {formatClock(keptSeconds)} of {formatClock(duration)}{removedCount ? ` · ${removedCount} removed` : ""}</span></span>
        {/if}
      {/if}
      <span class="grow"></span>
      {#if phase === "failed"}
        <button type="button" class="st-btn ghost" data-action="back-to-edit" onclick={backToEditing}>back to editing</button>
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
      <div class="ve-confirm" role="alertdialog" aria-modal="true" aria-label="Discard this clip?">
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

<style>
  /* Overlay and card. z-index 59: above ordinary overlays (50), and just BELOW the call bar, the
     incoming-call prompt and the floating call stage (all 60), so an incoming call stays visible
     and answerable over an editor. Those are small fixed pills at the top and bottom centre and
     cover no editor control. The overlay starts below the app's own title bar, like the other
     full-window layers, so the card is never clipped by it at the native 900×640 size. */
  .video-editor { position: fixed; inset: var(--titlebar-h, 0px) 0 0; z-index: 59; display: flex; align-items: center; justify-content: center; padding: 16px; background: color-mix(in oklab, var(--bg-0) 60%, transparent); color: var(--text); font-family: var(--ui); font-size: 0.84rem; }
  .card { display: flex; flex-direction: column; width: min(900px, 100%); height: min(640px, 100%); background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 18px 44px color-mix(in oklab, var(--bg-0) 60%, transparent); overflow: hidden; outline: none; }
  .card:focus-visible { outline: 1px solid var(--accent); outline-offset: -1px; }
  .grow { flex: 1; }

  /* Operator-terminal grammar shared with the flipnote viewer and the Studio. */
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .mono { font-family: var(--mono); font-size: 0.7rem; color: var(--text-2); white-space: nowrap; font-variant-numeric: tabular-nums; }
  .mono.strong { color: var(--text); min-width: 4.6em; }
  /* `ve-` prefixed where app.css has global rules of the same name (`.chip`, `.stage`): Svelte's
     scoping keeps these styles in, but it does not keep global ones out. */
  .ve-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; flex: none; background: var(--panel); }
  .ve-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .ve-chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); }
  .ve-chip.ok .micro { color: var(--ok); }
  .ve-chip.ok i { background: var(--ok); }
  .ve-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); }
  .ve-chip.warn .micro { color: var(--warn); }
  .ve-chip.warn i { background: var(--warn); }
  .st-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; cursor: pointer; }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); padding: 0 16px; }
  .st-btn svg.ol { width: 12px; height: 12px; margin-left: -4px; }
  .st-btn:disabled { opacity: 0.45; cursor: default; }
  .st-btn.x { width: 26px; padding: 0; justify-content: center; flex: none; }
  .st-btn.x svg { width: 12px; height: 12px; }
  .st-tool { width: 28px; height: 28px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: inline-flex; align-items: center; justify-content: center; position: relative; padding: 0; flex: none; cursor: pointer; }
  .st-tool:hover:not(:disabled) { color: var(--text); border-color: var(--border); }
  .st-tool svg { width: 11px; height: 11px; }
  .st-tool.sm { width: 26px; height: 26px; }
  .st-tool.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tool.danger:hover:not(:disabled) { color: var(--danger-hi); border-color: var(--danger-brd); background: var(--danger-dim); }
  .st-tool:disabled { opacity: 0.4; cursor: default; }
  .st-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin-right: 2px; }
  .st-tool.play svg { width: 14px; height: 14px; }
  .st-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  svg.solid { fill: currentColor; stroke: none; }
  .st-tool.ol svg, svg.ol { fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; width: 14px; height: 14px; }

  /* Head row */
  .head { display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border); min-width: 0; }
  .film { width: 14px; height: 14px; flex: none; fill: none; stroke: var(--accent); stroke-width: 1.5; stroke-linejoin: round; }
  .title { margin: 0; font-size: 0.95rem; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }
  .facts { overflow: hidden; text-overflow: ellipsis; min-width: 0; flex: 0 1 auto; }

  /* Picture */
  .ve-stage { position: relative; flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; background: var(--bg-0); cursor: pointer; }
  .ve-stage video { display: block; width: 100%; height: 100%; object-fit: contain; background: var(--bg-0); }
  .ve-stage-chip { position: absolute; left: 12px; top: 10px; }

  /* Transport and tools */
  .transport { display: flex; align-items: center; gap: 6px; padding: 8px 14px; border-top: 1px solid var(--border); min-height: 46px; box-sizing: border-box; }
  .transport .micro { color: var(--muted); }
  .sep { width: 1px; height: 18px; background: var(--border); margin: 0 4px; flex: none; }

  /* Timeline: ruler above a track that holds the filmstrip and every overlay. Positions are
     percentages of the track, so nothing depends on a measured width. */
  .timeline { position: relative; padding: 2px 14px 12px; border-top: 1px solid var(--border-soft); background: color-mix(in oklab, var(--bg-0) 35%, var(--panel)); user-select: none; touch-action: none; outline: none; }
  .timeline:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
  .timeline.placeholder, .transport.placeholder { min-height: 46px; }
  .timeline.placeholder { min-height: 96px; }
  .ruler { position: relative; height: 20px; margin: 0 7px; }
  .tick { position: absolute; bottom: 0; height: 6px; border-left: 1px solid var(--border); }
  .tick .micro { position: absolute; bottom: 7px; left: 3px; font-size: 0.56rem; letter-spacing: 0.06em; }
  .tick.last .micro { left: auto; right: 3px; }
  .head-mark { position: absolute; bottom: -1px; width: 0; height: 0; margin-left: -5px; border-left: 5px solid transparent; border-right: 5px solid transparent; border-top: 7px solid var(--accent-hi); pointer-events: none; z-index: 2; }
  .track { position: relative; height: 60px; margin: 0 7px; border: 1px solid var(--border); border-radius: 3px; background: var(--bg-0); cursor: text; }
  .timeline.blade .track { cursor: crosshair; }
  .timeline.locked .track { cursor: default; }
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

  .playhead { position: absolute; top: -3px; bottom: -3px; width: 2px; margin-left: -1px; background: var(--accent-hi); box-shadow: 0 0 0 1px color-mix(in oklab, var(--bg-0) 50%, transparent); pointer-events: none; z-index: 5; }
  .blade-guide { position: absolute; top: -4px; bottom: -4px; width: 0; margin-left: -1px; border-left: 2px dashed var(--warn); pointer-events: none; z-index: 6; }
  .blade-guide.refused { border-left-color: var(--faint); }

  /* Export progress replaces nothing: it sits over the timeline, which is locked meanwhile. */
  .busy { position: absolute; inset: 0; display: flex; align-items: center; gap: 10px; padding: 0 21px; background: color-mix(in oklab, var(--panel) 88%, transparent); z-index: 10; }
  .busy .micro { color: var(--text-2); }
  .bar { flex: 1; height: 6px; border-radius: 999px; background: var(--bg-0); border: 1px solid var(--border); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); }

  .alert { margin: 0; padding: 8px 14px; border-top: 1px solid var(--warn-brd); background: var(--warn-dim); color: var(--warn); font-size: 0.78rem; }

  /* "Discard this clip?": a small panel over the dimmed card, inside it so focus stays trapped. */
  .card { position: relative; }
  .ve-confirm { position: absolute; inset: 0; z-index: 20; display: flex; align-items: center; justify-content: center; background: color-mix(in oklab, var(--bg-0) 62%, transparent); }
  .ve-confirm-box { display: flex; flex-direction: column; gap: 14px; padding: 18px 20px; min-width: 280px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); box-shadow: 0 12px 32px color-mix(in oklab, var(--bg-0) 60%, transparent); }
  .ve-confirm-box h3 { margin: 0; font-size: 0.95rem; font-weight: 600; color: var(--text); }
  .ve-confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .st-btn.danger { background: var(--danger-dim); border-color: var(--danger-brd); color: var(--danger-hi); }

  /* Footer */
  .foot { display: flex; align-items: center; gap: 8px; padding: 8px 14px; border-top: 1px solid var(--border); background: var(--bg-elev); }

  /* Visually hidden button text: present for scripts and assistive tech, invisible on screen. */
  .vh { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); clip-path: inset(50%); white-space: nowrap; border: 0; }
</style>
