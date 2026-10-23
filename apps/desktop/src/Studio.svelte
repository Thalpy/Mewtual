<script lang="ts">
  // The flipnote editor surface (design-creative-suite.md 2.2, 2.10; mockups "Flipnote Editor",
  // "Timeline States", "Art / Sound / Music"), connected to the native Studio through the shared
  // session: reads are typed views, edits are publish-then-apply saves that keep their identity
  // across retries. Score/Sound and immutable media reuse the native Studio, patch validator,
  // bounded blob fetch and App's existing JamEngine; neither a second store nor a synth lives here.
  import { onDestroy, onMount, untrack } from "svelte";
  import { animationFrameAt } from "./pixa-media.ts";
  import { decodePix, pixToRgba } from "./pix.ts";
  import {
    CLEAR,
    PixRaster,
    UndoStack,
    brushCells,
    clampBrush,
    eraseValue,
    floodFill,
    mirrored,
    paintDot,
    paintShape,
    paintStroke,
    pressureSize,
    shapeFilled,
    shapeOutline,
    stampBitmap,
    textBitmap,
    type Bitmap,
    type Cell,
    type Mirror,
    type Shape,
    type Tool,
  } from "./pix-canvas.ts";
  import {
    BRUSH_MAX,
    BRUSH_MIN,
    FLIPNOTE_FPS_MAX,
    FLIPNOTE_FPS_MIN,
    FLIPNOTE_FRAME_BYTES_PROMISE,
    FLIPNOTE_H,
    FLIPNOTE_MAX_FRAMES,
    FLIPNOTE_MAX_PATCHES,
    FLIPNOTE_W,
    LAYER_NAMES,
    PIX_ROLE_NAMES,
    type PixPaletteEntry,
  } from "./studio-contract.ts";
  import { DEFAULT_PALETTE } from "./studio-store.ts";
  import { paletteGroups, paletteColorLabel } from "./studio-palette.ts";
  import { base64ToBytes, type FrameConflictValue, type FrameView, type NativeExpiry, type RecoveryMode, type RecoveryVersion } from "./studio-native.ts";
  import { reason, type RecoveryItem, type SaveRecord } from "./studio-session.ts";
  import { ensureStudio, setStudioScope, studio, studioScopeWaiting } from "./studio-state.svelte.ts";
  import { retryText, saveHeadline, saveProblemText } from "./studio-save-words.ts";
  import { frameClaimState, type ClaimView, type StudioClaimActions, type StudioClaimSnapshot } from "./draw-claims.ts";
  import ScoreEditor from "./ScoreEditor.svelte";
  import { scoreSelectionMemory } from "./score-selection.ts";
  import StudioSound from "./StudioSound.svelte";
  import StudioMusic, { type MusicTrack } from "./StudioMusic.svelte";
  import StudioSoundfont from "./StudioSoundfont.svelte";
  import { auditionPadPlayback, auditionPlayback } from "./studio-audition.ts";
  import type { ScorePlayback } from "./studio-export.ts";
  import { instrumentName, noteName } from "./studio-sound-ui.ts";
  import { JAM_SAVED_PATCHES_MAX, type JamPatch, type JamTake } from "./jam-contract.ts";
  import { jamPatchId, validateJamPatch } from "./jam-patch.ts";
  import type { StudioAudioActions, StudioAudioState } from "./studio-audio.ts";
  import { decodePixa, type Pixa } from "./pixa.ts";
  import { JAM_TAKE_MIME } from "./jukebox.ts";

  let { me, server, channel, nameOf, colorOf, adapt = true, onnotice, claims = null, claimActions = null,
    patchLibrary = [], patchShelves = [], libraryCount = 0, serverName = "", onsavePatch, onsharePatch,
    audioActions = null, audioState = { state: "stopped", elapsedMs: 0 }, onshareScore, onsharePixa, onpostPixa, onpostScore, onnavigate } = $props<{
    me: string;
    server: number | null;
    channel: string;
    nameOf: (identity: string) => string;
    colorOf: (identity: string) => string;
    adapt?: boolean;
    onnotice: (text: string, kind: "info" | "warn" | "error") => void;
    /// Gate 5 frame claims of the call this Studio's server is in, or null outside such a call.
    /// Advisory only: nothing in this component lets a claim decide whether an edit may happen.
    claims?: StudioClaimSnapshot | null;
    claimActions?: StudioClaimActions | null;
    patchLibrary?: readonly { name: string; patch: JamPatch }[];
    /// The same recipes with where each lives, for the Sound tab's rack shelves: presets ship with
    /// the app, "mine" is this device's library, "share" came from this server's encrypted share.
    patchShelves?: readonly { name: string; patch: JamPatch; shelf: "preset" | "mine" | "share" }[];
    /// How many recipes this device's library holds (the soundfont popup's save footer).
    libraryCount?: number;
    /// The server's display name for the popup's share button; empty hides sharing.
    serverName?: string;
    /// Keep a recipe in this device's library / seal it into the server share. Both reject with a
    /// message when refused (library full, wrong server); absent means the action is unavailable.
    onsavePatch?: (name: string, patch: JamPatch) => Promise<void>;
    onsharePatch?: (name: string, patch: JamPatch, server: number) => Promise<void>;
    audioActions?: StudioAudioActions | null;
    audioState?: StudioAudioState;
    onshareScore?: (take: JamTake, server: number) => Promise<void>;
    onsharePixa?: (bytes: Uint8Array, server: number) => Promise<void>;
    onpostPixa?: (bytes: Uint8Array, server: number) => Promise<void>;
    /// Attach the score's .jamtake to the chat composer, the way a flipnote's .pixa is posted.
    onpostScore?: (take: JamTake, server: number) => Promise<void>;
    /// Called the moment this surface decides to navigate to another document (the Music tab's
    /// open/create), before the departure Save can wait: see StudioNav's prop of the same name.
    onnavigate?: () => void;
  }>();

  // Built once per mount (state may be written during init, never inside a derived); the
  // surface remounts with the tab, and the session outlives it.
  // svelte-ignore state_referenced_locally
  const session = ensureStudio(me);
  // Imperative session calls run untracked: an effect must depend on what selects the work
  // (props, the selection), never on the revision the work itself bumps.
  // While the previous channel's edits are still saving, the session stays on that channel and this
  // surface shows their progress instead of an editor. `scopeWaiting` flips back once they have
  // landed, which re-runs the effect and makes the switch.
  const scopeWaiting = $derived.by(() => { void studio.rev; return studioScopeWaiting(server, channel); });
  const waitTally = $derived.by(() => { void studio.rev; return scopeWaiting ? session.saveTally() : null; });
  $effect(() => {
    const s = server, c = channel; void scopeWaiting;
    untrack(() => setStudioScope(s, c));
  });
  // The sidebar selects; the surface follows the selection (and re-opens after a remount).
  $effect(() => {
    const id = studio.selected;
    untrack(() => {
      if (id && session.doc?.object !== id) session.open(id);
      else if (!id && session.doc) session.open(null);
    });
  });

  const objectId = $derived(studio.selected);
  // Each field is derived from the revision directly: the session replaces its document state
  // on every change, but nothing downstream should hinge on that reference changing.
  const doc = $derived.by(() => { void studio.rev; return session.doc?.object === objectId ? session.doc : null; });
  const view = $derived.by(() => { void studio.rev; const d = session.doc; return d?.object === objectId ? d.view : null; });
  const model = $derived.by(() => { void studio.rev; return session.doc?.object === objectId ? session.buffering ? session.workingModel : session.doc.model : null; });
  const entry = $derived.by(() => {
    void studio.rev;
    const m = session.indexModel;
    return m ? [...m.entries, ...m.overflow].find((e) => e.id === objectId) ?? null : null;
  });
  const isScore = $derived(doc?.objectKind === "score" || (!doc && entry?.kind === "score"));
  const score = $derived.by(() => { void studio.rev; return session.doc?.object === objectId ? session.buffering ? session.workingScore : session.doc.score : null; });
  const sourceGeneration = $derived.by(() => { void studio.rev; return session.editorMediaGeneration; });
  // Which object and source generation the current audio was started against. Playback is
  // stopped when the source it was prepared from is no longer the one on screen (a navigation, a
  // lock, a flipnote's or an audition's document edited meanwhile) and never because of the
  // generation its own gesture produced: placing a note bumps the generation and then auditions
  // that note, and the audition must survive it. A Score document playing is the exception: its
  // edits, local or remote, do not stop it; a loop takes them into its next pass (the audio
  // session's `refresh`) and a one-shot play finishes the pass it began.
  let audioOwner = { object: "", generation: -1 };
  function claimAudio() { audioOwner = { object: objectId, generation: untrack(() => session.editorMediaGeneration) }; }
  $effect(() => {
    const object = objectId, generation = sourceGeneration;
    untrack(() => {
      if (audioOwner.object === object && audioOwner.generation === generation) return;
      if (audioOwner.object === object && isScore && playing) audioActions?.refresh?.();
      else audioActions?.stop();
    });
  });
  $effect(() => { void objectId; untrack(() => { mediaReceipt = ""; savedPreview = null; }); });
  const readOnlyWhy = $derived.by(() => { void studio.rev; return objectId ? session.editRefusal(objectId) : ""; });
  const pending = $derived.by(() => { void studio.rev; return objectId ? session.pendingFor(objectId) : []; });
  const attention = $derived(pending.filter((s) => s.mayHaveCommitted || s.status === "uncertain" || s.status === "blocked"));
  const inflight = $derived(pending.some((s) => s.status === "queued" || s.status === "inflight"));
  // Restored after lock and never sent: the deadline will not move these, so the editor offers
  // their explicit Save and discard in every state, without a refused departure first.
  const heldQueued = $derived.by(() => { void studio.rev; return pending.filter((s) => session.awaitsExplicitSave(s.id)); });
  const blockedCount = $derived.by(() => { void studio.rev; return attention.length ? session.blockedBehind(attention[0].id) : 0; });
  const pendingInserts = $derived(pending.filter((s): s is Extract<SaveRecord, { kind: "frame" }> => s.kind === "frame" && s.op === "insert" && !model?.frames.some(f => f.id === s.frame)));
  const receivePaused = $derived.by(() => { void studio.rev; return session.receivePaused; });
  const receivePauseDetail = $derived.by(() => { void studio.rev; return session.receivePauseDetail; });

  function who(id: string): string { return id === me ? "you" : nameOf(id); }
  function tint(id: string): string { return id === me ? "var(--accent)" : colorOf(id); }

  // --- Editor state ------------------------------------------------------------------------------
  let frameId = $state("");
  let raster = $state.raw<PixRaster | null>(null);
  // The complete reference the raster was decoded from (object, frame, cid, declared bytes), or
  // "unsaved" while it holds strokes a save is carrying. A raster is only ever shown for its own
  // reference; a changed cid OR a changed declaration is a different reference.
  let rasterRef = $state("");
  const refKey = (object: string, frame: string, cid: string, bytes: number) => `${object}/${frame}/${cid}/${bytes}`;
  const undo = new UndoStack();
  let tool = $state<Tool>("pen");
  let shape = $state<Shape>("ellipse");
  let shapeFill = $state(false);
  let brush = $state(3);
  // Pen pressure drives the brush size (the chosen size is the maximum). Never in the document:
  // a frame is still just pixels, so a viewer without a tablet sees exactly what was drawn.
  let pressureOn = $state(true);
  let penSeen = $state(false);
  let mirror = $state<Mirror>({ h: false, v: false });
  let color = $state(1); // palette index
  let layer = $state(2);
  let layerVisible = $state([true, true, true]);
  let onion = $state(true);
  let grid = $state(true);
  // The prop seeds the toggle; the surface then owns it (a per-viewer preference, 2.1).
  // svelte-ignore state_referenced_locally
  let adaptOn = $state(adapt);
  let inspectorTab = $state<"art" | "sound" | "music" | "history">("art");
  let stampIdx = $state(0);
  let textDraft = $state("nya~");
  // A one-note audition ("hear it", the popup keyboard) runs through the same audio session as
  // document playback, so the session reports it as playing. The editor must not treat it as
  // one: the rack stays enabled and the timeline does not follow the audition's clock. The flag
  // is raised before the audition starts and lowered when the session reports stopped.
  let auditioning = $state(false);
  $effect(() => { if (audioState.state === "stopped") auditioning = false; });
  // Which audition request owns the flag. A newer start cancels an older one still preparing, and
  // the older request then resolves false; that completion belongs to nobody and must neither
  // lower the newer audition's flag (the editor would read its clock as document playback and
  // scrub the timeline) nor report a failure. Both document Plays (editor and saved version) and
  // teardown also retire it.
  let auditionRequest = 0;
  const playing = $derived(audioState.state !== "stopped" && !auditioning);
  let loop = $state(true);
  $effect(() => { void objectId; const score = isScore; untrack(() => { loop = !score; }); });
  let mediaBusy = $state(false), mediaReceipt = $state("");
  let savedPreview = $state.raw<Pixa | null>(null);
  const ZOOM_MIN = 1, ZOOM_MAX = 6;
  let zoom = $state(3);
  let fitCanvas = $state(true);
  function setZoom(z: number) { fitCanvas = false; zoom = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, Math.round(z))); }
  let dirty = $state(false);
  // A selection can change before Svelte runs its effects. Pixel custody belongs to the
  // object/frame that loaded this raster, never to the newly selected sidebar item.
  let rasterOwner: { generation: number; object: string; frame: string } | null = null;
  let tick = $state(0); // 1 Hz for countdowns
  let paintRev = $state(0); // bumped after every raster edit so overlays repaint

  const frames = $derived<FrameView[]>(model?.frames ?? []);
  const frameIds = $derived(frames.map((f) => f.id));
  const frameIndex = $derived(frameIds.indexOf(frameId));
  const frameRec = $derived(frames.find((f) => f.id === frameId) ?? null);
  const pendingHere = $derived(pendingInserts.find((s) => s.frame === frameId) ?? null);
  const frameKnown = $derived(frameIndex >= 0 || pendingHere !== null);
  const totalBytes = $derived(model?.declaredFrameBytes ?? 0);
  const displayedPalette = $derived(raster?.palette ?? DEFAULT_PALETTE);
  const colorGroups = $derived(paletteGroups(displayedPalette));
  const overCapCount = $derived(model?.overCapCount ?? 0);
  const canEdit = $derived(!!raster && !!model && !isScore && !readOnlyWhy && overCapCount === 0 && !playing);
  const hasScope = $derived.by(() => { void studio.rev; return session.scope !== null; });
  // Blob state is session data, not part of the model: read it through the revision so the
  // strip and the veil follow a fetch as it lands.
  function blobStateOf(cid: string, bytes: number) { void studio.rev; return session.blobState(cid, bytes); }
  function blobProblemOf(cid: string, bytes: number) { void studio.rev; return session.blobProblem(cid, bytes); }
  const blobStateHere = $derived(frameRec ? blobStateOf(frameRec.cid, frameRec.bytes) : "held");
  function sameBytes(a: Uint8Array | undefined, b: Uint8Array): boolean {
    if (!a || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }

  // Built-in stamps (the emoji/ folder's pix stamps are the real source once C1 lands): palette
  // indices straight from the default palette, CLEAR where the stamp leaves the frame alone.
  const STAMPS: { name: string; bm: Bitmap }[] = [
    { name: "heart", bm: bits(8, [".##..##.", "########", "########", ".######.", "..####..", "...##...", "........", "........"], 8) },
    { name: "star", bm: bits(8, ["...#....", "...#....", "..###...", "#######.", ".#####..", "..###...", ".##.##..", "#.....#."], 9) },
    { name: "paw", bm: bits(8, [".#..#...", "#..#..#.", ".....#..", "..###...", ".#####..", ".#####..", "..###...", "........"], 1) },
    { name: "moon", bm: bits(8, ["...###..", "..##....", ".##.....", ".##.....", ".##.....", ".##.....", "..##....", "...###.."], 1) },
    { name: "note", bm: bits(8, ["....##..", "....#.#.", "....#...", "....#...", "....#...", "..###...", ".####...", "..##...."], 4) },
  ];
  function bits(w: number, rows: string[], value: number): Bitmap {
    const pixels = new Uint8Array(w * rows.length).fill(CLEAR);
    rows.forEach((r, y) => { for (let x = 0; x < w; x++) if (r[x] === "#") pixels[y * w + x] = value; });
    return { w, h: rows.length, pixels };
  }

  // --- Theme adaptation: role entries take the viewer's tokens ---------------------------------
  function cssRgb(name: string): [number, number, number] | null {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    const m = /^#([0-9a-f]{6})$/i.exec(v);
    if (!m) return null;
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  let roleLut = $state.raw<Record<number, [number, number, number]>>({});
  function refreshRoles() {
    const bg = cssRgb("--bg-0");
    const out: Record<number, [number, number, number]> = {};
    const set = (role: number, c: [number, number, number] | null) => { if (c) out[role] = c; };
    set(1, bg);
    set(2, cssRgb("--text"));
    set(3, cssRgb("--accent"));
    set(4, cssRgb("--muted"));
    set(5, bg ? [Math.round(bg[0] * 0.6), Math.round(bg[1] * 0.6), Math.round(bg[2] * 0.6)] : null);
    set(6, cssRgb("--border"));
    set(7, cssRgb("--faint"));
    set(8, cssRgb("--text-2"));
    roleLut = out;
  }
  function resolve(e: PixPaletteEntry): readonly [number, number, number] {
    if (adaptOn && e.role !== 0) { const c = roleLut[e.role]; if (c) return c; }
    return [e.r, e.g, e.b];
  }
  function wellCss(e: PixPaletteEntry): string {
    const [r, g, b] = resolve(e);
    return `rgb(${r}, ${g}, ${b})`;
  }

  // --- Frame open / commit --------------------------------------------------------------------
  function openFrame(id: string, fromPlayback = false) {
    if (!model) return;
    if (!fromPlayback && playing) audioActions?.stop();
    if (!commit()) return;
    frameId = id;
    loadRaster();
  }

  /// Load the raster from the newest unsaved pixels for this frame, else from the held blob for
  /// its selected value; ask for the blob (top priority) when neither is here yet.
  function loadRaster() {
    const rec = frames.find((f) => f.id === frameId) ?? null;
    const unsaved = objectId && frameId ? session.unsavedPix(objectId, frameId) : null;
    const bytes = unsaved ?? (rec ? session.blob(rec.cid, rec.bytes) : undefined);
    if (!bytes) {
      raster = null;
      rasterRef = "";
      if (rec) session.want(rec.cid, rec.bytes, 0);
      return;
    }
    let img;
    try { img = decodePix(bytes); } catch (e) { raster = null; rasterRef = ""; onnotice(`frame pixels rejected: ${reason(e)}`, "warn"); return; }
    const r = new PixRaster(img.w, img.h, img.palette);
    r.loadFlat(img.pixels);
    raster = r;
    rasterOwner = objectId ? { generation: session.generation, object: objectId, frame: frameId } : null;
    rasterRef = unsaved ? "unsaved" : rec ? refKey(objectId, frameId, rec.cid, rec.bytes) : "";
    undo.clear();
    dirty = false;
    paintRev++;
  }

  /// Write the raster back as a replace_frame save if anything changed (2.9: an op only when a
  /// value changed; bursts are coalesced by committing on stroke end, not per cell). The save
  /// keeps the pixels until it lands; a failure shows up in the save card, never as lost work.
  function commit(): boolean {
    if (!dirty) return true;
    if (!raster || !rasterOwner || rasterOwner.generation !== session.generation) return false;
    try {
      session.saveFrame(rasterOwner.object, rasterOwner.frame, raster.encode());
      rasterRef = "unsaved";
    } catch (e) {
      onnotice(reason(e), "warn");
      return false;
    }
    dirty = false;
    return true;
  }

  const removeCloseHandler = session.beforeDocumentClose(() => {
    // Pointer-up normally commits; navigation can also interrupt an in-progress stroke.
    if (!commit()) throw new Error("Could not save this drawing. Please retry before opening another flipnote.");
    stroking = false; last = null; anchor = null; previewCells = [];
    audioActions?.stop();
  });
  const removeSaveHandler = session.beforeSave(() => {
    if (!commit()) throw new Error("Could not keep this drawing. Your changes are still on the canvas.");
  });

  function edited(changedCells: number) {
    if (!changedCells) return;
    dirty = true;
    session.markDirty();
    paintRev++;
    // Painting says "I am drawing this frame" to the call (advisory; it gates nothing). The session
    // ignores it outside a call and after this device Passed the frame away. Every pointer sample
    // lands here, so a frame already claimed costs nothing further.
    if (claims && objectId && frameId && (claims.local.claim?.obj !== objectId || claims.local.claim?.fr !== frameId)) {
      claimActions?.autoClaim(objectId, frameId);
    }
  }

  // --- Frame claims (Gate 5): who is drawing which frame, never a lock ---------------------------
  // Moving to another frame or object withdraws my claim, any ask for a frame I am no longer
  // looking at, and a Pass suppression for another frame, so a claim always names the frame I
  // actually have open (the session decides; see selectionChanged).
  $effect(() => {
    const obj = objectId ?? "";
    const fr = frameId;
    untrack(() => {
      if (claims) claimActions?.select(obj, fr);
    });
  });
  const frameClaims = $derived(frameClaimState(claims, objectId ?? "", frameId));
  // Live peer claims per frame of the open object, for the timeline markers.
  const claimsByFrame = $derived.by(() => {
    const out = new Map<string, ClaimView[]>();
    for (const e of claims?.entries ?? []) {
      if (e.obj !== objectId || e.kind !== "claim") continue;
      const list = out.get(e.fr) ?? [];
      list.push(e);
      out.set(e.fr, list);
    }
    return out;
  });
  // Asks on the frame I am drawing: each is a sticky notice with its countdown and Pass.
  const askNotices = $derived((claims?.asksOnMine ?? []).filter((a: ClaimView) => a.obj === objectId));
  function frameNumber(fr: string): string {
    const i = frameIds.indexOf(fr);
    return i >= 0 ? String(i + 1) : "?";
  }
  function claimTitle(list: readonly ClaimView[]): string {
    return list.map((c) => `${who(c.peer)} is drawing this frame · ${c.secondsLeft} s unless refreshed`).join("\n");
  }

  // --- Pointer input on the canvas ----------------------------------------------------------------
  let rootEl = $state<HTMLDivElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let overlayEl = $state<HTMLCanvasElement | null>(null);
  let hover = $state<Cell | null>(null);
  let anchor: Cell | null = null;
  let last: Cell | null = null;
  let stroking = false;
  let previewCells = $state.raw<Cell[]>([]);

  function cellAt(e: PointerEvent): Cell | null {
    if (!canvasEl || !raster) return null;
    const rect = canvasEl.getBoundingClientRect();
    const x = Math.floor(((e.clientX - rect.left) / rect.width) * raster.w);
    const y = Math.floor(((e.clientY - rect.top) / rect.height) * raster.h);
    return [x, y];
  }

  function paintValue(): number {
    return tool === "eraser" && raster ? eraseValue(raster, layer) : color;
  }

  function sizeFor(e: PointerEvent): number {
    if (e.pointerType === "pen") penSeen = true;
    return pressureSize(brush, e.pressure, e.pointerType, pressureOn);
  }

  function onDown(e: PointerEvent) {
    if (!raster || !canEdit || e.button === 2) return;
    const c = cellAt(e);
    if (!c) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
    if (tool === "pen" || tool === "eraser") {
      undo.push(raster);
      stroking = true;
      last = c;
      edited(paintDot(raster, layer, c[0], c[1], paintValue(), sizeFor(e), mirror));
    } else if (tool === "fill") {
      undo.push(raster);
      edited(floodFill(raster, layer, c[0], c[1], color));
      commit();
    } else if (tool === "shape") {
      anchor = c;
      previewCells = [];
    } else if (tool === "stamp") {
      undo.push(raster);
      const bm = STAMPS[stampIdx].bm;
      edited(stampBitmap(raster, layer, c[0] - (bm.w >> 1), c[1] - (bm.h >> 1), bm, mirror));
      commit();
    } else if (tool === "text") {
      undo.push(raster);
      const bm = textBitmap(textDraft, color);
      edited(stampBitmap(raster, layer, c[0], c[1], bm, mirror));
      commit();
    }
  }

  function onMove(e: PointerEvent) {
    const c = cellAt(e);
    hover = c;
    if (!raster || !c) return;
    if (stroking && last && (tool === "pen" || tool === "eraser")) {
      // The samples the browser coalesced since the last event, each with its own pressure,
      // so a fast pen stroke stays continuous and a lightening touch thins along the way.
      const samples = typeof e.getCoalescedEvents === "function" && e.getCoalescedEvents().length ? e.getCoalescedEvents() : [e];
      let changed = 0;
      for (const s of samples) {
        const sc = cellAt(s);
        if (!sc) continue;
        changed += paintStroke(raster, layer, last[0], last[1], sc[0], sc[1], paintValue(), sizeFor(s), mirror);
        last = sc;
      }
      edited(changed);
    } else if (anchor && tool === "shape") {
      const cells = shapeFill ? shapeFilled(shape, anchor[0], anchor[1], c[0], c[1]) : shapeOutline(shape, anchor[0], anchor[1], c[0], c[1]);
      previewCells = mirrored(cells, raster.w, raster.h, mirror);
    }
  }

  function onUp(e: PointerEvent) {
    if (!raster) return;
    const c = cellAt(e);
    if (stroking) {
      stroking = false;
      last = null;
      commit();
    } else if (anchor && tool === "shape" && c) {
      undo.push(raster);
      edited(paintShape(raster, layer, shape, anchor[0], anchor[1], c[0], c[1], color, brush, shapeFill, mirror));
      anchor = null;
      previewCells = [];
      commit();
    }
  }

  function onLeave() {
    hover = null;
    if (stroking) { stroking = false; last = null; commit(); }
  }

  function doUndo() { if (raster && undo.undo(raster)) { dirty = true; paintRev++; commit(); } }
  function doRedo() { if (raster && undo.redo(raster)) { dirty = true; paintRev++; commit(); } }

  // --- Rendering ---------------------------------------------------------------------------------
  let scratch: HTMLCanvasElement | null = null;
  function scratchCanvas(w: number, h: number): CanvasRenderingContext2D {
    if (!scratch) scratch = document.createElement("canvas");
    scratch.width = w;
    scratch.height = h;
    return scratch.getContext("2d")!;
  }

  function drawFrame() {
    if (!canvasEl || !model) return;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    const w = raster?.w ?? FLIPNOTE_W, h = raster?.h ?? FLIPNOTE_H;
    canvasEl.width = w * zoom;
    canvasEl.height = h * zoom;
    ctx.imageSmoothingEnabled = false;
    const bgc = resolve(raster?.palette[raster.bgIndex()] ?? DEFAULT_PALETTE[0]);
    ctx.fillStyle = `rgb(${bgc[0]}, ${bgc[1]}, ${bgc[2]})`;
    ctx.fillRect(0, 0, canvasEl.width, canvasEl.height);
    if (raster) {
      // Respect hidden layers in the view only: the frame's bytes always flatten everything.
      const flat = new Uint8Array(raster.w * raster.h);
      const base = raster.layers[0];
      flat.set(layerVisible[0] ? base : new Uint8Array(base.length).fill(raster.bgIndex()));
      for (let l = 1; l < raster.layers.length; l++) {
        if (!layerVisible[l]) continue;
        const src = raster.layers[l];
        for (let i = 0; i < src.length; i++) if (src[i] !== CLEAR) flat[i] = src[i];
      }
      const s = scratchCanvas(raster.w, raster.h);
      s.putImageData(new ImageData(pixToRgba({ w: raster.w, h: raster.h, palette: raster.palette, pixels: flat }, resolve), raster.w, raster.h), 0, 0);
      ctx.drawImage(scratch!, 0, 0, canvasEl.width, canvasEl.height);
    }
    // Onion: the previous frame ghosted OVER the current one (one frame back in v1), with its
    // paper cells cut out so only its marks show through. Under the frame it would be hidden by
    // the current frame's opaque paper.
    if (onion && frameIndex > 0) {
      const prevRec = frames[frameIndex - 1];
      const prev = session.blob(prevRec.cid, prevRec.bytes);
      if (!prev) session.want(prevRec.cid, prevRec.bytes, 1);
      else {
        try {
          const img = decodePix(prev);
          const rgba = pixToRgba(img, resolve);
          const paper = 0;
          for (let i = 0; i < img.pixels.length; i++) if (img.pixels[i] === paper) rgba[i * 4 + 3] = 0;
          const s = scratchCanvas(img.w, img.h);
          s.putImageData(new ImageData(rgba, img.w, img.h), 0, 0);
          ctx.globalAlpha = 0.3;
          ctx.drawImage(scratch!, 0, 0, canvasEl.width, canvasEl.height);
          ctx.globalAlpha = 1;
        } catch { /* an invalid neighbour is reported on its own thumbnail */ }
      }
    }
  }

  function drawOverlay() {
    if (!overlayEl || !model) return;
    const ctx = overlayEl.getContext("2d");
    if (!ctx) return;
    const w = raster?.w ?? FLIPNOTE_W, h = raster?.h ?? FLIPNOTE_H;
    const z = zoom;
    overlayEl.width = w * z;
    overlayEl.height = h * z;
    ctx.clearRect(0, 0, overlayEl.width, overlayEl.height);
    const accent = getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#977df2";
    const ink = "#000000";
    // The cell grid needs at least three screen pixels per cell to read; the 4th-cell grid always.
    if (grid) {
      ctx.lineWidth = 1;
      if (z >= 3) {
        ctx.strokeStyle = ink;
        ctx.globalAlpha = 0.1;
        ctx.beginPath();
        for (let x = 1; x < w; x++) { ctx.moveTo(x * z + 0.5, 0); ctx.lineTo(x * z + 0.5, h * z); }
        for (let y = 1; y < h; y++) { ctx.moveTo(0, y * z + 0.5); ctx.lineTo(w * z, y * z + 0.5); }
        ctx.stroke();
      }
      ctx.strokeStyle = accent;
      ctx.globalAlpha = 0.3;
      ctx.beginPath();
      for (let x = 4; x < w; x += 4) { ctx.moveTo(x * z + 0.5, 0); ctx.lineTo(x * z + 0.5, h * z); }
      for (let y = 4; y < h; y += 4) { ctx.moveTo(0, y * z + 0.5); ctx.lineTo(w * z, y * z + 0.5); }
      ctx.stroke();
      ctx.globalAlpha = 1;
    }
    ctx.fillStyle = accent;
    ctx.globalAlpha = 0.55;
    for (const [x, y] of previewCells) if (x >= 0 && y >= 0 && x < w && y < h) ctx.fillRect(x * z, y * z, z, z);
    ctx.globalAlpha = 1;
    if (hover && canEdit && (tool === "pen" || tool === "eraser" || tool === "shape")) {
      ctx.strokeStyle = accent;
      ctx.lineWidth = 1;
      for (const [x, y] of mirrored(brushCells(hover[0], hover[1], tool === "shape" ? 1 : brush), w, h, mirror)) {
        if (x >= 0 && y >= 0 && x < w && y < h) ctx.strokeRect(x * z + 0.5, y * z + 0.5, z - 1, z - 1);
      }
    } else if (hover && canEdit && tool === "stamp") {
      const bm = STAMPS[stampIdx].bm;
      ctx.strokeStyle = accent;
      ctx.strokeRect((hover[0] - (bm.w >> 1)) * z + 0.5, (hover[1] - (bm.h >> 1)) * z + 0.5, bm.w * z, bm.h * z);
    }
  }

  $effect(() => {
    void paintRev; void onion; void adaptOn; void roleLut; void layerVisible; void frameIndex; void studio.rev; void zoom;
    drawFrame();
  });
  $effect(() => {
    void grid; void hover; void previewCells; void brush; void mirror; void tool; void stampIdx; void canEdit; void zoom;
    drawOverlay();
  });
  $effect(() => {
    void adaptOn;
    refreshRoles();
  });
  $effect(() => {
    // A new object, or a frame list that no longer holds the open frame: open the first frame.
    if (model && (!frameId || !frameKnown)) {
      const first = frameIds[0] ?? pendingInserts[0]?.frame;
      if (first) openFrame(first);
      else { frameId = ""; raster = null; rasterRef = ""; }
    } else if (!model && frameId) { frameId = ""; raster = null; rasterRef = ""; }
  });
  $effect(() => {
    // Pixels arrived for the open frame, or its selected reference (cid or declared bytes)
    // changed under us while we were not drawing: follow it. A dirty raster keeps the member's
    // strokes, and pending strokes are the reference until their save lands; only a clean
    // raster whose reference no longer has valid held bytes is dropped, so the veil and the
    // lock take over instead of an editable canvas of a value the session no longer accepts.
    void studio.rev;
    if (!frameId || dirty || !objectId) return;
    const rec = frames.find((f) => f.id === frameId);
    const unsaved = session.unsavedPix(objectId, frameId);
    const wanted = unsaved ? "unsaved" : rec ? refKey(objectId, frameId, rec.cid, rec.bytes) : "";
    if (!wanted || wanted === rasterRef) return;
    // Our own save just landed: the selected value now names the bytes this raster already
    // holds, so adopt the reference and keep the undo history instead of reloading.
    if (rasterRef === "unsaved" && raster && rec && sameBytes(session.blob(rec.cid, rec.bytes), raster.encode())) { rasterRef = wanted; return; }
    if (unsaved || (rec && session.blob(rec.cid, rec.bytes))) { loadRaster(); return; }
    // The current reference has no valid bytes here: a stale raster must not stand in for it.
    if (raster) { raster = null; rasterRef = ""; undo.clear(); paintRev++; }
    if (rec) session.want(rec.cid, rec.bytes, 0);
  });
  $effect(() => {
    // The recovery rail follows the open document. Watching is idempotent for the same target,
    // and it runs untracked so the listing it starts cannot re-trigger this effect.
    const id = objectId;
    untrack(() => { if (id) session.watchRecovery(id); });
  });

  /// Thumbnail action: decode the frame's held bytes into a small canvas, or ask for them.
  function thumb(node: HTMLCanvasElement, arg: { cid: string; bytes: number; rev: number }) {
    const draw = (a: { cid: string; bytes: number; rev: number }) => {
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      ctx.clearRect(0, 0, node.width, node.height);
      const bytes = session.blob(a.cid, a.bytes);
      if (!bytes) { session.want(a.cid, a.bytes, 2); return; }
      try {
        const img = decodePix(bytes);
        const s = scratchCanvas(img.w, img.h);
        s.putImageData(new ImageData(pixToRgba(img, resolve), img.w, img.h), 0, 0);
        ctx.drawImage(scratch!, 0, 0, node.width, node.height);
      } catch { /* shown as invalid by the frame's state */ }
    };
    draw(arg);
    return { update: draw };
  }

  /// A pending insert's thumbnail comes from its own unsaved pixels.
  function thumbBytes(node: HTMLCanvasElement, arg: { bytes: Uint8Array; rev: number }) {
    const draw = (a: { bytes: Uint8Array; rev: number }) => {
      const ctx = node.getContext("2d");
      if (!ctx) return;
      ctx.imageSmoothingEnabled = false;
      try {
        const img = decodePix(a.bytes);
        const s = scratchCanvas(img.w, img.h);
        s.putImageData(new ImageData(pixToRgba(img, resolve), img.w, img.h), 0, 0);
        ctx.drawImage(scratch!, 0, 0, node.width, node.height);
      } catch { /* nothing to show */ }
    };
    draw(arg);
    return { update: draw };
  }

  // --- Timeline actions ------------------------------------------------------------------------
  /// Runs a session call and turns its refusal into a notice; the boolean lets a caller that
  /// has a follow-up (the score composer auditions a note it just placed) skip it on refusal.
  function guard(fn: () => void): boolean {
    try { fn(); return true; } catch (e) { onnotice(reason(e), "warn"); return false; }
  }

  function blankPix(): Uint8Array {
    return new PixRaster(FLIPNOTE_W, FLIPNOTE_H, raster?.palette.map((e) => ({ ...e })) ?? DEFAULT_PALETTE.map((e) => ({ ...e }))).encode();
  }

  function addFrameAfter(copy: boolean) {
    if (!model || !objectId) return;
    if (!commit()) return;
    guard(() => {
      const bytes = copy && raster ? raster.encode() : blankPix();
      const id = session.insertFrame(objectId, frameIndex >= 0 ? frameId : null, bytes);
      frameId = id;
      loadRaster();
    });
  }

  function deleteFrame() {
    if (!model || !frameId || !objectId) return;
    if (pendingHere) { onnotice("this frame is still being saved; wait for it or discard the save below", "info"); return; }
    if (!commit()) return;
    const idx = frameIndex;
    const gone = frameId;
    guard(() => {
      session.removeFrame(objectId, gone);
      const next = frameIds.filter((f) => f !== gone)[Math.max(0, Math.min(idx, frameIds.length - 2))];
      frameId = ""; raster = null; rasterRef = "";
      if (next) openFrame(next);
    });
  }

  function step(delta: number) {
    if (!frameIds.length) return;
    const n = frameIds.length;
    let i = (Math.max(0, frameIndex) + delta + n) % n;
    openFrame(frameIds[i]);
  }

  $effect(() => {
    const elapsed = audioState.elapsedMs, status = audioState.state, fps = model?.fps ?? 12;
    if (status === "playing" && !auditioning && frameIds.length && !isScore && !savedPreview) {
      // Sound holds/releases may outlast the pictures. Keep the pictures moving without
      // shortening authored sound or creating another overlapping synth playback.
      const id = frameIds[animationFrameAt(frameIds.length, fps, elapsed, loop)];
      if (id !== frameId) untrack(() => openFrame(id, true));
    }
  });

  /// `offsetMs` lets the score composer start from its selected step. Button handlers pass a
  /// MouseEvent here, and the space hotkey nothing: both read as "from the start".
  function toggleStudioPlay(offsetMs: number | Event | undefined = undefined) {
    if (playing) { audioActions?.stop(); return; }
    if (!audioActions || !objectId) { onnotice("audio playback is unavailable on this surface", "warn"); return; }
    if (!commit()) return;
    // Document playback replacing a sounding audition: the session goes straight from
    // "playing" to "preparing" without the UI ever observing "stopped", so the flag must be
    // lowered here or the transport would keep reading "play" for the whole document run.
    auditioning = false; auditionRequest++;
    savedPreview = null;
    const object = objectId, kind = isScore ? "score" : "flipnote";
    // Play is local listening: it plays what is on screen, saved yet or not (see mediaSource).
    const prepare = () => kind === "score" ? session.prepareScorePlayback(object) : session.prepareFlipnotePixa(object, { playback: true });
    const from = typeof offsetMs === "number" && Number.isFinite(offsetMs) && offsetMs > 0 ? Math.floor(offsetMs) : 0;
    claimAudio();
    // The App adapter resumes its existing context immediately, before prepare fetches/hashes.
    // Editor Play is the whole composition unless the composer asked for a start step. Stop
    // leaves a frame selected for editing, which must not turn the next Play into an implicit
    // seek that skips the first sound marker.
    // `document`: this is the open document's own playback (a layered score may hold more notes).
    void audioActions.start(kind, prepare, loop, from, true)
      .then((started: boolean) => { if (!started) onnotice("playback was cancelled or audio is unavailable; press Play to try again", "warn"); })
      .catch((e: unknown) => onnotice(reason(e), "warn"));
  }
  async function addStudioPatch(patch: JamPatch): Promise<string> {
    const object = objectId, gen = session.generation, kind = isScore ? "score" : "flipnote";
    const sourceView = session.doc?.view, sourceEpoch = session.mediaGeneration;
    const checked = validateJamPatch(patch); if (!checked.ok) throw new Error(checked.error);
    const id = await jamPatchId(checked.patch);
    if (!object || gen !== session.generation || session.doc?.object !== object || session.doc.objectKind !== kind || session.doc.view !== sourceView || session.mediaGeneration !== sourceEpoch) throw new Error("the recipe target changed; choose it again");
    if (kind === "score") session.setScorePatch(object, id, checked.patch); else session.setSoundPatch(object, id, checked.patch);
    return id;
  }

  // --- Sound: audition and the soundfont popup ---------------------------------------------------
  /// Play one note of a recipe through the Studio's own audio session. No second synth path: the
  /// note is a one-event take, so it inherits the session's gesture, scope and Deafen gates.
  function auditionSound(patch: JamPatch, note: number, durationMs: number, volume: number | undefined = undefined) {
    playAudition((group) => auditionPlayback({ group, exporter: me }, patch, note, durationMs, volume));
  }
  /// One pad hit through the same path, so a placed or chosen drum is heard like a placed note.
  function auditionPad(pad: number) {
    playAudition((group) => auditionPadPlayback({ group, exporter: me }, pad));
  }
  function playAudition(build: (group: string) => ScorePlayback) {
    if (!audioActions) { onnotice("audio playback is unavailable on this surface", "warn"); return; }
    const group = view?.group;
    if (!group) { onnotice("reopen this document before listening: its group is not known yet", "warn"); return; }
    let playback;
    try { playback = build(group); } catch (e) { onnotice(reason(e), "warn"); return; }
    // Raised before start: the session reports "preparing"/"playing" for this note and the
    // editor must already know it is not document playback when that lands.
    auditioning = true;
    const request = ++auditionRequest;
    claimAudio();
    void audioActions.start("score", async () => playback, false)
      .then((started: boolean) => { if (!started && request === auditionRequest) { auditioning = false; onnotice("audition unavailable; check Deafen and try again", "warn"); } })
      .catch((e: unknown) => { if (request === auditionRequest) { auditioning = false; onnotice(reason(e), "warn"); } });
  }
  /// The popup edits a clone and hands back a validated recipe; `null` means closed.
  let soundfont = $state.raw<{ name: string; initial: JamPatch } | null>(null);
  function openSoundfont(startFrom: JamPatch | null) {
    const seed = startFrom ?? patchLibrary[0]?.patch;
    if (!seed) { onnotice("no recipe to start from", "warn"); return; }
    const named = patchLibrary.find((p: { name: string; patch: JamPatch }) => p.patch === startFrom);
    soundfont = { name: named?.name ?? "", initial: JSON.parse(JSON.stringify(seed)) as JamPatch };
  }
  /// "Use in <flipnote>": the recipe joins the document; it is also kept in this device's library
  /// under its name, because the document stores descriptors and a name lives only in a library.
  /// A full library refuses the keeping half only, and says so.
  async function useSoundfont(name: string, patch: JamPatch) {
    await addStudioPatch(patch);
    if (!onsavePatch || !name.trim()) return;
    try { await onsavePatch(name.trim(), patch); }
    catch (e) { onnotice(`added to the flipnote; not kept in your library: ${reason(e)}`, "info"); }
  }
  async function shareSoundfont(name: string, patch: JamPatch) {
    const scope = session.scope;
    if (!onsharePatch || !scope) throw new Error("sharing is unavailable on this surface");
    await onsharePatch(name, patch, scope.server);
  }
  const refusePatch = async () => { throw new Error("unavailable on this surface"); };
  /// Scores of this channel, for the Music tab's picker and for naming a linked track.
  const musicTracks = $derived.by((): MusicTrack[] => {
    void studio.rev;
    const m = session.indexModel;
    if (!m) return [];
    return [...m.entries, ...m.overflow, ...m.deleted].filter((e) => e.kind === "score")
      .map((e) => ({ id: e.id, title: e.title, createdBy: e.createdBy, where: e.where }));
  });
  /// The timeline's sfx lane: one pill per frame naming the first marker (and how many more).
  function laneMarkers(frame: string): { label: string; author: string; count: number } | null {
    const here = model?.sound.sfx.filter((s) => s.frame === frame) ?? [];
    if (!here.length) return null;
    const first = here[0];
    return { label: `${instrumentName(model?.sound.patches[first.patch], patchLibrary)} · ${noteName(first.note)}`, author: first.source.author, count: here.length };
  }
  async function exportMedia(kind: "pixa" | "gif") {
    if (mediaBusy || !objectId) return;
    if (!commit()) return;
    const object = objectId, gen = session.generation; mediaBusy = true; mediaReceipt = "";
    try {
      if (kind === "pixa") {
        const result = await session.publishFlipnotePixa(object);
        // Pending saves already show their live status below; a one-off success banner would
        // remain stale after a delayed reference lands (or is refused).
        if (gen === session.generation && objectId === object) mediaReceipt = result.warning ? "The file was created, but saving this version needs attention." : "";
      } else {
        const result = await session.exportFlipnoteGif(object);
        if (gen === session.generation && objectId === object) mediaReceipt = `GIF saved locally: ${result.path}${result.warning ? ` · ${result.warning}` : ""} · visual only, no audio`;
      }
    } catch (e) { onnotice(reason(e), "warn"); } finally { mediaBusy = false; }
  }
  function browserDownload(bytes: Uint8Array, filename: string, type: string) {
    const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type }));
    const a = document.createElement("a"); a.href = url; a.download = filename; a.click();
    setTimeout(() => URL.revokeObjectURL(url), 10_000);
    // A browser download request has no native custody acknowledgement. Do not claim a path or
    // successful OS save; the native GIF route above deliberately has a different result.
    mediaReceipt = `Download started: ${filename}`;
  }
  /// Prepare the open document's package once and hand it where asked: a browser download, the
  /// server share (score only) or the chat composer. The preparation is checked against the
  /// document and session it started from before any callback sees it.
  async function downloadMedia(how: "download" | "share" | "post" = "download") {
    if (mediaBusy || !objectId) return;
    if (!commit()) return;
    const object = objectId, gen = session.generation, target = session.scope; mediaBusy = true;
    try {
      if (isScore) {
        const take = await session.prepareScoreTake(object);
        if (gen !== session.generation || objectId !== object || !target) throw new Error("the export target changed");
        if (how === "share") { if (!onshareScore) throw new Error("sharing is unavailable on this surface"); await onshareScore(take, target.server); }
        else if (how === "post") { if (!onpostScore) throw new Error("Chat attachments are unavailable."); await onpostScore(take, target.server); }
        else browserDownload(new TextEncoder().encode(JSON.stringify(take)), "Mewtual-score.jamtake", JAM_TAKE_MIME);
      } else {
        const bytes = await session.prepareFlipnotePixa(object);
        if (gen !== session.generation || objectId !== object || !target) throw new Error("The flipnote changed. Try again.");
        if (how === "post") {
          if (!onpostPixa) throw new Error("Chat attachments are unavailable.");
          await onpostPixa(bytes, target.server);
        } else browserDownload(bytes, "Mewtual-flipnote.pixa", "application/x-mewtual-pixa");
      }
    } catch (e) { onnotice(reason(e), "warn"); } finally { mediaBusy = false; }
  }
  async function publishedMedia(id: string, action: "play" | "download" | "share") {
    if (mediaBusy || !objectId) return;
    const object = objectId, gen = session.generation, target = session.scope;
    const prepare = async () => {
      const bytes = await session.preparePublishedPixa(object, id);
      if (gen !== session.generation || objectId !== object) throw new Error("the published export target changed");
      if (action === "play") savedPreview = decodePixa(bytes);
      return bytes;
    };
    if (action === "play") {
      if (!audioActions) return;
      auditioning = false; auditionRequest++; // a saved version is document playback, never an audition
      claimAudio();
      void audioActions.start("flipnote", prepare, loop).catch((e: unknown) => onnotice(reason(e), "warn"));
      return;
    }
    mediaBusy = true;
    try {
      const bytes = await prepare();
      if (action === "share") { if (!onsharePixa || !target) throw new Error("package sharing is unavailable"); await onsharePixa(bytes, target.server); }
      else browserDownload(bytes, "Mewtual-published-flipnote.pixa", "application/octet-stream");
    } catch (e) { onnotice(reason(e), "warn"); } finally { mediaBusy = false; }
  }

  function setFps(v: number) {
    if (!objectId || !Number.isInteger(v) || v < FLIPNOTE_FPS_MIN || v > FLIPNOTE_FPS_MAX) { onnotice("fps is 1 to 24", "warn"); return; }
    guard(() => session.setFps(objectId, v));
  }
  function setTitle(v: string) {
    if (!objectId || !model) return;
    const t = v.trim();
    if (t && t !== model.title) guard(() => session.setTitle(objectId, t));
  }

  // --- Conflicts: every live value of a frame's pixels, each usable on purpose -----------------
  async function useVersion(alt: FrameConflictValue, how: "replace" | "insertAfter") {
    if (!objectId || !frameId) return;
    if (!commit()) return;
    try { await session.useVersion(objectId, frameId, alt, how); } catch (e) { onnotice(reason(e), "warn"); }
  }

  // --- Saves: retry the same request, re-author on purpose, or discard on purpose ---------------
  function retrySave(s: SaveRecord) { guard(() => session.retry(s.id)); }
  function reauthorSave(s: SaveRecord) { guard(() => session.reauthor(s.id)); }
  function overlayRetryOf(s: SaveRecord) { return s.kind === "frame" || s.kind === "overlayApply" ? s.overlayRetry : null; }
  // Reauthor routes an edit through its own target's route now: an Index record (the listing half
  // of a rename, shown in this document's editor) through the Index's, not this document's.
  function refusalFor(s: SaveRecord): string {
    void studio.rev;
    return s.kind === "applyIndex" || s.object === null ? session.editRefusal(null) : readOnlyWhy;
  }
  // Armed for one record in one state. A record whose status changes since (a held edit that Save
  // has since sent and left uncertain, say) needs a fresh confirmation: the first click armed the
  // discard of something that may now have committed.
  let discardArmed = $state("");
  const armKey = (s: SaveRecord) => `${s.id}:${s.status}:${s.mayHaveCommitted}`;
  /** The armed confirmation. A Score write takes the rest of its gesture with it (the clear of the
   *  note's old place, the phrase's other notes), so the button says how many edits go. */
  const discardConfirm = (s: SaveRecord) => { const n = session.discardCount(s.id); return n > 1 ? `Discard ${n} edits?` : "Really discard?"; };
  function discardSave(s: SaveRecord) {
    if (discardArmed !== armKey(s)) { discardArmed = armKey(s); return; }
    discardArmed = "";
    session.discard(s.id);
    if (s.kind === "frame" && s.frame === frameId) { rasterRef = ""; loadRaster(); }
  }
  // How a failed save is moving: a countdown to its next automatic resend, "now" while it is being
  // sent again, or nothing when only the member can move it. `tick` redraws the countdown each second.
  function retryState(s: SaveRecord): { retrying: boolean; next: string } {
    void tick; void studio.rev;
    if (s.status === "queued" || s.status === "inflight" || session.resaving(s.id)) return { retrying: true, next: "Trying again now" };
    const at = session.autoRetryAt(s.id);
    return at === null ? { retrying: false, next: "" } : { retrying: true, next: retryText(at, session.now()) };
  }
  // While the Studio waits to switch channel: the previous channel's edits that will not save on
  // their own. Retry and discard act on all of them at once, the discard behind one confirmation
  // that lapses if that set changes.
  function stuckRecords(): SaveRecord[] {
    void studio.rev;
    return session.saves.filter(s => (s.status === "blocked" || s.status === "uncertain") && session.autoRetryAt(s.id) === null && !session.resaving(s.id));
  }
  const stuckArmKey = () => `stuck:${stuckRecords().map(armKey).join(",")}`;
  // Only what a retry can move: a spent local-draft ticket or a changed history would be a no-op.
  function retryStuck() { for (const s of stuckRecords()) if (session.canRetry(s.id)) retrySave(s); }
  function discardStuck() {
    const key = stuckArmKey();
    if (discardArmed !== key) { discardArmed = key; return; }
    discardArmed = "";
    for (const s of stuckRecords()) {
      session.discard(s.id);
      if (s.kind === "frame" && s.frame === frameId) { rasterRef = ""; loadRaster(); } // as discardSave
    }
  }
  // A lock snapshot restored for another channel (see StudioSession.pendingRestoredScope).
  const restoredElsewhere = $derived.by(() => { void studio.rev; return session.pendingRestoredScope; });
  const restoredArmKey = () => `restored:${restoredElsewhere?.server}:${restoredElsewhere?.channel}`;
  function discardRestored() {
    if (discardArmed !== restoredArmKey()) { discardArmed = restoredArmKey(); return; }
    discardArmed = "";
    session.discardRestoredDraft();
  }

  // --- Index entry: expiry and deletion are Index operations ------------------------------------
  function setExpiry(e: NativeExpiry) { if (objectId) guard(() => session.setEntryExpiry(objectId, e)); }
  const destructiveScopeKey = $derived.by(() => {
    void studio.rev;
    return session.scope ? `${session.scope.server}:${session.scope.channel}` : "";
  });
  const deleteKey = $derived(objectId && destructiveScopeKey ? `${destructiveScopeKey}:${objectId}` : "");
  let deleteArmed = $state("");
  function deleteFlipnote() {
    if (!objectId || deleteArmed !== deleteKey) return;
    if (!commit()) return;
    guard(() => { session.deleteEntry(objectId); deleteArmed = ""; studio.selected = ""; });
  }
  function expiryDate(e: NativeExpiry): string {
    if (e.kind !== "at") return "";
    const d = new Date(e.ms);
    return Number.isFinite(d.getTime()) ? d.toISOString().slice(0, 10) : "";
  }
  function expiryText(e: NativeExpiry): string {
    if (e.kind === "never") return "keeps forever";
    if (e.kind === "unrecorded") return "no expiry recorded";
    const d = Math.ceil((e.ms - Date.now()) / 86_400_000);
    return d <= 0 ? `expired ${new Date(e.ms).toLocaleDateString()}` : `expires in ${d}d`;
  }

  // --- Retained local draft ----------------------------------------------------------------------
  const overlay = $derived.by(() => { void studio.rev; return session.overlay?.object === objectId ? session.overlay : null; });
  const draftChoices = $derived.by(() => { void studio.rev; return session.overlayChoices("copy"); });
  const draftDestructiveKey = $derived.by(() => {
    const branch = overlay?.lifecycle?.branch;
    return objectId && destructiveScopeKey && branch ? `${destructiveScopeKey}:${objectId}:${branch.branch}:${branch.content}` : "";
  });
  const archiveDestructiveKey = $derived.by(() => {
    const archive = overlay?.archive?.archive;
    return objectId && destructiveScopeKey && archive ? `${destructiveScopeKey}:${objectId}:${archive}` : "";
  });
  const copyForgetKey = $derived.by(() => {
    const copy = overlay?.copy;
    return objectId && destructiveScopeKey && copy?.status === "uncertain" && copy.edit
      ? `${destructiveScopeKey}:${objectId}:copy:${copy.edit.nonce}`
      : "";
  });
  let draftDiscardArmed = $state("");
  let archiveReleaseArmed = $state("");
  let copyForgetArmed = $state("");
  // A confirmation is consent for one observed target/version, not a sticky UI mode. Clear it as
  // soon as navigation, scope, branch content or archive identity changes—even if the user later
  // returns to the same target.
  $effect(() => {
    const deleteNow = deleteKey;
    const draftNow = draftDestructiveKey;
    const archiveNow = archiveDestructiveKey;
    const copyNow = copyForgetKey;
    untrack(() => {
      if (deleteArmed && deleteArmed !== deleteNow) deleteArmed = "";
      if (draftDiscardArmed && draftDiscardArmed !== draftNow) draftDiscardArmed = "";
      if (archiveReleaseArmed && archiveReleaseArmed !== archiveNow) archiveReleaseArmed = "";
      if (copyForgetArmed && copyForgetArmed !== copyNow) copyForgetArmed = "";
    });
  });
  async function draftAction(action: () => Promise<unknown>, success: string) {
    try { await action(); onnotice(success, "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function downloadDraftExport() {
    const x = overlay?.exported;
    if (!x?.bytesB64) return;
    try {
      const url = URL.createObjectURL(new Blob([base64ToBytes(x.bytesB64)], { type: "application/octet-stream" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `${(model?.title || "draft").replace(/[^\w.-]+/g, "_")}.catcoms-studio-draft-v1.bin`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) { onnotice(`could not hand the draft archive to the browser: ${reason(e)}`, "warn"); }
  }
  async function copyDraftExport() {
    const x = overlay?.exported;
    if (!x?.bytesB64) return;
    try { await navigator.clipboard.writeText(x.bytesB64); onnotice("local draft archive copied as base64", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function copyDraftItem(choice: (typeof draftChoices)[number]) {
    if (!objectId) return;
    try { await session.copyOverlayChoice(objectId, choice.choice, choice.label, "copy"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function retryDraftCopy() {
    if (!objectId) return;
    try { await session.retryOverlayCopy(objectId); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function repreviewDraftCopy() {
    if (!objectId) return;
    try { await session.repreviewOverlayCopy(objectId); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function cancelRefusedDraftCopy(expectedNonce: string) {
    if (!objectId) return;
    if (session.cancelRefusedOverlayCopy(objectId, expectedNonce)) {
      onnotice("cancelled the refused copy; the backend confirmed that it wrote nothing", "info");
    }
  }
  function forgetDraftCopy() {
    // This handler is mounted only for the armed confirmation state. The session performs the
    // final status check, so a stale click can never remove a resolved or replacement operation.
    if (!objectId) return;
    const forgotten = session.forgetOverlayCopy(objectId, copyForgetArmed.slice(-32));
    copyForgetArmed = "";
    if (forgotten) onnotice("stopped tracking the uncertain copy; it may already have landed", "warn");
  }
  function words(value: string): string { return value.replace(/[A-Z]/g, (c) => ` ${c.toLowerCase()}`); }

  // --- Recovery rail -------------------------------------------------------------------------------
  const listing = $derived.by(() => { void studio.rev; return session.recoveryTarget === objectId ? session.recoveryListing : null; });
  const recoveryError = $derived.by(() => { void studio.rev; return session.recoveryError; });
  const run = $derived.by(() => { void studio.rev; return session.recoveryRun; });
  const pointer = $derived.by(() => { void studio.rev; return session.pointer; });
  let inspected = $state("");
  let copyConfirm = $state("");
  const inspectedVersion = $derived.by(() => { void studio.rev; return inspected ? session.versions.get(inspected) ?? null : null; });
  function exportFor(snapshot: string) { void studio.rev; return session.exports.get(snapshot) ?? null; }
  async function inspect(v: RecoveryVersion) {
    inspected = inspected === v.snapshot ? "" : v.snapshot;
    if (!inspected) return;
    try { await session.readVersion(v.snapshot); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function startRecovery(v: RecoveryVersion, mode: RecoveryMode) {
    if (mode === "copy" && copyConfirm !== v.snapshot) { copyConfirm = v.snapshot; return; }
    copyConfirm = "";
    if (!commit()) return;
    try { await session.save(); await session.runRecovery(v.snapshot, mode); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function exportVersion(v: RecoveryVersion) {
    try { await session.exportVersion(v.snapshot); onnotice("backup ready below · p1-recovery-v1, not a .pixa", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function saveExport(snapshot: string) {
    const x = session.exports.get(snapshot);
    if (!x) return;
    try {
      const url = URL.createObjectURL(new Blob([base64ToBytes(x.bytesB64)], { type: "application/octet-stream" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `${(model?.title || "index").replace(/[^\w.-]+/g, "_")}-epoch-${listing?.versions.find((v) => v.snapshot === snapshot)?.epoch ?? "x"}.p1-recovery-v1.bin`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) { onnotice(`could not hand the file to the browser: ${reason(e)}; copy it instead`, "warn"); }
  }
  async function copyExport(snapshot: string) {
    const x = session.exports.get(snapshot);
    if (!x) return;
    try { await navigator.clipboard.writeText(x.bytesB64); onnotice("backup copied as base64", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  async function acknowledge() {
    try { await session.acknowledgeEviction(); onnotice("eviction acknowledged: the named oldest version is removed at the next settlement pass", "info"); } catch (e) { onnotice(reason(e), "warn"); }
  }
  function deadlineText(deadlineMs: string): string {
    // Display only: the lossless decimal stays in the listing; the request never carries it.
    const ms = Number(deadlineMs);
    if (!Number.isFinite(ms)) return `at receiver clock ${deadlineMs}`;
    const left = ms - Date.now();
    if (left <= 0) return "at the next settlement pass";
    const d = Math.floor(left / 86_400_000), h = Math.floor((left % 86_400_000) / 3_600_000);
    return d ? `in ${d}d ${h}h` : `in ${h}h`;
  }
  function reasonText(r: RecoveryVersion["reason"]): string {
    return r === "excluded" ? "left out by a checkpoint" : r === "rewound" ? "rewound" : r === "conflictOverflow" ? "conflict overflow" : "repair";
  }
  function itemTone(s: RecoveryItem["state"]): string {
    if (s === "applied" || s === "alreadySaved" || s === "unchanged") return "ok";
    if (s === "error" || s === "deleted" || s === "missingTarget") return "danger";
    if (s === "conflict" || s === "full" || s === "uncertain") return "warn";
    return "info";
  }

  onMount(() => {
    const t = setInterval(() => { tick++; }, 1000);
    return () => clearInterval(t);
  });
  onDestroy(() => { removeCloseHandler(); removeSaveHandler(); auditionRequest++; audioActions?.stop(); commit(); claimActions?.release(); claimActions?.cancelAsk(); });

  // --- Keys --------------------------------------------------------------------------------------
  function onKey(e: KeyboardEvent) {
    // While the wait panel shows, the raster and document still belong to the last channel: an
    // undo or a Play here would act on something not on screen.
    if (scopeWaiting) return;
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    const k = e.key;
    if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === "z") { e.preventDefault(); if (e.shiftKey) doRedo(); else doUndo(); return; }
    if ((e.ctrlKey || e.metaKey) && k.toLowerCase() === "y") { e.preventDefault(); doRedo(); return; }
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const map: Record<string, () => void> = {
      b: () => (tool = "pen"), e: () => (tool = "eraser"), g: () => (tool = "fill"), l: () => (tool = "shape"),
      s: () => (tool = "stamp"), t: () => (tool = "text"), m: () => (mirror = { ...mirror, h: !mirror.h }),
      o: () => (onion = !onion), "#": () => (grid = !grid),
      "[": () => (brush = clampBrush(brush - 1)), "]": () => (brush = clampBrush(brush + 1)),
      "+": () => setZoom(zoom + 1), "=": () => setZoom(zoom + 1), "-": () => setZoom(zoom - 1),
      ",": () => step(-1), ".": () => step(1), " ": toggleStudioPlay,
    };
    if (map[k]) { e.preventDefault(); map[k](); return; }
    if (/^[0-9]$/.test(k) && raster) { e.preventDefault(); color = Math.min(raster.palette.length - 1, k === "0" ? 9 : Number(k) - 1); }
  }

  function fmtKib(n: number): string { return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} mib` : `${(n / 1024).toFixed(1)} kib`; }
  function rel(ts: number): string {
    void tick;
    const d = Date.now() - ts;
    if (d < 60_000) return "just now";
    if (d < 3_600_000) return `${Math.round(d / 60_000)} min ago`;
    if (d < 86_400_000) return `${Math.round(d / 3_600_000)}h ago`;
    return `${Math.round(d / 86_400_000)}d ago`;
  }
  function settlementChip(): { text: string; tone: string; title: string } {
    if (!doc) return { text: "", tone: "", title: "" };
    if (!view) {
      if (doc.error) return { text: "read failed", tone: "danger", title: doc.error };
      if (doc.absent && !pending.some((s) => s.kind === "create")) return { text: "not on this device", tone: "warn", title: "studio_read found no local copy; this is not a deletion and not proof nobody has it" };
      return { text: "reading…", tone: "", title: "" };
    }
    if (view.awaitingTenureReceipt) return { text: overlay?.inspection?.kind === "local-draft" ? "local-only draft" : "awaiting tenure", tone: "warn", title: "the base history is an unconfirmed preview; edits save only into a bounded local draft and grant no receipt, publication or authoring authority" };
    if (view.phase === "fault") return { text: "history fault", tone: "danger", title: "history fault: conflicting owner receipts" };
    if (view.phase === "closing") return { text: "rotating", tone: "warn", title: "rotating: durable edits resume when the owner settles this rotation" };
    if (view.phase === "settled") return { text: "Saved locally", tone: "ok", title: "Saved on this device" };
    return { text: "Saved locally", tone: "", title: "Saved on this device" };
  }
</script>

<!-- Hotkeys apply while focus is inside the editor (the tool rail, the canvas, the timeline);
     typing in one of its inputs is exempt inside onKey. -->
{#snippet pendingSaves()}
    {#if attention.length}
      {@const head = attention[0]}
      {@const motion = retryState(head)}
      <!-- One line for what happened, one for why and what happens next. The exact native text stays
           in the tooltip for diagnostics; the words come from studio-save-words.ts. A save the
           session is resending on its own is shown quietly; one only the member can move is a warning. -->
      <div class="st-banner st-save" class:warn={!motion.retrying} title={head.error}>
        <span><b>{saveHeadline(head.label, { retrying: motion.retrying, uncertain: head.mayHaveCommitted, others: attention.length - 1 + blockedCount })}</b></span>
        <span class="micro wrap">{saveProblemText(head.error, head.mayHaveCommitted)}{motion.next ? ` ${motion.next}.` : ""}</span>
        {#if (head.status === "uncertain" || head.status === "blocked") && !session.resaving(head.id)}<span class="st-card-acts">
          <!-- Retry only where a resend can change the outcome; after a changed history the way
               forward is a new edit (primary then), since the old request can no longer land. -->
          {#if session.canRetry(head.id)}
            <button type="button" class="st-btn primary" onclick={() => retrySave(head)}>{overlayRetryOf(head) === "afterExplicitAccess" ? "Reload and retry" : "Retry now"}</button>
          {/if}
          {#if (head.kind === "frame" || head.kind === "apply" || head.kind === "applyIndex" || head.kind === "overlayApply") && !refusalFor(head) && overlayRetryOf(head) !== "none" && session.canReauthor(head.id)}
            <button type="button" class="st-btn" class:primary={!session.canRetry(head.id)} onclick={() => reauthorSave(head)}>Save as a new edit</button>
          {/if}
          <button type="button" class="st-btn ghost" onclick={() => discardSave(head)}>{discardArmed === armKey(head) ? discardConfirm(head) : "Discard"}</button>
        </span>{/if}
      </div>
    {/if}
{/snippet}
<!-- Retained requests of a document with no model on screen: the read failed, the document is absent
     here, or it is still being read. Their controls never depend on that read, which is the thing
     recovery may be needed to get past (a restored queue can make leaving wait on them). Exact
     retry identity and the armed discard are the same as in pendingSaves. -->
{#snippet heldWork()}
        {#each attention.slice(0, 8) as s (s.id)}
          {@const motion = retryState(s)}
          <div class="st-card" class:warn={!motion.retrying} data-held-save={s.id} title={s.error}>
            <span>{saveHeadline(s.label, { retrying: motion.retrying, uncertain: s.mayHaveCommitted, others: session.blockedBehind(s.id) })}</span>
            <span class="micro wrap">{saveProblemText(s.error, s.mayHaveCommitted)}{motion.next ? ` ${motion.next}.` : ""}{session.needsDecision(s.id) && session.canReauthor(s.id) ? " It can be saved as a new edit once this document loads." : ""}</span>
            {#if s.status === "uncertain" || s.status === "blocked"}<span class="st-card-acts">
              {#if session.canRetry(s.id)}
                <button type="button" class="st-btn primary" onclick={() => retrySave(s)}>{overlayRetryOf(s) === "afterExplicitAccess" ? "Reload and retry" : "Retry now"}</button>
              {/if}
              <button type="button" class="st-btn ghost" onclick={() => discardSave(s)}>{discardArmed === armKey(s) ? discardConfirm(s) : "Discard"}</button>
            </span>{/if}
          </div>
        {/each}
        {#if attention.length > 8}<p class="micro">and {attention.length - 8} more</p>{/if}
        {@render heldQueuedCard()}
{/snippet}
{#snippet heldQueuedCard()}
    {#if heldQueued.length}
      <div class="st-card" data-held-queued>
        <span>{heldQueued.length} restored {heldQueued.length === 1 ? "edit" : "edits"} not saved yet · {heldQueued[0].label}</span>
        <span class="st-card-acts">
          <button type="button" class="st-btn primary" onclick={() => void session.saveDocument(objectId).catch(e => onnotice(reason(e), "warn"))}>Save</button>
          <button type="button" class="st-btn ghost" onclick={() => discardSave(heldQueued[0])}>{discardArmed === armKey(heldQueued[0]) ? discardConfirm(heldQueued[0]) : "Discard"}</button>
        </span>
      </div>
    {/if}
{/snippet}
<!-- The document's standing in shared history: one line, read from the view, never inferred. -->
{#snippet historyStatus()}
            {#if view}
              <p class="st-hist"><i class={view.awaitingTenureReceipt || view.phase === "closing" ? "warn" : view.phase === "fault" ? "danger" : "ok"}></i>
                {view.awaitingTenureReceipt ? "Local draft" : view.phase === "fault" ? "History needs repair" : view.phase === "closing" ? "Updating shared history" : "Saved on this device"}
              </p>
              {#if view.phase === "fault"}<p>Ask the owner to check Studio in Server settings.</p>{/if}
            {/if}
{/snippet}
<!-- The score composer's history tab: the same status line and rail the flipnote inspector shows. -->
{#snippet scoreHistory()}
  {@render historyStatus()}
  {@render historyRail()}
{/snippet}
<!-- Retained local drafts, retained versions and recovery walks: everything below the status. -->
{#snippet historyRail()}
            {#if overlay?.loading}
              <p class="micro wrap">checking this device for a retained local draft…</p>
            {:else if overlay?.error}
              <div class="st-card warn">
                <span>local draft status could not be read</span>
                <span class="micro wrap">{overlay.error}</span>
                <span class="st-card-acts"><button type="button" class="st-btn" onclick={() => objectId && session.refreshOverlay(objectId)}>retry status</button></span>
              </div>
            {:else if overlay?.inspection?.kind === "local-draft"}
              {@const draft = overlay.inspection}
              <div class="st-card warn">
                <span>local-only draft · {draft.accepted} saved {draft.accepted === 1 ? "edit" : "edits"} · {draft.transferState}</span>
                <span class="micro wrap">{draft.provenance === "unconfirmed" ? "created while the owner's tenure was unconfirmed" : "retained from a closing rotation"} · read-only evidence, not a receipt or shared publication{#if draft.eligibility === "manual" && draft.manualReason} · manual: {words(draft.manualReason)}{/if}{#if draft.handoffAttempt} · handoff {words(draft.handoffAttempt.state)} ({words(draft.handoffAttempt.reason)}){/if}</span>
                <span class="st-card-acts">
                  <button type="button" class="st-btn" onclick={() => objectId && draftAction(() => session.exportOverlay(objectId), "local draft export ready below")}>export</button>
                  <button type="button" class="st-btn" onclick={() => objectId && draftAction(() => session.archiveOverlay(objectId), "local draft preserved in an archive")}>preserve archive</button>
                  {#if overlay.archive}
                    <button type="button" class="st-btn ghost" onclick={() => objectId && draftAction(() => session.disposeOverlay(objectId, "preserve"), "live draft removed; archive preserved")}>remove live copy</button>
                  {/if}
                  {#if draftDestructiveKey && draftDiscardArmed === draftDestructiveKey}
                    <button type="button" class="st-btn danger" onclick={() => objectId && draftDiscardArmed === draftDestructiveKey && draftAction(() => session.disposeOverlay(objectId, "discard"), "local draft discarded")}>discard local draft for real</button>
                    <button type="button" class="st-btn ghost" onclick={() => (draftDiscardArmed = "")}>cancel</button>
                  {:else}
                    <button type="button" class="st-btn ghost" onclick={() => (draftDiscardArmed = draftDestructiveKey)}>discard…</button>
                  {/if}
                </span>
                {#if view && !view.awaitingTenureReceipt && view.phase !== "closing" && view.phase !== "fault" && draftChoices.length}
                  <span class="micro wrap">copy one retained value into the current document · this does not preserve or dispose the branch</span>
                  <span class="st-card-acts">
                    {#each draftChoices as choice (JSON.stringify(choice.choice))}
                      <button type="button" class="st-btn ghost" disabled={overlay.copy?.status === "previewing" || overlay.copy?.status === "applying" || overlay.copy?.status === "refused" || overlay.copy?.status === "uncertain"} onclick={() => copyDraftItem(choice)}>{choice.label}</button>
                    {/each}
                  </span>
                {/if}
              </div>
            {:else if overlay?.inspection?.kind === "disposed"}
              <p class="micro wrap">local draft disposed · {overlay.inspection.mode}{#if overlay.inspection.archive} · archive {overlay.inspection.archive.slice(0, 8)}{/if} · terminal local history, not absence</p>
            {:else if overlay?.lifecycle?.transferred}
              <p class="micro wrap">the retained draft was transferred into the successor history; refresh the document to see the installed result</p>
            {/if}
            {#if overlay?.copy}
              <div class="st-card warn">
                <span>retained copy {overlay.copy.label} · {words(overlay.copy.status)}</span>
                <span class="micro wrap">this operation remains independent of the source draft{#if overlay.copy.error} · {overlay.copy.error}{/if}</span>
                {#if overlay.copy.status === "uncertain"}
                  <span class="micro wrap">Stopping tracking does not undo the copy or prove that it never landed.</span>
                  <span class="st-card-acts">
                    <button type="button" class="st-btn primary" onclick={retryDraftCopy}>retry exact copy</button>
                    {#if copyForgetKey && copyForgetArmed === copyForgetKey}
                      <button type="button" class="st-btn danger" onclick={forgetDraftCopy}>stop tracking for real</button>
                      <button type="button" class="st-btn ghost" onclick={() => (copyForgetArmed = "")}>cancel</button>
                    {:else}
                      <button type="button" class="st-btn ghost" onclick={() => (copyForgetArmed = copyForgetKey)}>stop tracking…</button>
                    {/if}
                  </span>
                {:else if overlay.copy.status === "refused"}
                  <span class="st-card-acts">
                    <button type="button" class="st-btn primary" onclick={repreviewDraftCopy}>preview again as a new copy</button>
                    {#if overlay.copy.edit}
                      {#key overlay.copy.edit.nonce}
                        {@const refusedNonce = overlay.copy.edit.nonce}
                        <button type="button" class="st-btn ghost" onclick={() => cancelRefusedDraftCopy(refusedNonce)}>cancel this copy</button>
                      {/key}
                    {/if}
                  </span>
                {/if}
              </div>
            {/if}
            {#if overlay?.archive}
              <div class="st-card">
                <span>retained draft archive · {overlay.archive.accepted} edits · {fmtKib(Number(overlay.archive.physicalBytes ?? "0"))}</span>
                <span class="micro wrap">terminal read-only recovery evidence · no authoring authority · branch {overlay.archive.branch.slice(0, 8)}</span>
                <span class="st-card-acts">
                  <button type="button" class="st-btn" onclick={() => objectId && draftAction(() => session.exportOverlayArchive(objectId), "retained draft archive export ready below")}>export archive</button>
                  {#if archiveDestructiveKey && archiveReleaseArmed === archiveDestructiveKey}
                    <button type="button" class="st-btn danger" onclick={() => objectId && archiveReleaseArmed === archiveDestructiveKey && draftAction(() => session.releaseOverlayArchive(objectId), "archive released and status reconciled")}>release archive for real</button>
                    <button type="button" class="st-btn ghost" onclick={() => (archiveReleaseArmed = "")}>cancel</button>
                  {:else}
                    <button type="button" class="st-btn ghost" onclick={() => (archiveReleaseArmed = archiveDestructiveKey)}>release…</button>
                  {/if}
                </span>
              </div>
            {/if}
            {#if overlay?.exported?.bytesB64}
              <div class="st-card">
                <span>local draft backup ready · {fmtKib(overlay.exported.bytes ?? 0)}</span>
                <span class="micro wrap">{overlay.exported.format} · private operation history, not playable media or a .pixa</span>
                <span class="st-card-acts"><button type="button" class="st-btn" onclick={downloadDraftExport}>save file</button><button type="button" class="st-btn ghost" onclick={copyDraftExport}>copy base64</button></span>
              </div>
            {/if}
            {#if listing}
              {#if listing.evictionPending}
                <div class="st-card warn">
                  <span>a previous version will be removed {deadlineText(listing.evictionPending.deadlineMs)} unless exported</span>
                  <span class="micro wrap">oldest {listing.evictionPending.oldestSnapshot.slice(0, 8)} makes room for staged {listing.evictionPending.stagedSnapshot.slice(0, 8)} · acknowledging removes it now instead of at the deadline</span>
                  <span class="st-card-acts"><button type="button" class="st-btn" onclick={acknowledge}>acknowledge · remove now</button></span>
                </div>
              {/if}
              {#each listing.versions as v (v.snapshot)}
                <div class="st-ver" class:staged={v.staged}>
                  <p class="st-hist"><i class="info"></i>{v.staged ? "staged" : "previous"} version · epoch {v.epoch} · {reasonText(v.reason)} · {fmtKib(v.bytes)}</p>
                  <span class="st-card-acts">
                    <button type="button" class="st-btn ghost" onclick={() => inspect(v)}>{inspected === v.snapshot ? "hide" : "inspect"}</button>
                    <button type="button" class="st-btn" disabled={!!readOnlyWhy || run?.status === "running"} title="add what is missing; never overwrites" onclick={() => startRecovery(v, "restore")}>restore</button>
                    <button type="button" class="st-btn" disabled={!!readOnlyWhy || run?.status === "running"} title={isScore ? "may replace same-coordinate cells and recipes, title/BPM/beats/steps and apply retained removals; asks first, each item is previewed separately" : "replace current title/fps and apply the fork's deletions; asks first"} onclick={() => startRecovery(v, "copy")}>{copyConfirm === v.snapshot ? "confirm copy" : "copy"}</button>
                    <button type="button" class="st-btn ghost" onclick={() => exportVersion(v)}>export</button>
                  </span>
                  {#if copyConfirm === v.snapshot}<p class="micro wrap">{#if isScore}Score copy may replace same-coordinate cells and recipes, title, BPM, beats per bar and Steps, and apply retained clear/remove choices. Each item is previewed and saved separately, not an atomic restore; conflicts, missing recipes or out-of-length cells may still need attention. Press confirm copy.{:else}Copy may replace title, fps, same-ID sound markers, recipes and the linked Score, and apply retained deletions. PIXA births stay immutable; deleted IDs are never resurrected. Each item is previewed and saved separately, not atomically; missing media or prerequisites may remain blocked. Press confirm copy.{/if}</p>{/if}
                  {#if inspected === v.snapshot}
                    {#if inspectedVersion}
                      {@const c = inspectedVersion.content}
                      <p class="micro wrap">{#if c.kind === "flipnote"}title "{c.title?.selected.value ?? ""}" · {c.fps?.selected.value ?? 12} fps · {c.timeline.length} frames · {Object.keys(c.tombstones).length} deleted · {fmtKib(c.declaredFrameBytes)} declared{:else if c.kind === "score"}score "{c.title?.selected.value ?? ""}" · {c.bpm?.selected.value ?? 120} BPM · {Object.keys(c.cells).length + Object.keys(c.layers ?? {}).length} retained note slots{:else}index · {Object.keys(c.objects).length} entries · {Object.keys(c.overflow).length} overflow · {Object.keys(c.deletedObjects).length} deleted{/if} · historical, not the current view</p>
                    {:else}
                      <p class="micro wrap">reading…</p>
                    {/if}
                  {/if}
                  {#if exportFor(v.snapshot)}
                    {@const x = exportFor(v.snapshot)!}
                    <div class="st-card">
                      <span>backup ready · {fmtKib(x.bytes)}</span>
                      <span class="micro wrap">{x.format} · private history and operation evidence for recovery, not playable media or a .pixa</span>
                      <span class="st-card-acts"><button type="button" class="st-btn" onclick={() => saveExport(v.snapshot)}>save file</button><button type="button" class="st-btn ghost" onclick={() => copyExport(v.snapshot)}>copy base64</button></span>
                    </div>
                  {/if}
                </div>
              {:else}
                <p class="micro wrap">No previous versions.</p>
              {/each}
              {#if run && run.object === objectId}
                <div class="st-card">
                  <span>{run.mode} from epoch {listing.versions.find((v) => v.snapshot === run.snapshot)?.epoch ?? "?"} · {run.status === "running" ? "in progress" : run.status === "stopped" ? `stopped${run.error ? ` · ${run.error}` : ""}` : "walked"}</span>
                  <span class="micro wrap">each choice is previewed, applied only when ready, then the document is re-read · what was applied is saved content, not an all-or-nothing restore</span>
                  <ul class="st-run">
                    {#each run.items as it, i (i)}
                      <li><i class={itemTone(it.state)}></i><span>{it.label}</span><span class="grow"></span><span class="micro">{it.state}{#if it.error} · {it.error}{/if}</span></li>
                    {/each}
                  </ul>
                  {#if run.status === "running"}<span class="st-card-acts"><button type="button" class="st-btn ghost" onclick={() => session.stopRecovery()}>stop after this item</button></span>{/if}
                </div>
                {#if run.status !== "running"}
                  <div class="st-card">
                    <span>registry pointer · {pointer.status === "done" ? "restored" : pointer.status === "blocked" ? "blocked" : pointer.status === "running" ? "restoring…" : "pending"}</span>
                    <span class="micro wrap">{#if pointer.status === "done" && pointer.result}checkpoint epoch {pointer.result.checkpointEpoch} · registry epoch {pointer.result.registryEpochId.slice(0, 8)} · provisional{:else if pointer.status === "blocked"}{pointer.error} · content already saved stays saved; retry separately{:else}a separate step that makes the saved document discoverable again; retryable on its own{/if}</span>
                    <span class="st-card-acts"><button type="button" class="st-btn" disabled={pointer.status === "running"} onclick={() => session.restorePointer(objectId)}>{pointer.status === "blocked" ? "retry pointer" : "restore pointer"}</button></span>
                  </div>
                {/if}
              {/if}
            {:else if recoveryError}
              <p class="micro wrap">recovery listing failed: {recoveryError}</p>
            {/if}
{/snippet}
<svelte:window onkeydown={(e) => { if (rootEl && rootEl.contains(document.activeElement)) onKey(e); }} />
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="studio" bind:this={rootEl} role="application" aria-label="Studio editor" tabindex="0">
  {#if restoredElsewhere}
    <!-- A lock snapshot restored for a channel that is not this one. Until that channel opens, the
         session refuses new edits everywhere; if it can no longer be opened, this is the way out. -->
    <div class="st-card warn" data-restored-elsewhere>
      <span>Unsaved edits from another channel are waiting. Open that channel to save them.</span>
      <span class="st-card-acts">
        <button type="button" class="st-btn ghost" onclick={discardRestored}>{discardArmed === restoredArmKey() ? "Really discard?" : "Discard them"}</button>
      </span>
    </div>
  {/if}
  {#if scopeWaiting}
    <!-- The previous channel's edits are still saving. The session stays on that channel until they
         land and then switches here by itself; the member only needs to act on an edit that will not
         save on its own, and the words say which of the two this is. -->
    {@const stuck = stuckRecords()}
    <div class="studio-empty" data-scope-wait>
      <!-- "Need you" once nothing will resend on its own: what is left queued behind a stuck edit
           cannot run either. -->
      <p>{waitTally && waitTally.stuck > 0 && waitTally.retrying === 0 ? "Your edits from the last channel need you." : "Saving your edits from the last channel."}</p>
      {#if waitTally?.left}<p class="micro">{waitTally.left} left</p>{/if}
      {#if stuck.length}
        <p class="micro">{stuck.slice(0, 4).map(s => s.label).join(", ")}{stuck.length > 4 ? `, ${stuck.length - 4} more` : ""}</p>
        <!-- No bulk "save as a new edit" here: the document changed under those edits, and saving them
             again unseen could overwrite newer work. They are reviewed in their own channel. -->
        {#if stuck.some(s => session.canReauthor(s.id))}<p class="micro">Edits the document changed under can be saved again from the last channel.</p>{/if}
        <span class="st-card-acts">
          {#if stuck.some(s => session.canRetry(s.id))}<button type="button" class="st-btn primary" onclick={retryStuck}>Retry</button>{/if}
          <button type="button" class="st-btn ghost" onclick={discardStuck}>{discardArmed === stuckArmKey() ? "Really discard?" : "Discard"}</button>
        </span>
      {/if}
    </div>
  {:else if !objectId || (!model && !doc)}
    <div class="studio-empty">
      {#if !hasScope}
        <p class="muted">Open a channel to see its studio.</p>
      {:else}
        <p class="muted">Pick a flipnote on the left, or start a new one.</p>
      {/if}
    </div>
  {:else if isScore}
    <!-- The score composer owns the whole surface below the surface strip: header, grid,
         inspector (whose history tab renders this component's history rail) and transport.
         Pending-save and media banners stay here because the session, not the composer, owns them. -->
    <div class="st-score">
      {@render pendingSaves()}{@render heldQueuedCard()}
      {#if mediaReceipt}<div class="st-banner info wrap">{mediaReceipt}</div>{/if}
      {#key objectId}
        <ScoreEditor model={score} readonlyReason={readOnlyWhy} loading={doc?.loading ?? false} error={doc?.error ?? ""} {patchLibrary} library={patchShelves} {who} {tint}
          onop={(op, gesture) => guard(() => session.applyScoreOp(objectId, op, gesture))} onroom={() => session.draftRoom()} onaddpatch={addStudioPatch}
          selection={scoreSelectionMemory.recall(objectId)} onselection={(selection) => scoreSelectionMemory.remember(objectId, selection)}
          onplay={audioActions ? toggleStudioPlay : undefined} onstop={() => audioActions?.stop()} {playing} nextPass={playing && !!audioState.nextPass} refused={playing ? audioState.refused ?? 0 : 0} positionMs={playing ? audioState.elapsedMs : null}
          {loop} onloop={(next) => { loop = next; }} onaudition={audioActions ? auditionSound : undefined} onauditionpad={audioActions ? auditionPad : undefined} onnew={openSoundfont}
          onexport={() => downloadMedia()} onshare={onshareScore ? () => downloadMedia("share") : undefined} onpost={onpostScore ? () => downloadMedia("post") : undefined} {mediaBusy}
          history={scoreHistory} onnotice={(text, kind) => onnotice(text, kind === "error" ? "error" : "info")} />
      {/key}
    </div>
  {:else if !model}
    <div class="studio-empty">
      {#if doc?.error}
        <div class="st-banner danger">could not read this flipnote: {doc.error} <button type="button" class="st-btn" onclick={() => session.open(objectId)}>retry</button></div>
      {:else if doc?.absent && !pending.some((s) => s.kind === "create")}
        <p class="muted">This flipnote is not on this device yet. Absence is not a deletion: it may still arrive from another member.</p>
        <button type="button" class="st-btn" onclick={() => session.open(objectId)}>read again</button>
      {:else}
        <p class="muted">{pending.some((s) => s.kind === "create") ? "Creating…" : "Reading…"}</p>
      {/if}
      {@render heldWork()}
    </div>
  {:else}
    <!-- Header -->
    <div class="st-head">
      <input class="st-title" value={model.title} placeholder="untitled" disabled={!!readOnlyWhy} onchange={(e) => setTitle(e.currentTarget.value)} aria-label="Flipnote title" />
      {#if model.titleConflicts.length}<span class="st-chip warn" title="another title was set at the same time by {model.titleConflicts.map((c) => who(c.source.author)).join(', ')}: {model.titleConflicts.map((c) => c.value).join(' / ')}"><i></i><span class="micro">title conflict</span></span>{/if}
      <span class="micro">flipnote · {raster?.w ?? FLIPNOTE_W}×{raster?.h ?? FLIPNOTE_H}</span>
      <label class="st-chip" title="Frames per second, 1 to 24{model.fpsConflicts.length ? ' · another value was set concurrently' : ''}">
        <input type="number" min={FLIPNOTE_FPS_MIN} max={FLIPNOTE_FPS_MAX} value={model.fps} disabled={!!readOnlyWhy} onchange={(e) => setFps(Number(e.currentTarget.value))} />
        <span class="micro">fps</span>
      </label>
      <span class="st-chip {settlementChip().tone}" title={settlementChip().title}><i></i><span class="micro">{settlementChip().text}</span></span>
      {#if inflight}<span class="st-chip"><span class="micro">{pending.some(s => s.status === "inflight") ? "Saving…" : "Unsaved changes"}</span></span>{/if}
      <button type="button" class="st-btn" onclick={() => void session.saveDocument(objectId).catch(e => onnotice(reason(e), "warn"))}>Save</button>
      <span class="grow"></span>
      <button type="button" class="st-btn" disabled={!audioActions} onclick={toggleStudioPlay}>{playing ? "stop" : "play"}</button>
      <button type="button" class="st-btn" title="Attach this flipnote to a chat message" disabled={mediaBusy || playing || !onpostPixa} onclick={() => downloadMedia("post")}>
        <svg viewBox="0 0 16 16" style="width: 11px; height: 11px"><path d="M2 8l12-6-4 12-2.5-4.5z"></path></svg>
        Post to chat
      </button>
      <button type="button" class="st-btn" disabled={mediaBusy || playing} onclick={() => downloadMedia()}>download .pixa</button>
      <button type="button" class="st-btn" disabled={mediaBusy || playing} onclick={() => exportMedia("gif")}>save GIF</button>
    </div>
    {#if mediaReceipt}<div class="st-banner info wrap">{mediaReceipt}</div>{/if}

    {@render pendingSaves()}{@render heldQueuedCard()}
    {#if overCapCount}
      <div class="st-banner warn">document full: {overCapCount} frame{overCapCount === 1 ? "" : "s"} past the {FLIPNOTE_MAX_FRAMES}-frame list or the 8 MiB promise. Editing and playback/export are refused until they are trimmed.</div>
    {:else if readOnlyWhy}
      <div class="st-banner {view?.phase === 'fault' ? 'danger' : 'warn'}">{readOnlyWhy}</div>
    {/if}
    {#if receivePaused}
      <div class="st-banner warn">Studio sync is paused. <button type="button" class="st-btn" onclick={() => { commit(); session.open(objectId); }}>Retry</button>{#if receivePauseDetail}<details><summary>Details</summary>{receivePauseDetail}</details>{/if}</div>
    {/if}

    <!-- Editor row -->
    <div class="st-row">
      <div class="st-tools">
        {#each [["pen", "b"], ["eraser", "e"], ["fill", "g"], ["shape", "l"], ["stamp", "s"], ["text", "t"]] as [t, key]}
          <button type="button" class="st-tool" class:on={tool === t} title="{t} [{key}]" onclick={() => (tool = t as Tool)}>
            {#if t === "pen"}<svg viewBox="0 0 16 16"><path d="M3 13l1-4 7-7 3 3-7 7-4 1z"></path><path d="M9.5 3.5l3 3"></path></svg>
            {:else if t === "eraser"}<svg viewBox="0 0 16 16"><path d="M9 3l4 4-6 6H4.5L2.5 11z"></path><path d="M6 8l3.5 3.5M3 14h10"></path></svg>
            {:else if t === "fill"}<svg viewBox="0 0 16 16"><path d="M6.5 2.5l6 6-4.5 4.5L2.5 7.5z"></path><path d="M2.5 7.5h8"></path><path d="M13.5 10.5c0 1-.8 2-1 2s-1-1-1-2 1-2 1-2 1 1 1 2z"></path></svg>
            {:else if t === "shape"}<svg viewBox="0 0 16 16"><path d="M2.5 13.5l5-11 5 11z"></path><circle cx="11.5" cy="11.5" r="2.5"></circle></svg>
            {:else if t === "stamp"}<svg viewBox="0 0 16 16"><path d="M5.5 8.5V5.5a2.5 2.5 0 015 0v3"></path><path d="M3 8.5h10v2.5H3z"></path><path d="M4.5 11v2h7v-2"></path></svg>
            {:else}<svg viewBox="0 0 16 16"><path d="M3 4h10M8 4v9M6 13h4"></path></svg>{/if}
            <span class="k">{key}</span>
          </button>
        {/each}
        <button type="button" class="st-tool" class:on={mirror.h} title="mirror [m]" onclick={() => (mirror = { ...mirror, h: !mirror.h })}><svg viewBox="0 0 16 16"><path d="M8 2v12" stroke-dasharray="2 2"></path><path d="M5.5 5l-3 3 3 3M10.5 5l3 3-3 3"></path></svg><span class="k">m</span></button>
        <span class="st-sep"></span>
        <button type="button" class="st-tool" class:on={onion} title="onion skin [o]" onclick={() => (onion = !onion)}><svg viewBox="0 0 16 16"><rect x="2" y="2" width="8" height="8" rx="1"></rect><rect x="6" y="6" width="8" height="8" rx="1"></rect></svg><span class="k">o</span></button>
        <button type="button" class="st-tool" class:on={grid} title="gridlines [#]" onclick={() => (grid = !grid)}><svg viewBox="0 0 16 16"><rect x="2" y="2" width="12" height="12" rx="1"></rect><path d="M6 2v12M10 2v12M2 6h12M2 10h12"></path></svg><span class="k">#</span></button>
        <span class="grow"></span>
        <button type="button" class="st-tool" title="undo [ctrl+z]" disabled={!undo.canUndo} onclick={doUndo}><svg viewBox="0 0 16 16"><path d="M6 4L3 7l3 3"></path><path d="M3 7h6.5a3.5 3.5 0 010 7H7"></path></svg></button>
        <button type="button" class="st-tool" title="redo [ctrl+shift+z]" disabled={!undo.canRedo} onclick={doRedo}><svg viewBox="0 0 16 16"><path d="M10 4l3 3-3 3"></path><path d="M13 7H6.5a3.5 3.5 0 000 7H9"></path></svg></button>
      </div>

      <div class="st-canvas-card">
        <div class="st-canvas-scroll">
        <div class="st-canvas-wrap" class:locked={!canEdit} style="width: {fitCanvas ? '100%' : `${(raster?.w ?? FLIPNOTE_W) * zoom}px`}; aspect-ratio: {raster?.w ?? FLIPNOTE_W} / {raster?.h ?? FLIPNOTE_H}" onwheel={(e) => { if (e.ctrlKey) { e.preventDefault(); setZoom(zoom + (e.deltaY < 0 ? 1 : -1)); } }}>
          <canvas bind:this={canvasEl} class="st-canvas"></canvas>
          <canvas
            bind:this={overlayEl}
            class="st-overlay"
            onpointerdown={onDown}
            onpointermove={onMove}
            onpointerup={onUp}
            onpointercancel={onUp}
            onpointerleave={onLeave}
            oncontextmenu={(e) => e.preventDefault()}
          ></canvas>
          {#if !frameId}
            <div class="st-veil"><span>no frames yet</span><span class="micro">add one below to start drawing</span></div>
          {:else if !raster && frameRec}
            {#if blobStateHere === "invalid" || blobStateHere === "unavailable" || blobStateHere === "failed"}
              <div class="st-veil"><span>{blobStateHere === "invalid" ? "these pixels were rejected" : blobStateHere === "failed" ? "fetching failed" : "pixels not available yet"}</span><span class="micro">{blobProblemOf(frameRec.cid, frameRec.bytes) || "bounded by the declared size"} · {fmtKib(frameRec.bytes)}</span><span class="st-veil-acts"><button type="button" class="st-btn" onclick={() => session.retryBlob(frameRec!.cid, frameRec!.bytes)}>ask again</button></span></div>
            {:else}
              <div class="st-veil"><span>fetching {fmtKib(frameRec.bytes)}</span><span class="micro">bounded by the declared size · validated before it is shown</span></div>
            {/if}
          {:else if pendingHere}
            <div class="st-veil soft"><span class="micro">{pendingHere.status === "uncertain" ? "This frame's save is not confirmed yet." : pendingHere.status === "blocked" ? "This frame is not saved yet." : "Saving this frame…"}</span></div>
          {/if}
          <span class="st-readout left">{hover ? `${hover[0]},${hover[1]}` : ""}{#if tool === "shape"} · {shape}{/if}</span>
          <span class="st-readout right">layer · {LAYER_NAMES[layer]}{#if dirty} · unsaved{:else if rasterRef === "unsaved"} · saving{/if}</span>
        </div>
        </div>
        <div class="st-canvas-foot">
          <span class="grow"></span>
          <span class="st-zoom">
            <button type="button" class="st-tile txt" title="Fit the whole frame" class:on={fitCanvas} onclick={() => fitCanvas = true}>Fit</button>
            <button type="button" class="st-tile txt" title="zoom out [-]" disabled={!fitCanvas && zoom <= ZOOM_MIN} onclick={() => setZoom(zoom - 1)}>-</button>
            <span class="mono">{fitCanvas ? "" : `${zoom}×`}</span>
            <button type="button" class="st-tile txt" title="zoom in [+]" disabled={zoom >= ZOOM_MAX} onclick={() => setZoom(zoom + 1)}>+</button>
          </span>
        </div>
      </div>

      <!-- Inspector -->
      <div class="st-inspector">
        <div class="st-itabs">
          {#each ["art", "sound", "music", "history"] as t}
            <button type="button" class="st-itab" class:on={inspectorTab === t} onclick={() => (inspectorTab = t as typeof inspectorTab)}>{t}</button>
          {/each}
        </div>
        <div class="st-ibody">
          {#if inspectorTab === "art"}
            <div class="st-sec">tool · {tool}</div>
            {#if tool === "shape"}
              <div class="st-opt"><span class="lb">shape</span>
                {#each ["line", "rect", "ellipse", "triangle"] as s}
                  <button type="button" class="st-tile" class:on={shape === s} onclick={() => (shape = s as Shape)} title={s}>
                    {#if s === "line"}<svg viewBox="0 0 16 16"><path d="M3 13L13 3"></path></svg>
                    {:else if s === "rect"}<svg viewBox="0 0 16 16"><rect x="2.5" y="3.5" width="11" height="9"></rect></svg>
                    {:else if s === "ellipse"}<svg viewBox="0 0 16 16"><ellipse cx="8" cy="8" rx="5.5" ry="4"></ellipse></svg>
                    {:else}<svg viewBox="0 0 16 16"><path d="M8 3l5.5 10h-11z"></path></svg>{/if}
                  </button>
                {/each}
              </div>
              <div class="st-opt"><span class="lb">fill</span>
                <button type="button" class="st-tile" class:on={!shapeFill} onclick={() => (shapeFill = false)} title="outline"><svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="5"></circle></svg></button>
                <button type="button" class="st-tile" class:on={shapeFill} onclick={() => (shapeFill = true)} title="filled"><svg viewBox="0 0 16 16" class="solid"><circle cx="8" cy="8" r="5.5"></circle></svg></button>
              </div>
            {:else if tool === "stamp"}
              <div class="st-opt wrap"><span class="lb">stamp</span>
                {#each STAMPS as s, i}
                  <button type="button" class="st-tile" class:on={stampIdx === i} onclick={() => (stampIdx = i)} title={s.name}>
                    <svg viewBox="0 0 8 8" shape-rendering="crispEdges" class="solid">
                      {#each Array.from(s.bm.pixels) as v, p}{#if v !== CLEAR}<rect x={p % 8} y={Math.floor(p / 8)} width="1" height="1" style="fill: {raster ? wellCss(raster.palette[v] ?? DEFAULT_PALETTE[v]) : 'currentColor'}"></rect>{/if}{/each}
                    </svg>
                  </button>
                {/each}
              </div>
              <p class="micro wrap">stamps from emoji/ arrive with the stamp editor; these are built in</p>
            {:else if tool === "text"}
              <div class="st-opt"><span class="lb">text</span><input class="st-input" bind:value={textDraft} maxlength="24" /></div>
              <p class="micro wrap">5×7 face · becomes pixels on place</p>
            {:else if tool === "pen" || tool === "eraser"}
              <div class="st-opt">
                <span class="lb">pen</span>
                <button type="button" class="st-tile txt" class:on={pressureOn} onclick={() => (pressureOn = !pressureOn)} title="A pen's pressure sets the size, up to the chosen size. A mouse always paints the chosen size.">pressure</button>
                <span class="micro">{penSeen ? "pen detected" : "no pen detected"}</span>
              </div>
            {/if}
            <div class="st-opt"><span class="lb">size</span>
              <span class="st-size">
                {#each Array.from({ length: BRUSH_MAX - BRUSH_MIN + 1 }, (_, i) => i + BRUSH_MIN) as s}
                  <button type="button" class:on={brush === s} style="height: {4 + s * 1.4}px" onclick={() => (brush = s)} aria-label="brush {s}"></button>
                {/each}
              </span>
              <span class="mono">{brush}</span>
              <span class="lb" style="width: auto; margin-left: 10px">mirror</span>
              <button type="button" class="st-tile txt" class:on={mirror.h} onclick={() => (mirror = { ...mirror, h: !mirror.h })}>h</button>
              <button type="button" class="st-tile txt" class:on={mirror.v} onclick={() => (mirror = { ...mirror, v: !mirror.v })}>v</button>
            </div>

            <div class="st-sec nxt">palette</div>
            {#each [colorGroups.grayscale, colorGroups.colors] as indices}
              <div class="st-wells" class:grays={indices === colorGroups.grayscale}>
                {#each indices as i}{@const e = displayedPalette[i]}
                  <button type="button" class="st-well" class:sel={color === i} style="background: {wellCss(e)}" title={paletteColorLabel(e)} aria-label={`Colour ${paletteColorLabel(e)}`} aria-pressed={color === i} onclick={() => (color = i)}></button>
                {/each}
              </div>
            {/each}
            {#if colorGroups.themed.length}<details class="st-theme-colors"><summary>Theme colours</summary>
              <div class="st-wells grays">{#each colorGroups.themed as i}{@const e = displayedPalette[i]}<button type="button" class="st-well" class:sel={color === i} style="background: {wellCss(e)}" title={PIX_ROLE_NAMES[e.role]} aria-label={PIX_ROLE_NAMES[e.role]} onclick={() => (color = i)}></button>{/each}</div>
              <label><input type="checkbox" bind:checked={adaptOn} /> Match my theme</label>
            </details>{/if}

            <div class="st-sec nxt">layers</div>
            {#each [2, 1, 0] as l}
              <div class="st-layer" class:on={layer === l}>
                <button type="button" class="eye" class:off={!layerVisible[l]} title="show / hide (view only)" onclick={() => { const v = [...layerVisible]; v[l] = !v[l]; layerVisible = v; }}>
                  <svg viewBox="0 0 16 16"><path d="M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8s-2.5 4.5-6.5 4.5S1.5 8 1.5 8z"></path><circle cx="8" cy="8" r="2"></circle>{#if !layerVisible[l]}<path d="M2 2l12 12"></path>{/if}</svg>
                </button>
                <!-- The whole row selects the layer: a stack glyph with this layer's sheet lit. -->
                <button type="button" class="pick" onclick={() => (layer = l)} title="draw on the {LAYER_NAMES[l]} layer">
                  <svg viewBox="0 0 16 16" class="stack">
                    <path d="M2 11.5l6 3 6-3" class:lit={l === 0}></path>
                    <path d="M2 8l6 3 6-3" class:lit={l === 1}></path>
                    <path d="M8 1.5l6 3-6 3-6-3z" class:lit={l === 2}></path>
                  </svg>
                  <span class="nm">{LAYER_NAMES[l]}</span>
                  <span class="what">{layer === l ? "drawing here" : ""}</span>
                </button>
              </div>
            {/each}

            <div class="st-sec nxt">frame {frameIndex >= 0 ? frameIndex + 1 : "·"}</div>
            {#if frameRec?.conflicts.length}
              <div class="st-card warn">
                <span>another version by {#each frameRec.conflicts as c, i}{i ? ", " : ""}<b style="color: {tint(c.author)}">{who(c.author)}</b>{/each}</span>
                <span class="micro">replaced this frame at the same time · nothing resolved silently · pick one on purpose</span>
                <span class="st-alt"><i style="background: {tint(frameRec.author)}"></i><span>showing {who(frameRec.author)} · {rel(frameRec.ts)} · {fmtKib(frameRec.bytes)}</span><span class="grow"></span><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion({ cid: frameRec!.cid, bytes: frameRec!.bytes, author: frameRec!.author, ts: frameRec!.ts, opId: frameRec!.opId }, "replace")}>keep this</button></span>
                {#each frameRec.conflicts as c (c.opId)}
                  <span class="st-alt"><i style="background: {tint(c.author)}"></i><span>{who(c.author)} · {rel(c.ts)} · {fmtKib(c.bytes)}</span><span class="grow"></span><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion(c, "replace")}>use this</button><button type="button" class="st-link" disabled={!!readOnlyWhy} onclick={() => useVersion(c, "insertAfter")}>keep both</button></span>
                {/each}
              </div>
            {/if}
            {#if pendingHere}
              <div class="st-card accent">
                <span>New frame: {#if pendingHere.mayHaveCommitted || pendingHere.status === "uncertain"}not confirmed{:else if pendingHere.status === "blocked"}not saved{:else}saving{/if}</span>
                <span class="micro it">it joins the timeline when the save lands</span>
              </div>
            {:else if frameRec}
              <p class="micro">by {who(frameRec.author)} · {rel(frameRec.ts)} · {fmtKib(frameRec.bytes)}{#if frameRec.insertions > 1} · inserted {frameRec.insertions}× concurrently{/if}{#if frameRec.overCap} · over cap{/if}</p>
            {/if}
            {#if claims && frameId && objectId}
              {#if frameClaims.contested}
                <div class="st-card warn">
                  <span>more than one person is drawing this frame</span>
                  <span class="micro">claims are advisory · nothing is locked · the last save of each frame is kept as another version</span>
                </div>
              {/if}
              {#if frameClaims.mine}
                <p class="micro">you are drawing this frame <button type="button" class="st-link" onclick={() => claimActions?.release()}>stop</button></p>
              {/if}
              {#each frameClaims.claimants as c (c.peer)}
                <p class="micro"><b style="color: {tint(c.peer)}">{who(c.peer)}</b> is drawing this frame · {c.secondsLeft} s</p>
              {/each}
              {#each frameClaims.askers as a (a.peer)}
                <p class="micro"><b style="color: {tint(a.peer)}">{who(a.peer)}</b> asked for this frame · {a.secondsLeft} s</p>
              {/each}
              {#if !frameClaims.mine}
                {#if frameClaims.claimants.length}
                  {#if frameClaims.myAsk !== null}
                    <p class="micro">you asked for it · {frameClaims.myAsk} s <button type="button" class="st-link" onclick={() => claimActions?.cancelAsk()}>cancel</button></p>
                  {:else}
                    <button type="button" class="st-btn ghost" title="ask them to pass this frame to you; you can still draw on it either way" onclick={() => claimActions?.ask(objectId, frameId)}>ask to draw it</button>
                  {/if}
                {:else}
                  <button type="button" class="st-btn ghost" title="tell the call you are drawing this frame (painting does this too)" onclick={() => claimActions?.claim(objectId, frameId)}>claim this frame</button>
                {/if}
              {/if}
            {/if}
          {:else if inspectorTab === "sound"}
            {#key objectId}<StudioSound model={model.sound} frame={frameId} frameNumber={frameIndex + 1} frameIndexOf={(id) => frameIds.indexOf(id) + 1}
              {who} {tint} readonlyReason={playing ? "Stop playback before editing" : readOnlyWhy} library={patchShelves}
              onpatch={addStudioPatch} onsfx={(sfx) => guard(() => session.setSfx(objectId, { ...sfx, id: sfx.id ?? session.newId(), frame: frameId }))}
              onremove={(id) => guard(() => session.removeSfx(objectId, id))} onremovepatch={(id) => guard(() => session.removeSoundPatch(objectId, id))}
              onaudition={auditionSound} onnew={openSoundfont} {onnotice} />{/key}
          {:else if inspectorTab === "music"}
            <StudioMusic model={model.sound} frameCount={frames.length} fps={model.fps} tracks={musicTracks} {patchLibrary} {who} {tint}
              readonlyReason={playing ? "Stop playback before editing" : readOnlyWhy}
              onlink={(id) => guard(() => session.linkScore(objectId, id))}
              onopen={(id) => { onnavigate?.(); void session.navigate(() => { session.open(id); studio.selected = id; }).catch(e => onnotice(reason(e), "warn")); }}
              oncreate={() => { onnavigate?.(); void session.navigate(() => { const id = session.createScore("New music track"); session.open(id, { read: false }); studio.selected = id; }).catch(e => onnotice(reason(e), "warn")); }} />
          {:else}
            <!-- History reads top to bottom as one story: what this flipnote is, the immutable
                 versions it has been saved as, the retained edits recovery can bring back, the
                 sound evidence behind the readiness flags, and finally the one destructive act. -->
            <div class="st-sec">this flipnote</div>
            {@render historyStatus()}
            <div class="st-bar"><i style="width: {Math.min(100, (totalBytes / FLIPNOTE_FRAME_BYTES_PROMISE) * 100)}%"></i></div>
            <p class="micro wrap">{fmtKib(totalBytes)} of 8 mib declared · {frames.length} of {FLIPNOTE_MAX_FRAMES} frames{#if model.deletedFrames.length} · {model.deletedFrames.length} deleted kept in history{/if}</p>
            {#if entry}
              <div class="st-opt"><span class="lb">expiry</span>
                <button type="button" class="st-tile txt" class:on={entry.expiry.kind === "never"} disabled={!!readOnlyWhy} onclick={() => setExpiry({ kind: "never" })}>keeps</button>
                <button type="button" class="st-tile txt" class:on={entry.expiry.kind === "unrecorded"} disabled={!!readOnlyWhy} onclick={() => setExpiry({ kind: "unrecorded" })}>unset</button>
                <input type="date" class="st-input date" class:on={entry.expiry.kind === "at"} value={expiryDate(entry.expiry)} disabled={!!readOnlyWhy} onchange={(e) => { const ms = Date.parse(e.currentTarget.value); if (Number.isFinite(ms)) setExpiry({ kind: "at", ms }); }} aria-label="expiry date" />
              </div>
              <p class="micro wrap">{expiryText(entry.expiry)}{#if entry.expiryConflicts} · another expiry was set at the same time{/if}</p>
            {:else}
              <p class="micro wrap">no entry in the channel index on this device{#if pending.some((s) => s.kind === "create")} · it appears when the create lands{/if}</p>
            {/if}

            <div class="st-sec nxt">versions <span class="n">· {model.sound.exports.length}</span></div>
            <button type="button" class="st-btn" style="align-self: flex-start" disabled={mediaBusy || playing} onclick={() => exportMedia("pixa")}>save a version</button>
            {#each model.sound.exports.slice(0, 8) as exported, i (exported.id)}
              <div class="st-ver">
                <p class="st-hist"><i class="info"></i>v{i + 1} · {fmtKib(exported.bytes)}{#if exported.expiry} · expires {new Date(exported.expiry).toLocaleDateString()}{/if}</p>
                <span class="st-card-acts">
                  <button type="button" class="st-link" disabled={mediaBusy || playing || !audioActions} onclick={() => publishedMedia(exported.id, "play")}>play</button>
                  <button type="button" class="st-link" disabled={mediaBusy} onclick={() => publishedMedia(exported.id, "download")}>download</button>
                  <button type="button" class="st-link" disabled={mediaBusy || !onsharePixa} onclick={() => publishedMedia(exported.id, "share")}>share</button>
                  <button type="button" class="st-link" disabled={!!readOnlyWhy || playing} onclick={() => guard(() => session.removePublishedPixa(objectId, exported.id))}>remove</button>
                </span>
                <details class="st-fold"><summary class="micro">file details</summary><code>{exported.cid}</code></details>
              </div>
            {/each}
            {#if savedPreview}<div class="st-card"><p class="micro wrap">saved version · the immutable package, not the timeline you are editing</p><canvas width={FLIPNOTE_W} height={FLIPNOTE_H} style="width:100%;image-rendering:pixelated" use:thumbBytes={{ bytes: savedPreview.frames[Math.min(savedPreview.frames.length - 1, Math.floor(audioState.elapsedMs * savedPreview.fps / 1000))], rev: 0 }}></canvas><button type="button" class="st-btn" onclick={() => { audioActions?.stop(); savedPreview = null; }}>close saved preview</button></div>{/if}

            <div class="st-sec nxt">previous edits</div>
            {@render historyRail()}

            <details class="st-fold">
              <summary class="st-sec nxt">evidence · sound</summary>
              <StudioSound history model={model.sound} frame={frameId} frameNumber={frameIndex + 1} frameIndexOf={(id) => frameIds.indexOf(id) + 1} {who} {tint} readonlyReason={readOnlyWhy} library={patchShelves}
                onpatch={addStudioPatch} onsfx={() => {}} onremove={(id) => guard(() => session.removeSfx(objectId, id))} onremovepatch={(id) => guard(() => session.removeSoundPatch(objectId, id))}
                onaudition={auditionSound} onnew={openSoundfont} {onnotice} />
            </details>
            <details class="st-fold">
              <summary class="st-sec nxt danger">danger</summary>
              {#if entry}
                <div class="st-card-acts">
                  {#if deleteArmed === deleteKey && deleteKey}
                    <button type="button" class="st-btn danger" onclick={deleteFlipnote}>delete this flipnote</button>
                    <button type="button" class="st-btn ghost" onclick={() => (deleteArmed = "")}>cancel</button>
                  {:else}
                    <button type="button" class="st-btn ghost" disabled={!!readOnlyWhy} onclick={() => (deleteArmed = deleteKey)}>delete…</button>
                  {/if}
                </div>
                <p class="micro wrap">removes it from the channel list · retained history stays recoverable</p>
              {:else}
                <p class="micro wrap">nothing to delete until the index entry lands</p>
              {/if}
            </details>
          {/if}
        </div>
      </div>
    </div>

    <!-- Timeline -->
    <div class="st-timeline">
      <div class="st-transport">
        <button type="button" class="st-tool sm" title="first frame" onclick={() => frameIds[0] && openFrame(frameIds[0])}><svg viewBox="0 0 16 16" class="solid"><path d="M8 3 2 8l6 5zM14 3 8 8l6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="previous frame [,]" onclick={() => step(-1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3 3h1.5v10H3zM12.5 3 6 8l6.5 5z"></path></svg></button>
        <button type="button" class="st-tool play" class:on={playing} disabled={!audioActions} title={playing ? "stop [space]" : "play [space]"} onclick={toggleStudioPlay}>
          {#if playing}<svg viewBox="0 0 16 16" class="solid"><path d="M3.5 2.5h3.5v11H3.5zM9 2.5h3.5v11H9z"></path></svg>{:else}<svg viewBox="0 0 16 16" class="solid"><path d="M4 2.5 13 8 4 13.5z"></path></svg>{/if}
        </button>
        <button type="button" class="st-tool sm" title="next frame [.]" onclick={() => step(1)}><svg viewBox="0 0 16 16" class="solid"><path d="M3.5 3 10 8l-6.5 5zM11.5 3H13v10h-1.5z"></path></svg></button>
        <button type="button" class="st-tool sm" title="last frame" onclick={() => frameIds.length && openFrame(frameIds[frameIds.length - 1])}><svg viewBox="0 0 16 16" class="solid"><path d="M2 3l6 5-6 5zM8 3l6 5-6 5z"></path></svg></button>
        <button type="button" class="st-tool sm" class:on={loop} disabled={playing} title="loop whole animation and authored sound after all release tails" onclick={() => (loop = !loop)}><svg viewBox="0 0 16 16"><path d="M3 8a5 5 0 015-5h4M13 8a5 5 0 01-5 5H4"></path><path d="M10.5 1.5 12 3l-1.5 1.5M5.5 11.5 4 13l1.5 1.5"></path></svg></button>
        <span class="mono strong">{frameIndex >= 0 ? frameIndex + 1 : "·"} / {frames.length}</span>
        <span class="micro">{((Math.max(0, frameIndex) + 1) / Math.max(1, model.fps)).toFixed(2)} s of {(frames.length / Math.max(1, model.fps)).toFixed(2)} s</span>
        <span class="grow"></span>
        {#each askNotices as a (a.peer + a.fr)}
          <span class="st-ask" role="status">
            <b style="color: {tint(a.peer)}">{who(a.peer)}</b> asks for frame {frameNumber(a.fr)} · {a.secondsLeft} s
            <button type="button" class="st-link" title="stop drawing this frame and let them have it (nothing is locked either way)" onclick={() => claimActions?.pass(a.obj, a.fr)}>pass</button>
          </span>
        {/each}
        <button type="button" class="st-btn ghost" disabled={!!readOnlyWhy || overCapCount > 0} onclick={() => addFrameAfter(false)}>+ frame after</button>
        <button type="button" class="st-btn ghost" disabled={!canEdit} onclick={() => addFrameAfter(true)}>duplicate</button>
        <button type="button" class="st-btn ghost" disabled={!frameId || !!readOnlyWhy || !!pendingHere} onclick={deleteFrame}>delete</button>
      </div>
      <div class="st-strip">
        {#each frames as f, i (f.id)}
          {@const st = blobStateOf(f.cid, f.bytes)}
          {@const fc = claimsByFrame.get(f.id) ?? []}
          {@const mineHere = claims?.local.claim?.obj === objectId && claims?.local.claim?.fr === f.id}
          <button type="button" class="st-thumb" class:cur={f.id === frameId} class:over={!!f.overCap} class:conflict={f.conflicts.length > 0} onclick={() => openFrame(f.id)} title={f.overCap ? "over the cap: skipped in playback, greyed until trimmed" : st === "invalid" ? `pixels rejected: ${blobProblemOf(f.cid, f.bytes)}` : st === "unavailable" ? "pixels not available yet" : ""}>
            <span class="ix">{i + 1}</span>
            <span class="fr" class:fetching={st === "fetching" || st === "queued" || st === "idle"} class:missing={st === "unavailable" || st === "invalid" || st === "failed"}>
              {#if st === "held"}
                <canvas width="64" height="48" use:thumb={{ cid: f.cid, bytes: f.bytes, rev: studio.rev }}></canvas>
              {:else if st === "invalid"}
                <span class="micro">rejected</span>
              {:else if st === "failed"}
                <span class="micro">failed</span>
              {:else if st === "unavailable"}
                <span class="micro">not here</span>
              {:else}
                <canvas width="64" height="48" use:thumb={{ cid: f.cid, bytes: f.bytes, rev: studio.rev }}></canvas>
                <span class="micro tag">fetching</span>
              {/if}
              <i class="dot" style="background: {tint(f.author)}"></i>
              {#if f.conflicts.length}<i class="corner"></i>{/if}
              {#if fc.length || mineHere}
                <i class="claim" class:contested={fc.length + (mineHere ? 1 : 0) > 1} style="background: {fc.length ? tint(fc[0].peer) : tint(me)}" title={mineHere ? (fc.length ? `you and ${claimTitle(fc)}` : "you are drawing this frame") : claimTitle(fc)}></i>
              {/if}
            </span>
          </button>
        {/each}
        {#each pendingInserts as s (s.id)}
          <button type="button" class="st-thumb" class:cur={s.frame === frameId} onclick={() => openFrame(s.frame)} title={s.mayHaveCommitted || s.status === "uncertain" ? `Not confirmed: ${saveProblemText(s.error, true)}` : s.status === "blocked" ? `Not saved: ${saveProblemText(s.error)}` : "Saving this new frame"}>
            <span class="ix">{s.status === "uncertain" || s.status === "blocked" ? "?" : "…"}</span>
            <span class="fr pending" class:uncertain={s.status === "uncertain" || s.status === "blocked"}>
              <canvas width="64" height="48" use:thumbBytes={{ bytes: s.pix, rev: studio.rev }}></canvas>
              <i class="dot" style="background: var(--accent)"></i>
            </span>
          </button>
        {/each}
      </div>
      <div class="st-sfxlane">
        <span class="micro lb">sfx</span>
        {#each frames as f (f.id)}
          {@const lane = laneMarkers(f.id)}
          <span class="cell" title={lane ? `${lane.count} sound marker${lane.count === 1 ? "" : "s"} · ${lane.label}` : ""}>
            {#if lane}<i class="pill" style="background: {tint(lane.author)}">{lane.label}{#if lane.count > 1} +{lane.count - 1}{/if}</i>{/if}
          </span>
        {/each}
      </div>
    </div>
  {/if}
  {#if soundfont}
    <StudioSoundfont name={soundfont.name} initial={soundfont.initial} startFrom={patchLibrary} {libraryCount} libraryMax={JAM_SAVED_PATCHES_MAX}
      targetName={model?.title || "this flipnote"} recipeCount={model?.sound.patchSlots.filter((s) => !s.removed).length ?? 0} recipeMax={FLIPNOTE_MAX_PATCHES} shareName={onsharePatch ? serverName : ""}
      bpm={score?.bpm ?? 120} onaudition={(patch, note, ms) => auditionSound(patch, note, ms)} onuse={useSoundfont} onsave={onsavePatch ?? refusePatch} onshare={shareSoundfont} {onnotice} onclose={() => (soundfont = null)} />
  {/if}
</div>

<style>
  /* The composer fills the column like the flipnote editor does; it scrolls as a whole when the
     window is shorter than its 25 rows plus transport. */
  .st-score { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  /* History folds: the summary IS the section rule, so a closed fold reads like a heading. */
  .st-fold > summary { cursor: pointer; list-style: none; display: flex; align-items: center; gap: 6px; }
  .st-fold > summary::-webkit-details-marker { display: none; }
  .st-fold > summary::after { content: ""; margin-left: auto; width: 6px; height: 6px; border-right: 1px solid var(--faint); border-bottom: 1px solid var(--faint); transform: rotate(45deg); }
  .st-fold[open] > summary::after { transform: rotate(-135deg); }
  .st-fold > summary.micro { padding: 2px 0; }
  .st-fold code { overflow-wrap: anywhere; font-size: .7rem; }
  .st-sec .n { color: var(--faint); font-weight: 400; }
  .st-sec.danger { color: var(--danger); }
  .studio { display: flex; flex-direction: column; flex: 1; min-height: 0; outline: none; gap: 0; }
  .studio-empty { padding: 2rem; display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .micro { font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--faint); white-space: nowrap; }
  .micro.wrap { white-space: normal; line-height: 1.5; margin: 4px 0 0; }
  .micro.it { font-style: italic; text-transform: none; letter-spacing: 0.04em; }
  .mono { font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); white-space: nowrap; }
  .mono.strong { color: var(--text); }
  .grow { flex: 1; }
  svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.5; stroke-linecap: round; stroke-linejoin: round; flex: none; }
  svg.solid { fill: currentColor; stroke: none; }

  /* Header: the channel head's bar, with the document's facts and view toggles. */
  .st-head { display: flex; align-items: center; gap: 0.6rem; padding: 0.4rem 0.6rem; background: var(--panel); border-bottom: 1px solid var(--border); margin: -0.3rem -0.5rem 0.2rem; flex-wrap: wrap; }
  .st-title { font-size: 0.95rem; font-weight: 600; color: var(--text); background: transparent; border: 1px solid transparent; padding: 2px 4px; width: 9rem; }
  .st-title:hover, .st-title:focus { border-color: var(--border); background: var(--bg-elev); }
  .st-title:disabled { color: var(--text-2); }
  .st-chip { display: inline-flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 999px; padding: 2px 9px; white-space: nowrap; }
  .st-chip i { width: 5px; height: 5px; border-radius: 50%; background: var(--muted); }
  .st-chip.ok { border-color: var(--ok-brd); background: var(--ok-dim); } .st-chip.ok .micro, .st-chip.ok i { color: var(--ok); background: var(--ok); }
  .st-chip.ok .micro { background: none; }
  .st-chip.warn { border-color: var(--warn-brd); background: var(--warn-dim); } .st-chip.warn .micro { color: var(--warn); } .st-chip.warn i { background: var(--warn); }
  .st-chip.danger { border-color: var(--danger-brd); background: var(--danger-dim); } .st-chip.danger .micro { color: var(--danger); } .st-chip.danger i { background: var(--danger); }
  .st-chip input { width: 2.2rem; background: transparent; border: none; color: var(--text-2); font-family: var(--mono); font-size: 0.68rem; padding: 0; text-align: right; }
  .st-tg { display: inline-flex; align-items: center; gap: 6px; background: transparent; border: none; padding: 0; color: inherit; }
  .st-tg i { width: 22px; height: 12px; border-radius: 999px; background: var(--bg-elev); border: 1px solid var(--border); position: relative; display: inline-block; }
  .st-tg i::after { content: ""; position: absolute; top: 1px; left: 1px; width: 8px; height: 8px; border-radius: 50%; background: var(--faint); }
  .st-tg.on i { background: var(--accent-dim); border-color: var(--accent); }
  .st-tg.on i::after { left: 11px; background: var(--accent); }
  .st-tg.on .micro { color: var(--text-2); }
  .st-btn { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border: 1px solid var(--border); border-radius: var(--r); background: var(--bg-elev); color: var(--text-2); font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.06em; text-transform: uppercase; white-space: nowrap; }
  .st-btn.primary { background: var(--accent); border-color: var(--accent); color: var(--on-accent); }
  .st-btn.ghost { background: transparent; border-color: var(--border-soft); color: var(--muted); }
  .st-btn.danger { background: var(--danger-dim); border-color: var(--danger-brd); color: var(--danger); }
  .st-btn.dashed { border-style: dashed; }
  .st-btn:disabled { opacity: 0.45; }
  .st-banner { margin: 0.2rem 0; padding: 6px 10px; border-radius: var(--r); font-size: 0.78rem; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .st-banner.warn { background: var(--warn-dim); border: 1px solid var(--warn-brd); color: var(--text-2); }
  .st-banner.danger { background: var(--danger-dim); border: 1px solid var(--danger-brd); color: var(--text-2); }
  .st-banner.info { background: var(--bg-elev); border: 1px solid var(--border); color: var(--text-2); }
  .st-banner.st-save { flex-direction: column; align-items: flex-start; gap: 3px; }
  .st-banner.st-save .micro.wrap { margin: 0; }

  /* Editor row */
  .st-row { flex: 1; min-height: 0; display: flex; gap: 10px; padding: 6px 0; }
  .st-tools { display: flex; flex-direction: column; gap: 4px; flex: none; }
  .st-tool { width: 32px; height: 32px; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); display: flex; align-items: center; justify-content: center; position: relative; padding: 0; }
  .st-tool.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tool:disabled { opacity: 0.4; }
  .st-tool .k { position: absolute; right: 2px; bottom: 0; font-family: var(--mono); font-size: 0.5rem; color: var(--faint); }
  .st-tool.on .k { color: var(--accent-hi); }
  .st-tool.sm { width: 26px; height: 26px; } .st-tool.sm svg { width: 11px; height: 11px; }
  /* Play is the one transport control that changes state; it reads as the primary button. */
  .st-tool.play { width: 34px; height: 30px; border-radius: 999px; background: var(--accent); border-color: var(--accent); color: var(--on-accent); margin: 0 4px; }
  .st-tool.play svg { width: 14px; height: 14px; }
  .st-tool.play.on { background: var(--accent-dim); color: var(--accent-hi); }
  .st-sep { height: 1px; background: var(--border-soft); margin: 4px 2px; }
  .st-canvas-card { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); padding: 10px; max-width: 100%; }
  .st-canvas-scroll { overflow: auto; max-height: 100%; scrollbar-gutter: stable; }
  .st-canvas-wrap { position: relative; max-width: none; border: 1px solid var(--border); line-height: 0; cursor: crosshair; }
  .st-zoom { display: inline-flex; align-items: center; gap: 6px; }
  .st-zoom .st-tile.txt { width: 26px; padding: 0; font-size: 0.8rem; }
  .st-canvas-wrap.locked { cursor: default; }
  .st-canvas, .st-overlay { position: absolute; inset: 0; width: 100%; height: 100%; image-rendering: pixelated; }
  .st-canvas { position: relative; display: block; }
  .st-overlay { touch-action: none; }
  .st-veil { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 6px; background: color-mix(in oklab, var(--bg-0) 72%, transparent); color: var(--text); font-size: 0.84rem; line-height: 1.4; text-align: center; }
  .st-veil.soft { justify-content: flex-end; padding-bottom: 28px; background: transparent; }
  .st-veil-acts, .st-card-acts { display: flex; gap: 6px; margin-top: 4px; flex-wrap: wrap; }
  .st-readout { position: absolute; bottom: 6px; font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); background: color-mix(in oklab, var(--bg-0) 70%, transparent); padding: 2px 5px; border-radius: 3px; line-height: 1.3; }
  .st-readout.left { left: 8px; } .st-readout.right { right: 8px; color: var(--accent-hi); }
  .st-canvas-foot { display: flex; align-items: center; gap: 10px; }

  /* Inspector */
  .st-inspector { flex: 0 0 260px; min-width: 240px; display: flex; flex-direction: column; background: var(--panel); border: 1px solid var(--border); border-radius: var(--r-lg); overflow: hidden; }
  .st-itabs { display: flex; border-bottom: 1px solid var(--border); }
  .st-itab { flex: 1; padding: 7px 0; border: none; border-bottom: 2px solid transparent; border-radius: 0; background: transparent; font-family: var(--mono); font-size: 0.66rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); }
  .st-itab.on { color: var(--text); border-bottom-color: var(--accent); }
  .st-ibody { display: flex; flex-direction: column; gap: 4px; padding: 4px 12px 10px; overflow-y: auto; }
  .st-sec { padding: 6px 0 4px; font-family: var(--mono); font-size: 0.62rem; font-weight: 600; letter-spacing: 0.14em; text-transform: uppercase; color: var(--text-2); }
  .st-sec.nxt { border-top: 1px solid var(--border-soft); margin-top: 6px; }
  .st-opt { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; row-gap: 4px; }
  .st-opt .lb { font-family: var(--mono); font-size: 0.6rem; letter-spacing: 0.08em; text-transform: uppercase; color: var(--faint); width: 44px; flex: none; }
  .st-tile { display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 26px; padding: 0; border: 1px solid var(--border-soft); border-radius: var(--r); background: var(--bg-elev); color: var(--muted); font-family: var(--mono); font-size: 0.6rem; text-transform: uppercase; }
  .st-tile.txt { width: auto; padding: 0 8px; }
  .st-tile.on { background: var(--accent-dim); border-color: var(--accent); color: var(--accent-hi); }
  .st-tile:disabled { opacity: 0.45; }
  .st-tile svg { width: 14px; height: 14px; }
  .st-input { flex: 1; min-width: 0; background: var(--bg-elev); border: 1px solid var(--border); border-radius: var(--r); color: var(--text); font-family: var(--mono); font-size: 0.72rem; padding: 3px 6px; }
  .st-input.date { flex: 0 1 9rem; height: 26px; box-sizing: border-box; color-scheme: dark; }
  .st-input.date.on { border-color: var(--accent); background: var(--accent-dim); }
  .st-size { display: inline-flex; gap: 2px; align-items: flex-end; height: 16px; }
  .st-size button { width: 6px; padding: 0; border: none; border-radius: 1px; background: var(--faint); }
  .st-size button.on { background: var(--accent); }
  .st-wells { display: grid; grid-template-columns: repeat(8, minmax(0, 1fr)); gap: 4px; }
  .st-well { height: 22px; border-radius: 3px; border: 1px solid color-mix(in oklab, var(--bg-0) 60%, transparent); padding: 0; position: relative; }
  .st-well .micro { position: absolute; left: 0; right: 0; top: 100%; font-size: 0.5rem; letter-spacing: 0.04em; text-align: center; }
  .st-wells { margin-bottom: 8px; }
  .st-wells.grays { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .st-theme-colors { font-size: .72rem; color: var(--muted); }
  .st-theme-colors summary { cursor: pointer; margin: 3px 0 7px; }
  .st-theme-colors label { display: flex; align-items: center; gap: 6px; }
  .st-theme-colors input { width: auto; }
  .st-well.sel { outline: 2px solid var(--accent); outline-offset: 2px; }
  .st-layer { display: flex; align-items: center; gap: 8px; padding: 3px 6px; border-radius: var(--r); border: 1px solid transparent; }
  .st-layer.on { background: var(--accent-dim); border-color: var(--accent); }
  .st-layer .eye { width: 20px; height: 20px; padding: 0; border: none; background: transparent; color: var(--muted); display: inline-flex; align-items: center; justify-content: center; }
  .st-layer .eye svg { width: 14px; height: 14px; } .st-layer .eye.off { color: var(--faint); }
  .st-layer .pick { flex: 1; display: flex; align-items: center; gap: 8px; background: transparent; border: none; padding: 2px 0; color: inherit; text-align: left; }
  .st-layer .pick .stack { width: 16px; height: 16px; color: var(--faint); }
  .st-layer .pick .stack .lit { stroke: var(--accent); stroke-width: 2; }
  .st-layer .nm { font-family: var(--mono); font-size: 0.66rem; color: var(--text-2); }
  .st-layer.on .nm { color: var(--text); }
  .st-layer .what { margin-left: auto; font-size: 0.66rem; color: var(--muted); }
  .st-card { display: flex; flex-direction: column; gap: 3px; background: var(--bg-elev); border: 1px solid var(--border-soft); border-radius: var(--r); padding: 8px 10px; font-size: 0.8rem; color: var(--text); }
  .st-card.row { flex-direction: row; align-items: center; gap: 8px; }
  .st-card.accent { background: var(--accent-dim); border-color: var(--accent); }
  .st-card.warn { background: var(--warn-dim); border-color: var(--warn-brd); }
  .st-card .sub { font-size: 0.72rem; color: var(--text-2); }
  .st-alt { display: flex; align-items: center; gap: 6px; font-size: 0.74rem; color: var(--text-2); }
  .st-alt i { width: 7px; height: 7px; border-radius: 2px; flex: none; }
  .st-link { background: transparent; border: none; padding: 0; font-family: var(--mono); font-size: 0.62rem; letter-spacing: 0.1em; text-transform: uppercase; color: var(--muted); text-decoration: underline; }
  .st-link.danger { color: var(--danger); }
  .st-link:disabled { opacity: 0.45; text-decoration: none; }
  .st-bar { height: 4px; border-radius: 999px; background: var(--bg-elev); overflow: hidden; }
  .st-bar i { display: block; height: 100%; background: var(--accent); }
  .st-hist { display: flex; align-items: center; gap: 6px; margin: 2px 0; font-family: var(--mono); font-size: 0.68rem; color: var(--text-2); }
  .st-hist i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st-hist i.ok { background: var(--ok); } .st-hist i.warn { background: var(--warn); } .st-hist i.danger { background: var(--danger); } .st-hist i.info { background: var(--info); }
  .st-ver { display: flex; flex-direction: column; gap: 3px; padding: 6px 0; border-top: 1px dashed var(--border-soft); }
  .st-ver.staged .st-hist { color: var(--warn); }
  .st-run { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; max-height: 160px; overflow-y: auto; }
  .st-run li { display: flex; align-items: center; gap: 6px; font-size: 0.72rem; color: var(--text-2); }
  .st-run li i { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st-run li i.ok { background: var(--ok); } .st-run li i.warn { background: var(--warn); } .st-run li i.danger { background: var(--danger); } .st-run li i.info { background: var(--info); }

  /* Timeline */
  .st-timeline { flex: none; background: var(--panel); border-top: 1px solid var(--border); margin: 0 -0.5rem; padding: 8px 10px 10px; display: flex; flex-direction: column; gap: 8px; }
  .st-transport { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .st-strip { display: flex; gap: 6px; align-items: flex-start; overflow-x: auto; padding-bottom: 4px; scrollbar-gutter: stable; }
  .st-thumb { display: flex; flex-direction: column; gap: 3px; align-items: center; background: transparent; border: none; padding: 0; color: inherit; flex: none; }
  .st-thumb .ix { font-family: var(--mono); font-size: 0.58rem; color: var(--faint); white-space: nowrap; }
  .st-thumb.cur .ix { color: var(--accent-hi); }
  .st-thumb .fr { position: relative; display: block; width: 64px; height: 48px; border: 1px solid var(--border); border-radius: 3px; overflow: hidden; line-height: 0; background: var(--bg-0); }
  .st-thumb .fr canvas { display: block; width: 64px; height: 48px; image-rendering: pixelated; }
  .st-thumb.cur .fr { outline: 2px solid var(--accent); outline-offset: 1px; }
  .st-thumb .fr.fetching, .st-thumb .fr.pending { border-style: dashed; border-color: var(--info); display: flex; align-items: center; justify-content: center; }
  .st-thumb .fr.missing { border-style: dashed; border-color: var(--warn); display: flex; align-items: center; justify-content: center; }
  .st-thumb .fr.fetching .micro, .st-thumb .fr.missing .micro { color: var(--info); font-size: 0.5rem; }
  .st-thumb .fr.missing .micro { color: var(--warn); }
  .st-thumb .fr .tag { position: absolute; left: 0; right: 0; bottom: 0; text-align: center; background: color-mix(in oklab, var(--bg-0) 70%, transparent); line-height: 1.6; }
  .st-thumb .fr.pending canvas { opacity: 0.7; }
  .st-thumb .fr.pending.uncertain { border-color: var(--warn); }
  .st-thumb.over .fr { opacity: 0.45; background: repeating-linear-gradient(135deg, var(--bg-elev) 0 6px, var(--panel) 6px 12px); }
  .st-thumb.conflict .fr { border-color: var(--warn); }
  .st-thumb .dot { position: absolute; left: 3px; bottom: 3px; width: 7px; height: 7px; border-radius: 2px; }
  .st-thumb .claim { position: absolute; right: 3px; bottom: 3px; width: 7px; height: 7px; border-radius: 50%; box-shadow: 0 0 0 1px var(--bg-0); }
  .st-thumb .claim.contested { box-shadow: 0 0 0 2px var(--warn); }
  .st-ask { display: inline-flex; align-items: center; gap: 6px; font-size: 0.7rem; padding: 2px 8px; border: 1px solid var(--accent); border-radius: 4px; background: color-mix(in oklab, var(--accent) 12%, transparent); }
  .st-thumb .corner { position: absolute; right: 0; top: 0; width: 0; height: 0; border-top: 9px solid var(--warn); border-left: 9px solid transparent; }
  .st-sfxlane { display: flex; align-items: center; gap: 6px; }
  .st-sfxlane .lb { width: 24px; font-size: 0.5rem; }
  .st-sfxlane .cell { width: 64px; height: 12px; border-bottom: 1px solid var(--border-soft); position: relative; flex: none; display: flex; align-items: center; }
  /* One pill per frame under its thumbnail: the first marker's name and note, tinted by who set
     it. The pill is as wide as the frame; a longer label clips rather than pushing the lane. */
  .st-sfxlane .pill { display: block; width: 100%; height: 11px; line-height: 11px; border-radius: 999px; padding: 0 5px; box-sizing: border-box; font-family: var(--mono); font-size: 0.5rem; letter-spacing: 0.04em; text-transform: uppercase; color: var(--on-accent); white-space: nowrap; overflow: hidden; text-overflow: clip; font-style: normal; }
</style>
